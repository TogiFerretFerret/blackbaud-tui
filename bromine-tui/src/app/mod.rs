//! UI state and input handling. Pure logic: no terminal, no network. Commands go out through a channel
//! and results come back as [`Event`]s, so everything here is testable.
//!
//! What the LMS told us lives in the [`Store`]; this module decides what to ask for. Views show whatever
//! the store has straight away and the app quietly refetches what has gone stale ([`App::ensure`]), so a
//! refresh never blanks the screen.

mod input;
mod keys;
mod mouse;

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use bromine::{AssignmentRow, Bucket, CenterItem, Class, ClassFile, FileRef, GradeRow, Grades, ItemKind, Status, Topic, UserFile};
use chrono::NaiveDateTime;
use ratatui::layout::Rect;

pub use input::{Input, LinkPicker, Modal, ModalAction, NewTask};

use crate::cache;
use crate::data::{FeedEntry, Key, Load, Payload, Store};
use crate::worker::{Cmd, Event, FileAction};

/// How often (in ticks, about ten a second) the app quietly refreshes what is on screen, and saves the cache.
const POLL_TICKS: u32 = 100;
const SAVE_TICKS: u32 = 50;

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

/// One line of the Grades view: a marking period (with its total) or one graded assignment.
pub enum GradeEntry<'a> {
    Period(&'a Grades),
    Row(&'a GradeRow),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Header(Bucket),
    /// Index into the assignment center's items.
    Item(usize),
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
    /// Classes tab: the right-hand list.
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
    pub store: Store,

    // assignments
    pub rows: Vec<Row>,
    pub sel: usize,
    pub offset: usize,
    pub show_past: bool,
    pub hide_done: bool,
    pub filter: String,
    pub filtering: bool,
    pub detail_scroll: usize,
    previous_status: HashMap<(i64, bool), Vec<Status>>,

    // classes
    pub class_sel: usize,
    pub class_offset: usize,
    pub class_scroll: usize,
    pub class_tab: ClassTab,
    pub focus: Focus,
    pub sub_sel: usize,
    pub sub_offset: usize,

    // feed (the combined feed, rebuilt when its sources change)
    pub feed: Vec<FeedEntry>,
    pub feed_sel: usize,
    pub feed_offset: usize,
    pub feed_scroll: usize,

    // files (the school's files for you)
    pub file_sel: usize,
    pub file_offset: usize,
    pub file_scroll: usize,
    /// Which of the shown attachments `o`, `d` and `P` act on.
    pub att_sel: usize,

    pub toast: Option<Toast>,
    pub modal: Option<Modal>,
    pub spinner: usize,
    /// Writes and downloads in flight (reads are `store.loading`).
    other_busy: usize,
    tick_count: u32,
    dirty: bool,
    writer: Option<cache::Writer>,
    tx: Sender<Cmd>,
}

impl App {
    /// `store` is what the on-disk cache held (or an empty one).
    pub fn new(tx: Sender<Cmd>, now: NaiveDateTime, store: Store) -> Self {
        Self {
            tab: Tab::Assignments,
            now,
            should_quit: false,
            hits: Hits::default(),
            store,
            rows: vec![],
            sel: 0,
            offset: 0,
            show_past: false,
            hide_done: false,
            filter: String::new(),
            filtering: false,
            detail_scroll: 0,
            previous_status: HashMap::new(),
            class_sel: 0,
            class_offset: 0,
            class_scroll: 0,
            class_tab: ClassTab::Work,
            focus: Focus::Classes,
            sub_sel: 0,
            sub_offset: 0,
            feed: vec![],
            feed_sel: 0,
            feed_offset: 0,
            feed_scroll: 0,
            file_sel: 0,
            file_offset: 0,
            file_scroll: 0,
            att_sel: 0,
            toast: None,
            modal: None,
            spinner: 0,
            other_busy: 0,
            tick_count: 0,
            dirty: false,
            writer: None,
            tx,
        }
    }

    /// Save the store to disk as it changes.
    pub fn with_cache(mut self, writer: cache::Writer) -> Self {
        self.writer = Some(writer);
        self
    }

    /// Show what the cache had and ask for whatever is stale.
    pub fn start(&mut self) {
        self.rebuild_rows();
        self.rebuild_feed();
        self.refresh_core(false);
        self.refresh_view(false);
    }

    // ------------------------------------------------------------------ talking to the worker

    /// When the main data of the current view was last fetched.
    pub fn primary_fetched(&self) -> Option<NaiveDateTime> {
        self.store.fetched(&match self.tab {
            Tab::Assignments => Key::Center,
            Tab::Classes => Key::Classes,
            Tab::Feed => Key::Feed,
            Tab::Files => Key::UserFiles,
        })
    }

    /// Anything in flight, for the spinner.
    pub fn busy(&self) -> usize {
        self.store.loading.len() + self.other_busy
    }

    /// Fetch `key` unless it is already on its way.
    fn request(&mut self, key: Key) {
        if self.store.loading.insert(key.clone()) && self.tx.send(Cmd::Load(key.clone())).is_err() {
            self.store.loading.remove(&key); // the worker is gone (shutting down)
        }
    }

    /// Fetch `key` if it is missing or stale; `force` also refetches a fresh one.
    pub fn fetch(&mut self, key: Key, force: bool) {
        if self.store.needs(&key, self.now, force) {
            self.request(key);
        }
    }

    fn send_other(&mut self, cmd: Cmd) {
        self.other_busy += 1;
        if self.tx.send(cmd).is_err() {
            self.other_busy -= 1;
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast { text: text.into(), error, ticks_left: if error { 60 } else { 30 } });
    }

    /// The things every view leans on, and the announcements and topics that make up the feed.
    fn refresh_core(&mut self, force: bool) {
        for key in [Key::Center, Key::Classes, Key::Feed] {
            self.fetch(key, force);
        }
        self.refresh_feed_sources(force);
    }

    fn refresh_feed_sources(&mut self, force: bool) {
        let sections: Vec<i64> = self.classes().iter().map(|c| c.section_id).collect();
        for s in sections {
            self.fetch(Key::News(s), force);
            self.fetch(Key::Topics(s), force);
        }
    }

    /// What the current view shows.
    fn view_keys(&self) -> Vec<Key> {
        match self.tab {
            Tab::Assignments => {
                let mut keys = vec![Key::Center];
                keys.extend(self.selected_item().filter(|i| !i.is_task()).map(|i| Key::Detail(i.id)));
                keys
            }
            Tab::Classes => {
                let mut keys = vec![Key::Classes];
                keys.extend(self.class_view_keys());
                keys
            }
            Tab::Feed => {
                let mut keys = vec![Key::Feed];
                for c in self.classes() {
                    keys.extend([Key::News(c.section_id), Key::Topics(c.section_id)]);
                }
                keys
            }
            Tab::Files => vec![Key::UserFiles],
        }
    }

    fn class_view_keys(&self) -> Vec<Key> {
        let Some(c) = self.selected_class() else { return vec![] };
        let (section, lead) = (c.section_id, c.lead_section_id);
        let mut keys = vec![Key::ClassInfo(section)];
        match self.class_tab {
            ClassTab::About => {}
            ClassTab::Work => {
                keys.push(Key::ClassRows(lead));
                keys.extend(self.selected_work().map(|r| Key::Detail(r.id)));
            }
            ClassTab::Topics => {
                keys.push(Key::Topics(section));
                keys.extend(self.selected_topic().map(|t| Key::TopicItems { topic: t.topic_id, index: t.index_id }));
            }
            ClassTab::News => keys.push(Key::News(section)),
            ClassTab::Files => keys.push(Key::ClassFiles(section)),
            ClassTab::Grades => keys.push(Key::Grades(section)),
            ClassTab::People => keys.push(Key::People(section)),
        }
        keys
    }

    /// Refresh what is on screen: only what is stale, or all of it when `force`.
    pub fn refresh_view(&mut self, force: bool) {
        for key in self.view_keys() {
            self.fetch(key, force);
        }
    }

    /// Fetch what the Classes view now needs (after the selection or section changed).
    fn want_class_data(&mut self) {
        if self.tab != Tab::Classes {
            return;
        }
        for key in self.class_view_keys() {
            self.fetch(key, false);
        }
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
        self.tick_count += 1;
        if self.tick_count.is_multiple_of(POLL_TICKS) {
            self.poll();
        }
        if self.tick_count.is_multiple_of(SAVE_TICKS) {
            self.save_cache();
        }
    }

    /// Background upkeep: refetch what is stale and visible, without blanking anything.
    fn poll(&mut self) {
        for key in [Key::Center, Key::Feed] {
            self.fetch(key, false);
        }
        self.refresh_view(false);
        if self.tab == Tab::Feed {
            self.refresh_feed_sources(false);
        }
    }

    /// Write the store to disk if anything changed since the last time.
    pub fn save_cache(&mut self) {
        if self.dirty
            && let Some(w) = &self.writer
        {
            w.save(&mut self.store);
            self.dirty = false;
        }
    }

    /// Save what is left to save and wait for it to reach the disk (on the way out).
    pub fn shutdown(&mut self) {
        self.save_cache();
        if let Some(w) = self.writer.take() {
            w.finish();
        }
    }

    // ------------------------------------------------------------------ selection helpers

    pub fn items(&self) -> &[CenterItem] {
        self.store.items()
    }

    pub fn classes(&self) -> &[Class] {
        self.store.class_list()
    }

    pub fn selected_item(&self) -> Option<&CenterItem> {
        match self.rows.get(self.sel)? {
            Row::Item(i) => self.items().get(*i),
            Row::Header(_) => None,
        }
    }

    fn selected_key(&self) -> Option<(ItemKind, i64)> {
        self.selected_item().map(|i| (i.kind, i.id))
    }

    pub fn selected_class(&self) -> Option<&Class> {
        self.classes().get(self.class_sel)
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
                .items()
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
        let found = keep.and_then(|(kind, id)| {
            rows.iter().position(|r| matches!(r, Row::Item(i) if self.items()[*i].kind == kind && self.items()[*i].id == id))
        });
        self.sel = found.or_else(|| rows.iter().position(|r| matches!(r, Row::Item(_)))).unwrap_or(0);
        self.rows = rows;
        self.keep_visible();
        self.want_detail();
    }

    /// Rebuild the combined feed, keeping the same post selected when it is still there.
    pub fn rebuild_feed(&mut self) {
        let keep = self.feed.get(self.feed_sel).map(|f| f.key.clone());
        self.feed = self.store.combined_feed();
        self.feed_sel = keep.and_then(|k| self.feed.iter().position(|f| f.key == k)).unwrap_or(self.feed_sel);
        self.feed_sel = self.feed_sel.min(self.feed.len().saturating_sub(1));
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

    /// Fetch (or refresh) the selected assignment's details.
    fn want_detail(&mut self) {
        if let Some(item) = self.selected_item().filter(|i| !i.is_task()) {
            let key = Key::Detail(item.id);
            self.fetch(key, false);
        }
    }

    // ------------------------------------------------------------------ the Classes view's lists

    pub fn selected_work(&self) -> Option<&AssignmentRow> {
        self.store.class_rows(self.selected_class()?.lead_section_id).ready()?.get(self.sub_sel)
    }

    pub fn selected_topic(&self) -> Option<&Topic> {
        if self.class_tab != ClassTab::Topics {
            return None;
        }
        self.store.topics(self.selected_class()?.section_id).ready()?.get(self.sub_sel)
    }

    pub fn selected_class_file(&self) -> Option<&ClassFile> {
        self.store.class_files(self.selected_class()?.section_id).ready()?.get(self.sub_sel)
    }

    pub fn selected_user_file(&self) -> Option<&UserFile> {
        self.store.user_files().ready()?.get(self.file_sel)
    }

    /// The `i`th line of the Grades view for the selected class.
    pub fn grade_entry(&self, i: usize) -> Option<GradeEntry<'_>> {
        let mut i = i;
        for period in self.store.grades(self.selected_class()?.section_id).ready()? {
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
        let s = c.section_id;
        match self.class_tab {
            ClassTab::About => 0,
            ClassTab::Work => len(self.store.class_rows(c.lead_section_id)),
            ClassTab::Topics => len(self.store.topics(s)),
            ClassTab::News => len(self.store.news(s)),
            ClassTab::Files => len(self.store.class_files(s)),
            ClassTab::People => len(self.store.people(s)),
            ClassTab::Grades => self.store.grades(s).ready().map_or(0, |g| g.iter().map(|p| 1 + p.rows.len()).sum()),
        }
    }

    fn clamp_sub(&mut self) {
        self.sub_sel = self.sub_sel.min(self.sub_len().saturating_sub(1));
        self.sub_offset = self.sub_offset.min(self.sub_len().saturating_sub(1));
        self.class_sel = self.class_sel.min(self.classes().len().saturating_sub(1));
        let files = self.store.user_files().ready().map_or(0, Vec::len);
        self.file_sel = self.file_sel.min(files.saturating_sub(1));
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
        let from_detail = |id: i64| self.store.detail(id).ready().map(|d| d.downloads.clone()).unwrap_or_default();
        let one = |name: &str, path: &str| vec![FileRef { name: name.to_owned(), path: path.to_owned() }];
        match self.tab {
            Tab::Assignments => self.selected_item().filter(|i| !i.is_task()).map(|i| from_detail(i.id)).unwrap_or_default(),
            Tab::Classes => match self.class_tab {
                ClassTab::Work => self.selected_work().map(|r| from_detail(r.id)).unwrap_or_default(),
                ClassTab::Topics => self
                    .selected_topic()
                    .and_then(|t| self.store.topic_items(t).ready())
                    .into_iter()
                    .flatten()
                    .filter_map(|i| {
                        Some(FileRef {
                            name: i.file.clone().or_else(|| i.title.clone()).unwrap_or_else(|| "file".into()),
                            path: i.path.clone()?,
                        })
                    })
                    .collect(),
                ClassTab::Files => self.selected_class_file().map(|f| one(&f.name, &f.path)).unwrap_or_default(),
                _ => vec![],
            },
            Tab::Files => self.selected_user_file().map(|f| one(&f.name, &f.path)).unwrap_or_default(),
            Tab::Feed => vec![],
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
        self.send_other(Cmd::Fetch { path, name, action });
    }

    // ------------------------------------------------------------------ events from the worker

    pub fn on_event(&mut self, ev: Event) {
        if !matches!(ev, Event::Loaded { .. } | Event::LoginNeeded | Event::LoginUrl { .. } | Event::LoginDone | Event::LoginFailed(_)) {
            self.other_busy = self.other_busy.saturating_sub(1);
        }
        match ev {
            Event::Loaded { key, result: Ok(payload) } => self.on_loaded(key, payload),
            Event::Loaded { key, result: Err(e) } => {
                let had_data = self.store.fetched(&key).is_some();
                self.store.fail(&key, e.clone(), self.now);
                // with nothing to show the pane says so itself; with something, a refresh failing is news
                if had_data && matches!(key, Key::Center | Key::Classes | Key::Feed) {
                    self.toast(format!("Could not refresh: {e}"), true);
                }
            }
            Event::StatusSet { id, status, task, result } => {
                let prev = self.previous_status.get_mut(&(id, task)).and_then(|v| (!v.is_empty()).then(|| v.remove(0)));
                match result {
                    Ok(()) => {
                        self.dirty = true;
                        self.toast(format!("Marked as {}", status.label()), false)
                    }
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
                self.fetch(Key::Center, true);
            }
            Event::TaskCreated(Err(e)) => self.toast(e, true),
            Event::TaskEdited(Ok(())) => {
                self.toast("Task updated", false);
                self.fetch(Key::Center, true);
            }
            Event::TaskEdited(Err(e)) => self.toast(e, true),
            Event::TaskDeleted { id, result } => match result {
                Ok(()) => {
                    self.store.items_mut().retain(|i| !(i.is_task() && i.id == id));
                    self.dirty = true;
                    self.rebuild_rows();
                    self.toast("Task deleted", false);
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

    fn on_loaded(&mut self, key: Key, payload: Payload) {
        self.store.apply(&key, payload, self.now);
        self.dirty = true;
        match key {
            Key::Center => self.rebuild_rows(),
            Key::Classes => {
                self.refresh_feed_sources(false);
                self.rebuild_feed();
                if self.tab == Tab::Classes {
                    self.want_class_data();
                }
            }
            Key::Feed | Key::News(_) => self.rebuild_feed(),
            Key::Topics(_) => {
                self.rebuild_feed();
                self.want_class_data(); // the first topic's content
            }
            Key::ClassRows(_) => self.want_class_data(), // the first row's details
            Key::Detail(id) => {
                // the assignment's own status is the truth: show it in the list too
                if let Some(status) = self.store.detail(id).ready().map(|d| d.status)
                    && let Some(item) = self.store.items_mut().iter_mut().find(|i| !i.is_task() && i.id == id)
                {
                    item.status = status;
                }
            }
            _ => {}
        }
    }

    fn apply_status(&mut self, id: i64, task: bool, status: Status) {
        for item in self.store.items_mut().iter_mut().filter(|i| i.id == id && i.is_task() == task) {
            item.status = status;
        }
        if !task && let Some(e) = self.store.details.get_mut(&id) {
            e.value.status = status;
        }
        self.dirty = true;
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
        self.send_other(Cmd::SetStatus { id, status, task });
    }
}

fn len<T>(l: Load<'_, Vec<T>>) -> usize {
    l.ready().map_or(0, Vec::len)
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
