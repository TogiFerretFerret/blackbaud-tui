//! UI state and input handling. Pure logic: no terminal, no network. Commands go out through a channel
//! and results come back as [`Event`]s, so everything here is testable.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;

use bromine::{
    Announcement, AssignmentDetail, AssignmentRow, Bucket, CenterItem, Class, ClassFile, ClassInfo, FeedItem, FileRef, GradeRow, Grades,
    ItemKind, Person, Status, TaskPatch, Topic, TopicItem, UserFile,
};
use chrono::{Duration, NaiveDateTime};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::fmt;
use crate::worker::{Cmd, Event, FileAction};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Assignments,
    Classes,
    Feed,
    Files,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Assignments, Tab::Classes, Tab::Feed, Tab::Files];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Assignments => "Assignments",
            Tab::Classes => "Classes",
            Tab::Feed => "Feed",
            Tab::Files => "Files",
        }
    }

    fn next(self, by: isize) -> Tab {
        let i = Tab::ALL.iter().position(|t| *t == self).unwrap_or(0) as isize;
        Tab::ALL[(i + by).rem_euclid(Tab::ALL.len() as isize) as usize]
    }
}

/// The views inside a class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClassTab {
    About,
    Work,
    Topics,
    News,
    Files,
    Grades,
    People,
}

impl ClassTab {
    pub const ALL: [ClassTab; 7] =
        [ClassTab::About, ClassTab::Work, ClassTab::Topics, ClassTab::News, ClassTab::Files, ClassTab::Grades, ClassTab::People];

    pub fn title(self) -> &'static str {
        match self {
            ClassTab::About => "About",
            ClassTab::Work => "Work",
            ClassTab::Topics => "Topics",
            ClassTab::News => "News",
            ClassTab::Files => "Files",
            ClassTab::Grades => "Grades",
            ClassTab::People => "People",
        }
    }

    fn next(self, by: isize) -> ClassTab {
        let i = ClassTab::ALL.iter().position(|t| *t == self).unwrap_or(0) as isize;
        ClassTab::ALL[(i + by).rem_euclid(ClassTab::ALL.len() as isize) as usize]
    }
}

/// Which column of the Classes view the arrow keys move.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Classes,
    Sub,
}

/// Something fetched on demand.
#[derive(Debug, Clone, PartialEq)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> From<Result<T, String>> for Load<T> {
    fn from(r: Result<T, String>) -> Self {
        match r {
            Ok(v) => Load::Ready(v),
            Err(e) => Load::Failed(e),
        }
    }
}

type Cache<T> = HashMap<i64, Load<Vec<T>>>;

fn ready<T>(cache: &Cache<T>, key: i64) -> Option<&Vec<T>> {
    match cache.get(&key) {
        Some(Load::Ready(v)) => Some(v),
        _ => None,
    }
}

/// One line of the Grades view: a marking period (with its total) or one graded assignment.
pub enum GradeEntry<'a> {
    Period(&'a Grades),
    Row(&'a GradeRow),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Header(Bucket),
    /// Index into [`App::items`].
    Item(usize),
}

/// A single-line text field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
    pub text: String,
    /// In characters.
    pub cursor: usize,
}

impl Input {
    pub fn with(text: &str) -> Self {
        Self { text: text.to_owned(), cursor: text.chars().count() }
    }

    fn byte(&self, chars: usize) -> usize {
        self.text.char_indices().nth(chars).map_or(self.text.len(), |(i, _)| i)
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte(self.cursor);
            self.text.remove(at);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.chars().count() {
            let at = self.byte(self.cursor);
            self.text.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }

    /// Handle an editing key; `true` if it was one.
    pub fn handle(&mut self, key: &KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => self.insert(c),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.chars().count(),
            _ => return false,
        }
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTask {
    pub desc: Input,
    pub due: Input,
    /// 0 = description, 1 = due.
    pub field: usize,
    pub error: Option<String>,
    /// `Some(id)` when editing that task instead of making a new one.
    pub editing: Option<i64>,
}

/// Pick the assignment a task belongs to (or none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPicker {
    pub task_id: i64,
    pub filter: Input,
    pub sel: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Help,
    NewTask(NewTask),
    ConfirmDelete { id: i64, name: String },
    ConfirmPrint { name: String, path: String },
    Link(LinkPicker),
    Login { url: Option<String>, opened: bool, failed: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalAction {
    Submit,
    Cancel,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub ticks_left: u16,
}

/// Where things were drawn last frame, so mouse events can be mapped back to them.
#[derive(Default)]
pub struct Hits {
    pub tabs: Vec<(Rect, Tab)>,
    /// The scrollable list of the current tab (inner area) and its visible rows.
    pub list: Rect,
    pub rows: Vec<(Rect, usize)>,
    pub detail: Rect,
    pub buttons: Vec<(Rect, Status)>,
    pub modal_buttons: Vec<(Rect, ModalAction)>,
    /// Classes tab: the right-hand assignment list.
    pub sub_list: Rect,
    pub sub_rows: Vec<(Rect, usize)>,
    pub sub_tabs: Vec<(Rect, ClassTab)>,
    /// Attachment lines in the detail pane.
    pub attachments: Vec<(Rect, usize)>,
    /// Rows of a picker modal.
    pub modal_rows: Vec<(Rect, usize)>,
}

pub struct App {
    pub tab: Tab,
    pub now: NaiveDateTime,
    pub should_quit: bool,
    pub hits: Hits,

    // assignments
    pub items: Vec<CenterItem>,
    pub rows: Vec<Row>,
    pub sel: usize,
    pub offset: usize,
    pub show_past: bool,
    pub hide_done: bool,
    pub filter: String,
    pub filtering: bool,
    pub details: HashMap<i64, AssignmentDetail>,
    pub detail_failed: HashMap<i64, String>,
    detail_pending: HashSet<i64>,
    pub detail_scroll: usize,
    previous_status: HashMap<(i64, bool), Vec<Status>>,

    // classes
    pub classes: Vec<Class>,
    pub class_sel: usize,
    pub class_offset: usize,
    pub class_info: HashMap<i64, ClassInfo>,
    pub class_rows: HashMap<i64, Vec<AssignmentRow>>,
    pub class_scroll: usize,
    pub class_tab: ClassTab,
    pub focus: Focus,
    pub sub_sel: usize,
    pub sub_offset: usize,
    pub topics: Cache<Topic>,
    pub topic_items: Cache<TopicItem>,
    pub news: Cache<Announcement>,
    pub class_files: Cache<ClassFile>,
    pub grades: Cache<Grades>,
    pub people: Cache<Person>,
    class_info_pending: HashSet<i64>,
    class_rows_pending: HashSet<i64>,

    // feed
    pub feed: Vec<FeedItem>,
    pub feed_sel: usize,
    pub feed_offset: usize,
    pub feed_scroll: usize,

    // files (the school's files for you)
    pub user_files: Option<Load<Vec<UserFile>>>,
    pub file_sel: usize,
    pub file_offset: usize,
    pub file_scroll: usize,
    /// Which of the shown attachments `o`, `d` and `P` act on.
    pub att_sel: usize,

    pub center_loaded: bool,
    pub pending: usize,
    pub toast: Option<Toast>,
    pub modal: Option<Modal>,
    pub spinner: usize,
    tx: Sender<Cmd>,
}

impl App {
    pub fn new(tx: Sender<Cmd>, now: NaiveDateTime) -> Self {
        Self {
            tab: Tab::Assignments,
            now,
            should_quit: false,
            hits: Hits::default(),
            items: vec![],
            rows: vec![],
            sel: 0,
            offset: 0,
            show_past: false,
            hide_done: false,
            filter: String::new(),
            filtering: false,
            details: HashMap::new(),
            detail_failed: HashMap::new(),
            detail_pending: HashSet::new(),
            detail_scroll: 0,
            previous_status: HashMap::new(),
            classes: vec![],
            class_sel: 0,
            class_offset: 0,
            class_info: HashMap::new(),
            class_rows: HashMap::new(),
            class_scroll: 0,
            class_tab: ClassTab::Work,
            focus: Focus::Classes,
            sub_sel: 0,
            sub_offset: 0,
            topics: HashMap::new(),
            topic_items: HashMap::new(),
            news: HashMap::new(),
            class_files: HashMap::new(),
            grades: HashMap::new(),
            people: HashMap::new(),
            class_info_pending: HashSet::new(),
            class_rows_pending: HashSet::new(),
            feed: vec![],
            feed_sel: 0,
            feed_offset: 0,
            feed_scroll: 0,
            user_files: None,
            file_sel: 0,
            file_offset: 0,
            file_scroll: 0,
            att_sel: 0,
            center_loaded: false,
            pending: 0,
            toast: None,
            modal: None,
            spinner: 0,
            tx,
        }
    }

    /// Ask for everything the first screen needs.
    pub fn start(&mut self) {
        self.send(Cmd::Center);
        self.send(Cmd::Classes);
        self.send(Cmd::Feed);
    }

    fn send(&mut self, cmd: Cmd) {
        self.pending += 1;
        if self.tx.send(cmd).is_err() {
            self.pending -= 1; // the worker is gone (shutting down)
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast { text: text.into(), error, ticks_left: if error { 60 } else { 30 } });
    }

    /// Called about ten times a second.
    pub fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
        if let Some(t) = &mut self.toast {
            t.ticks_left = t.ticks_left.saturating_sub(1);
            if t.ticks_left == 0 {
                self.toast = None;
            }
        }
    }

    // ------------------------------------------------------------------ selection helpers

    pub fn selected_item(&self) -> Option<&CenterItem> {
        match self.rows.get(self.sel)? {
            Row::Item(i) => self.items.get(*i),
            Row::Header(_) => None,
        }
    }

    fn selected_key(&self) -> Option<(ItemKind, i64)> {
        self.selected_item().map(|i| (i.kind, i.id))
    }

    pub fn selected_class(&self) -> Option<&Class> {
        self.classes.get(self.class_sel)
    }

    pub fn list_height(&self) -> usize {
        (self.hits.list.height as usize).max(1)
    }

    /// Rebuild the visible rows from the items and the current view options, keeping the selection.
    pub fn rebuild_rows(&mut self) {
        let keep = self.selected_key();
        let q = self.filter.to_lowercase();
        let mut rows = Vec::new();
        for b in Bucket::ALL {
            let idx: Vec<usize> = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, it)| {
                    it.bucket == b
                        && (self.show_past || b.is_upcoming())
                        && !(self.hide_done && it.status.is_done())
                        && (q.is_empty() || it.name.to_lowercase().contains(&q) || it.class.to_lowercase().contains(&q))
                })
                .map(|(i, _)| i)
                .collect();
            if !idx.is_empty() {
                rows.push(Row::Header(b));
                rows.extend(idx.into_iter().map(Row::Item));
            }
        }
        self.rows = rows;
        let found = keep.and_then(|(kind, id)| {
            self.rows.iter().position(|r| matches!(r, Row::Item(i) if self.items[*i].kind == kind && self.items[*i].id == id))
        });
        self.sel = found.or_else(|| self.rows.iter().position(|r| matches!(r, Row::Item(_)))).unwrap_or(0);
        self.keep_visible();
        self.want_detail();
    }

    /// Scroll just enough to show the selection. Does nothing before the first draw (height unknown).
    pub fn keep_visible(&mut self) {
        let h = self.hits.list.height as usize;
        if h == 0 {
            return;
        }
        if self.sel < self.offset {
            self.offset = self.sel;
            // show the section header above the first item of a section
            if self.offset > 0 && matches!(self.rows[self.offset - 1], Row::Header(_)) {
                self.offset -= 1;
            }
        } else if self.sel >= self.offset + h {
            self.offset = self.sel + 1 - h;
        }
        self.offset = self.offset.min(self.rows.len().saturating_sub(1));
    }

    fn move_sel(&mut self, delta: isize) {
        let step = delta.signum();
        let mut left = delta.abs();
        let mut i = self.sel as isize;
        while left > 0 {
            let mut j = i + step;
            while j >= 0 && (j as usize) < self.rows.len() && matches!(self.rows[j as usize], Row::Header(_)) {
                j += step;
            }
            if j < 0 || j as usize >= self.rows.len() {
                break;
            }
            i = j;
            left -= 1;
        }
        if i as usize != self.sel {
            self.sel = i as usize;
            self.detail_scroll = 0;
            self.keep_visible();
            self.want_detail();
        }
    }

    fn select_row(&mut self, row: usize) {
        if matches!(self.rows.get(row), Some(Row::Item(_))) && row != self.sel {
            self.sel = row;
            self.detail_scroll = 0;
            self.keep_visible();
            self.want_detail();
        }
    }

    /// Fetch the selected assignment's details if we do not have them.
    fn want_detail(&mut self) {
        let Some(item) = self.selected_item() else { return };
        if !item.is_task() {
            self.want_detail_of(item.id);
        }
    }

    fn want_detail_of(&mut self, id: i64) {
        if !self.details.contains_key(&id) && self.detail_pending.insert(id) {
            self.send(Cmd::Detail(id));
        }
    }

    /// Fetch whatever the Classes view is showing and has not got yet.
    fn want_class_data(&mut self) {
        let Some(c) = self.selected_class() else { return };
        let (section, lead) = (c.section_id, c.lead_section_id);
        if !self.class_info.contains_key(&section) && self.class_info_pending.insert(section) {
            self.send(Cmd::ClassInfo { section_id: section });
        }
        macro_rules! lazy {
            ($map:ident, $cmd:expr) => {
                if !self.$map.contains_key(&section) {
                    self.$map.insert(section, Load::Loading);
                    self.send($cmd);
                }
            };
        }
        match self.class_tab {
            ClassTab::About => {}
            ClassTab::Work => {
                if !self.class_rows.contains_key(&lead) && self.class_rows_pending.insert(lead) {
                    self.send(Cmd::ClassAssignments { lead_section_id: lead });
                }
                if let Some(id) = self.selected_work().map(|r| r.id) {
                    self.want_detail_of(id);
                }
            }
            ClassTab::Topics => {
                lazy!(topics, Cmd::Topics { section_id: section });
                if let Some(t) = self.selected_topic().cloned()
                    && !self.topic_items.contains_key(&t.topic_id)
                {
                    self.topic_items.insert(t.topic_id, Load::Loading);
                    self.send(Cmd::TopicItems(t));
                }
            }
            ClassTab::News => lazy!(news, Cmd::News { section_id: section }),
            ClassTab::Files => lazy!(class_files, Cmd::ClassFiles { section_id: section }),
            ClassTab::Grades => lazy!(grades, Cmd::Grades { section_id: section }),
            ClassTab::People => lazy!(people, Cmd::People { section_id: section }),
        }
    }

    // ------------------------------------------------------------------ the Classes view's lists

    pub fn selected_work(&self) -> Option<&AssignmentRow> {
        self.class_rows.get(&self.selected_class()?.lead_section_id)?.get(self.sub_sel)
    }

    pub fn selected_topic(&self) -> Option<&Topic> {
        if self.class_tab != ClassTab::Topics {
            return None;
        }
        ready(&self.topics, self.selected_class()?.section_id)?.get(self.sub_sel)
    }

    pub fn selected_class_file(&self) -> Option<&ClassFile> {
        ready(&self.class_files, self.selected_class()?.section_id)?.get(self.sub_sel)
    }

    /// The `i`th line of the Grades view for the selected class.
    pub fn grade_entry(&self, i: usize) -> Option<GradeEntry<'_>> {
        let mut i = i;
        for period in ready(&self.grades, self.selected_class()?.section_id)? {
            if i == 0 {
                return Some(GradeEntry::Period(period));
            }
            i -= 1;
            if let Some(row) = period.rows.get(i) {
                return Some(GradeEntry::Row(row));
            }
            i -= period.rows.len();
        }
        None
    }

    /// How many lines the sub-list of the current class view has.
    pub fn sub_len(&self) -> usize {
        let Some(c) = self.selected_class() else { return 0 };
        let section = c.section_id;
        match self.class_tab {
            ClassTab::About => 0,
            ClassTab::Work => self.class_rows.get(&c.lead_section_id).map_or(0, Vec::len),
            ClassTab::Topics => ready(&self.topics, section).map_or(0, Vec::len),
            ClassTab::News => ready(&self.news, section).map_or(0, Vec::len),
            ClassTab::Files => ready(&self.class_files, section).map_or(0, Vec::len),
            ClassTab::People => ready(&self.people, section).map_or(0, Vec::len),
            ClassTab::Grades => ready(&self.grades, section).map_or(0, |g| g.iter().map(|p| 1 + p.rows.len()).sum()),
        }
    }

    fn clamp_sub(&mut self) {
        self.sub_sel = self.sub_sel.min(self.sub_len().saturating_sub(1));
        self.sub_offset = self.sub_offset.min(self.sub_len().saturating_sub(1));
    }

    fn reset_sub(&mut self) {
        self.sub_sel = 0;
        self.sub_offset = 0;
        self.class_scroll = 0;
        self.att_sel = 0;
    }

    fn sub_height(&self) -> usize {
        (self.hits.sub_list.height as usize).max(1)
    }

    fn set_class_tab(&mut self, tab: ClassTab) {
        self.class_tab = tab;
        self.reset_sub();
        self.want_class_data();
    }

    /// The files the current selection offers to open, save or print.
    pub fn attachments(&self) -> Vec<FileRef> {
        let from_detail = |id: i64| self.details.get(&id).map(|d| d.downloads.clone()).unwrap_or_default();
        match self.tab {
            Tab::Assignments => self.selected_item().filter(|i| !i.is_task()).map(|i| from_detail(i.id)).unwrap_or_default(),
            Tab::Classes => match self.class_tab {
                ClassTab::Work => self.selected_work().map(|r| from_detail(r.id)).unwrap_or_default(),
                ClassTab::Topics => {
                    let items = self.selected_topic().and_then(|t| match self.topic_items.get(&t.topic_id) {
                        Some(Load::Ready(v)) => Some(v),
                        _ => None,
                    });
                    items
                        .into_iter()
                        .flatten()
                        .filter_map(|i| {
                            Some(FileRef {
                                name: i.file.clone().or_else(|| i.title.clone()).unwrap_or_else(|| "file".into()),
                                path: i.path.clone()?,
                            })
                        })
                        .collect()
                }
                ClassTab::Files => {
                    self.selected_class_file().map(|f| vec![FileRef { name: f.name.clone(), path: f.path.clone() }]).unwrap_or_default()
                }
                _ => vec![],
            },
            Tab::Files => {
                self.selected_user_file().map(|f| vec![FileRef { name: f.name.clone(), path: f.path.clone() }]).unwrap_or_default()
            }
            Tab::Feed => vec![],
        }
    }

    pub fn selected_user_file(&self) -> Option<&UserFile> {
        match &self.user_files {
            Some(Load::Ready(v)) => v.get(self.file_sel),
            _ => None,
        }
    }

    fn fetch_attachment(&mut self, action: FileAction) {
        let atts = self.attachments();
        let Some(a) = atts.get(self.att_sel.min(atts.len().saturating_sub(1))) else {
            return self.toast("There is no file here to open", true);
        };
        let (name, path) = (a.name.clone(), a.path.clone());
        if action == FileAction::Print {
            self.modal = Some(Modal::ConfirmPrint { name, path });
        } else {
            self.send_fetch(path, name, action);
        }
    }

    fn send_fetch(&mut self, path: String, name: String, action: FileAction) {
        self.toast(format!("Downloading {name}…"), false);
        self.send(Cmd::Fetch { path, name, action });
    }

    // ------------------------------------------------------------------ events from the worker

    pub fn on_event(&mut self, ev: Event) {
        let counted = !matches!(ev, Event::LoginNeeded | Event::LoginUrl { .. } | Event::LoginDone | Event::LoginFailed(_));
        if counted {
            self.pending = self.pending.saturating_sub(1);
        }
        match ev {
            Event::Center(Ok(items)) => {
                self.items = items;
                self.center_loaded = true;
                self.rebuild_rows();
            }
            Event::Center(Err(e)) => self.toast(format!("Could not load assignments: {e}"), true),
            Event::Classes(Ok(classes)) => {
                self.classes = classes;
                self.class_sel = self.class_sel.min(self.classes.len().saturating_sub(1));
                if self.tab == Tab::Classes {
                    self.want_class_data();
                }
            }
            Event::Classes(Err(e)) => self.toast(format!("Could not load classes: {e}"), true),
            Event::Feed(Ok(feed)) => {
                self.feed = feed;
                self.feed_sel = self.feed_sel.min(self.feed.len().saturating_sub(1));
            }
            Event::Feed(Err(e)) => self.toast(format!("Could not load the feed: {e}"), true),
            Event::Detail { id, result } => {
                self.detail_pending.remove(&id);
                match result {
                    Ok(d) => {
                        // the assignment's own status is the truth: show it in the list too
                        if let Some(item) = self.items.iter_mut().find(|i| !i.is_task() && i.id == id) {
                            item.status = d.status;
                        }
                        self.details.insert(id, d);
                        self.detail_failed.remove(&id);
                    }
                    Err(e) => {
                        self.detail_failed.insert(id, e);
                    }
                }
            }
            Event::ClassInfo { section_id, result } => {
                self.class_info_pending.remove(&section_id);
                if let Ok(info) = result {
                    self.class_info.insert(section_id, info);
                }
            }
            Event::ClassAssignments { lead_section_id, result } => {
                self.class_rows_pending.remove(&lead_section_id);
                match result {
                    Ok(rows) => {
                        self.class_rows.insert(lead_section_id, rows);
                        self.want_class_data(); // the first row's details
                    }
                    Err(e) => self.toast(format!("Could not load that class's assignments: {e}"), true),
                }
            }
            Event::Topics { section_id, result } => {
                self.topics.insert(section_id, result.into());
                self.want_class_data(); // the first topic's content
            }
            Event::TopicItems { topic_id, result } => {
                self.topic_items.insert(topic_id, result.into());
            }
            Event::News { section_id, result } => {
                self.news.insert(section_id, result.into());
            }
            Event::ClassFiles { section_id, result } => {
                self.class_files.insert(section_id, result.into());
            }
            Event::Grades { section_id, result } => {
                self.grades.insert(section_id, result.into());
            }
            Event::People { section_id, result } => {
                self.people.insert(section_id, result.into());
            }
            Event::UserFiles(result) => {
                self.user_files = Some(result.into());
            }
            Event::TaskEdited(result) => match result {
                Ok(()) => {
                    self.toast("Task updated", false);
                    self.send(Cmd::Center);
                }
                Err(e) => self.toast(e, true),
            },
            Event::Fetched { name, action, result } => match result {
                Ok(path) => self.toast(
                    match action {
                        FileAction::Save => format!("Saved {}", path.display()),
                        FileAction::Open => format!("Opened {name}"),
                        FileAction::Print => format!("Sent {name} to the printer (double-sided)"),
                    },
                    false,
                ),
                Err(e) => self.toast(e, true),
            },
            Event::StatusSet { id, status, task, result } => {
                let prev = self.previous_status.get_mut(&(id, task)).and_then(|v| (!v.is_empty()).then(|| v.remove(0)));
                match result {
                    Ok(()) => self.toast(format!("Marked as {}", status.label()), false),
                    Err(e) => {
                        if let Some(prev) = prev {
                            self.apply_status(id, task, prev);
                        }
                        self.toast(e, true);
                    }
                }
            }
            Event::TaskCreated(Ok(_)) => {
                self.toast("Task added", false);
                self.send(Cmd::Center);
            }
            Event::TaskCreated(Err(e)) => self.toast(e, true),
            Event::TaskDeleted { id, result } => match result {
                Ok(()) => {
                    self.items.retain(|i| !(i.is_task() && i.id == id));
                    self.rebuild_rows();
                    self.toast("Task deleted", false);
                }
                Err(e) => self.toast(e, true),
            },
            Event::LoginNeeded => self.modal = Some(Modal::Login { url: None, opened: false, failed: None }),
            Event::LoginUrl { url, opened } => self.modal = Some(Modal::Login { url: Some(url), opened, failed: None }),
            Event::LoginDone => {
                self.modal = None;
                self.toast("Signed in", false);
            }
            Event::LoginFailed(e) => self.modal = Some(Modal::Login { url: None, opened: false, failed: Some(e) }),
        }
        self.clamp_sub();
    }

    fn apply_status(&mut self, id: i64, task: bool, status: Status) {
        for item in self.items.iter_mut().filter(|i| i.id == id && i.is_task() == task) {
            item.status = status;
        }
        if !task && let Some(d) = self.details.get_mut(&id) {
            d.status = status;
        }
    }

    /// Optimistically show a new status for the selected item and ask the worker to make it so.
    pub fn set_status(&mut self, status: Status) {
        let Some(item) = self.selected_item() else { return };
        if !status.is_settable() || item.status == status {
            return;
        }
        let (id, task, prev) = (item.id, item.is_task(), item.status);
        self.previous_status.entry((id, task)).or_default().push(prev);
        self.apply_status(id, task, status);
        self.send(Cmd::SetStatus { id, status, task });
    }

    // ------------------------------------------------------------------ keyboard

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.modal.is_some() {
            return self.on_modal_key(key);
        }
        if self.filtering {
            return self.on_filter_key(key);
        }
        let page = self.list_height().saturating_sub(1).max(1) as isize;
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.modal = Some(Modal::Help),
            KeyCode::Char('1') => self.set_tab(Tab::Assignments),
            KeyCode::Char('2') => self.set_tab(Tab::Classes),
            KeyCode::Char('3') => self.set_tab(Tab::Feed),
            KeyCode::Char('4') => self.set_tab(Tab::Files),
            KeyCode::Tab => self.set_tab(self.tab.next(1)),
            KeyCode::BackTab => self.set_tab(self.tab.next(-1)),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::PageDown => self.move_cursor(page),
            KeyCode::PageUp => self.move_cursor(-page),
            KeyCode::Home | KeyCode::Char('g') => self.move_cursor(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_cursor(isize::MAX / 2),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.scroll_detail(8),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.scroll_detail(-8),
            KeyCode::Char(']') => self.scroll_detail(3),
            KeyCode::Char('[') => self.scroll_detail(-3),
            KeyCode::Enter => self.activate(),
            KeyCode::Char('o') => self.fetch_attachment(FileAction::Open),
            KeyCode::Char('d') => self.fetch_attachment(FileAction::Save),
            KeyCode::Char('P') => self.fetch_attachment(FileAction::Print),
            KeyCode::Char('a') => {
                let n = self.attachments().len();
                self.att_sel = if n == 0 { 0 } else { (self.att_sel + 1) % n };
            }
            _ if self.tab == Tab::Assignments => self.on_assignments_key(key),
            _ if self.tab == Tab::Classes => self.on_classes_key(key),
            _ => {}
        }
    }

    fn on_classes_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Right | KeyCode::Char('l') => {
                if self.selected_class().is_some() {
                    self.focus = Focus::Sub;
                }
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc => self.focus = Focus::Classes,
            KeyCode::Char('.') | KeyCode::Char('>') => self.set_class_tab(self.class_tab.next(1)),
            KeyCode::Char(',') | KeyCode::Char('<') => self.set_class_tab(self.class_tab.next(-1)),
            KeyCode::Char('f') => self.filter_by_class(),
            _ => {}
        }
    }

    fn on_assignments_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('t') => self.set_status(Status::Todo),
            KeyCode::Char('i') => self.set_status(Status::InProgress),
            KeyCode::Char('c') => self.set_status(Status::Completed),
            KeyCode::Char('p') => {
                self.show_past = !self.show_past;
                self.rebuild_rows();
            }
            KeyCode::Char('h') => {
                self.hide_done = !self.hide_done;
                self.rebuild_rows();
            }
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.rebuild_rows();
                }
            }
            KeyCode::Char('n') => self.open_new_task(),
            KeyCode::Char('e') => match self.selected_item().filter(|i| i.is_task()) {
                Some(item) => {
                    let due = item.due.map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
                    self.modal = Some(Modal::NewTask(NewTask {
                        desc: Input::with(&item.name),
                        due: Input::with(&due),
                        field: 0,
                        error: None,
                        editing: Some(item.id),
                    }));
                }
                None if self.selected_item().is_some() => self.toast("Only your own tasks can be edited", true),
                None => {}
            },
            KeyCode::Char('l') => match self.selected_item().filter(|i| i.is_task()) {
                Some(item) => self.modal = Some(Modal::Link(LinkPicker { task_id: item.id, filter: Input::default(), sel: 0 })),
                None if self.selected_item().is_some() => self.toast("Only your own tasks can be linked to a class", true),
                None => {}
            },
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(item) = self.selected_item().filter(|i| i.is_task()) {
                    self.modal = Some(Modal::ConfirmDelete { id: item.id, name: item.name.clone() });
                } else if self.selected_item().is_some() {
                    self.toast("Only your own tasks can be deleted", true);
                }
            }
            _ => {}
        }
    }

    fn open_new_task(&mut self) {
        let tomorrow = (self.now + Duration::days(1)).format("%Y-%m-%d").to_string();
        self.modal =
            Some(Modal::NewTask(NewTask { desc: Input::default(), due: Input::with(&tomorrow), field: 0, error: None, editing: None }));
    }

    fn on_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.filtering = false,
            KeyCode::Esc => {
                self.filtering = false;
                self.filter.clear();
                self.rebuild_rows();
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.rebuild_rows();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c);
                self.rebuild_rows();
            }
            _ => {}
        }
    }

    fn on_modal_key(&mut self, key: KeyEvent) {
        let link_len = match &self.modal {
            Some(Modal::Link(lp)) => self.link_choices(&lp.filter.text).len(),
            _ => 0,
        };
        match (&mut self.modal, key.code) {
            (Some(Modal::ConfirmPrint { path, name }), KeyCode::Char('y') | KeyCode::Enter) => {
                let (path, name) = (std::mem::take(path), std::mem::take(name));
                self.modal = None;
                self.send_fetch(path, name, FileAction::Print);
            }
            (Some(Modal::ConfirmPrint { .. }), KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q')) => self.modal = None,
            (Some(Modal::Link(_)), KeyCode::Esc) => self.modal = None,
            (Some(Modal::Link(lp)), KeyCode::Down) => lp.sel = (lp.sel + 1).min(link_len.saturating_sub(1)),
            (Some(Modal::Link(lp)), KeyCode::Up) => lp.sel = lp.sel.saturating_sub(1),
            (Some(Modal::Link(_)), KeyCode::Enter) => self.submit_link(),
            (Some(Modal::Link(lp)), _) => {
                if lp.filter.handle(&key) {
                    lp.sel = 0;
                }
            }
            (Some(Modal::Login { failed: Some(_), .. }), KeyCode::Char('r')) => {
                self.modal = None;
                self.refresh();
            }
            (Some(Modal::Help), _) | (Some(Modal::Login { .. }), KeyCode::Esc) => self.modal = None,
            (Some(Modal::ConfirmDelete { id, .. }), KeyCode::Char('y') | KeyCode::Enter) => {
                let id = *id;
                self.modal = None;
                self.send(Cmd::DeleteTask(id));
            }
            (Some(Modal::ConfirmDelete { .. }), KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q')) => self.modal = None,
            (Some(Modal::NewTask(_)), KeyCode::Esc) => self.modal = None,
            (Some(Modal::NewTask(nt)), KeyCode::Tab | KeyCode::BackTab | KeyCode::Down | KeyCode::Up) => nt.field = 1 - nt.field,
            (Some(Modal::NewTask(_)), KeyCode::Enter) => self.submit_new_task(),
            (Some(Modal::NewTask(nt)), _) => {
                nt.error = None;
                let field = if nt.field == 0 { &mut nt.desc } else { &mut nt.due };
                field.handle(&key);
            }
            _ => {}
        }
    }

    fn submit_new_task(&mut self) {
        let Some(Modal::NewTask(nt)) = &mut self.modal else { return };
        let desc = nt.desc.text.trim().to_owned();
        if desc.is_empty() || desc.chars().count() > 255 {
            nt.error = Some("Describe the task (1-255 characters)".into());
            nt.field = 0;
            return;
        }
        let Some(due) = fmt::parse_due(&nt.due.text, self.now) else {
            nt.error = Some("Due: try 2026-10-02, tomorrow, fri 3pm".into());
            nt.field = 1;
            return;
        };
        let editing = nt.editing;
        self.modal = None;
        match editing {
            Some(id) => self.send(Cmd::EditTask { id, patch: TaskPatch { description: Some(desc), due: Some(due), ..Default::default() } }),
            None => self.send(Cmd::CreateTask { description: desc, due }),
        }
    }

    /// Indices into [`App::items`] the link picker offers; `None` first, meaning "no link".
    pub fn link_choices(&self, filter: &str) -> Vec<Option<usize>> {
        let q = filter.to_lowercase();
        let hit = |i: &CenterItem| q.is_empty() || i.name.to_lowercase().contains(&q) || i.class.to_lowercase().contains(&q);
        std::iter::once(None).chain(self.items.iter().enumerate().filter(|(_, i)| !i.is_task() && hit(i)).map(|(n, _)| Some(n))).collect()
    }

    fn submit_link(&mut self) {
        let Some(Modal::Link(lp)) = &self.modal else { return };
        let task_id = lp.task_id;
        let choice = self.link_choices(&lp.filter.text).get(lp.sel).copied().flatten();
        let patch = match choice {
            Some(n) => TaskPatch { assignment_index_id: Some(self.items[n].id), ..Default::default() },
            None => TaskPatch { unlink: true, ..Default::default() },
        };
        self.modal = None;
        self.send(Cmd::EditTask { id: task_id, patch });
    }

    fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.att_sel = 0;
        match tab {
            Tab::Classes => self.want_class_data(),
            Tab::Files => self.want_user_files(),
            _ => {}
        }
    }

    fn want_user_files(&mut self) {
        if self.user_files.is_none() {
            self.user_files = Some(Load::Loading);
            self.send(Cmd::UserFiles);
        }
    }

    pub fn refresh(&mut self) {
        self.details.clear();
        self.class_info.clear();
        self.class_rows.clear();
        self.class_info_pending.clear();
        self.class_rows_pending.clear();
        self.detail_pending.clear();
        self.topics.clear();
        self.topic_items.clear();
        self.news.clear();
        self.class_files.clear();
        self.grades.clear();
        self.people.clear();
        self.user_files = None;
        self.start();
        match self.tab {
            Tab::Classes => self.want_class_data(),
            Tab::Files => self.want_user_files(),
            _ => self.want_detail(),
        }
        self.toast("Refreshing…", false);
    }

    fn move_cursor(&mut self, delta: isize) {
        match self.tab {
            Tab::Assignments => self.move_sel(delta),
            Tab::Classes if self.focus == Focus::Sub => {
                self.sub_sel = clamp_move(self.sub_sel, delta, self.sub_len());
                self.sub_offset = scroll_into_view(self.sub_sel, self.sub_offset, self.sub_height());
                self.class_scroll = 0;
                self.att_sel = 0;
                self.want_class_data();
            }
            Tab::Classes => {
                let before = self.class_sel;
                self.class_sel = clamp_move(self.class_sel, delta, self.classes.len());
                self.class_offset = scroll_into_view(self.class_sel, self.class_offset, self.list_height());
                if self.class_sel != before {
                    self.reset_sub();
                }
                self.want_class_data();
            }
            Tab::Feed => {
                self.feed_sel = clamp_move(self.feed_sel, delta, self.feed.len());
                self.feed_scroll = 0;
                self.feed_offset = scroll_into_view(self.feed_sel, self.feed_offset, self.list_height());
            }
            Tab::Files => {
                let len = match &self.user_files {
                    Some(Load::Ready(v)) => v.len(),
                    _ => 0,
                };
                self.file_sel = clamp_move(self.file_sel, delta, len);
                self.file_scroll = 0;
                self.file_offset = scroll_into_view(self.file_sel, self.file_offset, self.list_height());
            }
        }
    }

    fn scroll_detail(&mut self, by: isize) {
        let scroll = match self.tab {
            Tab::Assignments => &mut self.detail_scroll,
            Tab::Classes => &mut self.class_scroll,
            Tab::Feed => &mut self.feed_scroll,
            Tab::Files => &mut self.file_scroll,
        };
        *scroll = (*scroll as isize + by).max(0) as usize;
    }

    /// Enter: filter to a class, or jump from a feed post to its assignment.
    fn activate(&mut self) {
        match self.tab {
            Tab::Classes if self.focus == Focus::Classes => {
                if self.selected_class().is_some() {
                    self.focus = Focus::Sub;
                }
            }
            Tab::Classes => match self.class_tab {
                ClassTab::Work => {
                    if let Some(id) = self.selected_work().map(|r| r.id) {
                        self.open_assignment(id);
                    }
                }
                ClassTab::Topics | ClassTab::Files => self.fetch_attachment(FileAction::Open),
                _ => {}
            },
            Tab::Feed => {
                if let Some(id) = self.feed.get(self.feed_sel).and_then(|f| f.assignment_id) {
                    self.open_assignment(id);
                }
            }
            Tab::Files => self.fetch_attachment(FileAction::Open),
            Tab::Assignments => {}
        }
    }

    /// Show that assignment in the Assignments view (past work and the filter are adjusted to reveal it).
    fn open_assignment(&mut self, id: i64) {
        self.filter.clear();
        self.show_past = true;
        self.hide_done = false;
        self.rebuild_rows();
        if let Some(row) = self.rows.iter().position(|r| matches!(r, Row::Item(i) if self.items[*i].id == id && !self.items[*i].is_task()))
        {
            self.tab = Tab::Assignments;
            self.select_row(row);
        } else {
            self.toast("That assignment is not in your assignment center", true);
        }
    }

    /// Show only the selected class's work in the Assignments view.
    fn filter_by_class(&mut self) {
        if let Some(c) = self.selected_class() {
            self.filter = fmt::short_class(&c.name, 99);
            self.show_past = true;
            self.tab = Tab::Assignments;
            self.rebuild_rows();
        }
    }

    // ------------------------------------------------------------------ mouse

    pub fn on_mouse(&mut self, m: MouseEvent) {
        let pos = Position::new(m.column, m.row);
        match m.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let by = if m.kind == MouseEventKind::ScrollDown { 3 } else { -3 };
                if self.modal.is_some() {
                    return;
                }
                if self.hits.detail.contains(pos) {
                    self.scroll_detail(by);
                } else if self.hits.list.contains(pos) {
                    self.scroll_list(by);
                } else if self.hits.sub_list.contains(pos) {
                    let max = self.sub_len().saturating_sub(self.sub_height());
                    self.sub_offset = ((self.sub_offset as isize + by).max(0) as usize).min(max);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => self.click(pos),
            _ => {}
        }
    }

    /// Wheel over a list moves the viewport, not the selection.
    fn scroll_list(&mut self, by: isize) {
        let height = self.list_height();
        let files = match &self.user_files {
            Some(Load::Ready(v)) => v.len(),
            _ => 0,
        };
        let (offset, total) = match self.tab {
            Tab::Assignments => (&mut self.offset, self.rows.len()),
            Tab::Classes => (&mut self.class_offset, self.classes.len()),
            Tab::Feed => (&mut self.feed_offset, self.feed.len()),
            Tab::Files => (&mut self.file_offset, files),
        };
        let max = total.saturating_sub(height);
        *offset = ((*offset as isize + by).max(0) as usize).min(max);
    }

    fn click(&mut self, pos: Position) {
        if self.modal.is_some() {
            let action = self.hits.modal_buttons.iter().find(|(r, _)| r.contains(pos)).map(|(_, a)| *a);
            match (action, &self.modal) {
                (Some(ModalAction::Cancel), _) => self.modal = None,
                (Some(ModalAction::Submit), Some(Modal::ConfirmDelete { id, .. })) => {
                    let id = *id;
                    self.modal = None;
                    self.send(Cmd::DeleteTask(id));
                }
                (Some(ModalAction::Submit), Some(Modal::NewTask(_))) => self.submit_new_task(),
                (Some(ModalAction::Submit), Some(Modal::ConfirmPrint { path, name })) => {
                    let (path, name) = (path.clone(), name.clone());
                    self.modal = None;
                    self.send_fetch(path, name, FileAction::Print);
                }
                (None, Some(Modal::Link(_))) => {
                    if let Some(row) = self.hits.modal_rows.iter().find(|(r, _)| r.contains(pos)).map(|(_, n)| *n) {
                        if let Some(Modal::Link(lp)) = &mut self.modal {
                            lp.sel = row;
                        }
                        self.submit_link();
                    }
                }
                (None, Some(Modal::Help)) => self.modal = None, // click anywhere to dismiss
                _ => {}
            }
            return;
        }
        if let Some((_, tab)) = self.hits.tabs.iter().find(|(r, _)| r.contains(pos)) {
            let tab = *tab;
            return self.set_tab(tab);
        }
        if let Some((_, status)) = self.hits.buttons.iter().find(|(r, _)| r.contains(pos)) {
            let status = *status;
            return self.set_status(status);
        }
        if let Some((_, tab)) = self.hits.sub_tabs.iter().find(|(r, _)| r.contains(pos)) {
            let tab = *tab;
            return self.set_class_tab(tab);
        }
        if let Some((_, idx)) = self.hits.attachments.iter().find(|(r, _)| r.contains(pos)) {
            self.att_sel = *idx;
            return self.fetch_attachment(FileAction::Open);
        }
        if let Some((_, idx)) = self.hits.sub_rows.iter().find(|(r, _)| r.contains(pos)) {
            let idx = *idx;
            self.focus = Focus::Sub;
            if idx < self.sub_len() && idx != self.sub_sel {
                self.sub_sel = idx;
                self.class_scroll = 0;
                self.att_sel = 0;
                self.want_class_data();
            }
            return;
        }
        if let Some((_, idx)) = self.hits.rows.iter().find(|(r, _)| r.contains(pos)) {
            let idx = *idx;
            match self.tab {
                Tab::Assignments => self.select_row(idx),
                Tab::Classes => {
                    self.focus = Focus::Classes;
                    if idx < self.classes.len() && idx != self.class_sel {
                        self.class_sel = idx;
                        self.reset_sub();
                        self.want_class_data();
                    }
                }
                Tab::Feed => {
                    if idx < self.feed.len() && idx != self.feed_sel {
                        self.feed_sel = idx;
                        self.feed_scroll = 0;
                    }
                }
                Tab::Files => {
                    if idx != self.file_sel && matches!(&self.user_files, Some(Load::Ready(v)) if idx < v.len()) {
                        self.file_sel = idx;
                        self.file_scroll = 0;
                    }
                }
            }
        }
    }
}

fn clamp_move(sel: usize, delta: isize, len: usize) -> usize {
    ((sel as isize).saturating_add(delta)).clamp(0, len.saturating_sub(1) as isize) as usize
}

fn scroll_into_view(sel: usize, offset: usize, height: usize) -> usize {
    if sel < offset {
        sel
    } else if sel >= offset + height {
        sel + 1 - height
    } else {
        offset
    }
}
