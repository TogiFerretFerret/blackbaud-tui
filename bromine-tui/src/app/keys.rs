//! Keyboard handling.

use bromine::{CenterItem, Status, TaskPatch};
use chrono::Duration;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{App, ClassTab, Focus, Input, LinkPicker, Modal, NewTask, Row, Tab, clamp_move, scroll_into_view};
use crate::fmt;
use crate::worker::{Cmd, FileAction};

impl App {
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
            KeyCode::Char('r') => self.refresh(false),
            KeyCode::Char('R') => self.refresh(true),
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
                self.refresh(true);
            }
            (Some(Modal::Help), _) | (Some(Modal::Login { .. }), KeyCode::Esc) => self.modal = None,
            (Some(Modal::ConfirmDelete { id, .. }), KeyCode::Char('y') | KeyCode::Enter) => {
                let id = *id;
                self.modal = None;
                self.send_other(Cmd::DeleteTask(id));
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

    pub(super) fn submit_new_task(&mut self) {
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
            Some(id) => {
                self.send_other(Cmd::EditTask { id, patch: TaskPatch { description: Some(desc), due: Some(due), ..Default::default() } })
            }
            None => self.send_other(Cmd::CreateTask { description: desc, due }),
        }
    }

    /// Indices into the assignment center's items the link picker offers; `None` first, meaning "no link".
    pub fn link_choices(&self, filter: &str) -> Vec<Option<usize>> {
        let q = filter.to_lowercase();
        let hit = |i: &CenterItem| q.is_empty() || i.name.to_lowercase().contains(&q) || i.class.to_lowercase().contains(&q);
        std::iter::once(None).chain(self.items().iter().enumerate().filter(|(_, i)| !i.is_task() && hit(i)).map(|(n, _)| Some(n))).collect()
    }

    pub(super) fn submit_link(&mut self) {
        let Some(Modal::Link(lp)) = &self.modal else { return };
        let task_id = lp.task_id;
        let choice = self.link_choices(&lp.filter.text).get(lp.sel).copied().flatten();
        let patch = match choice {
            Some(n) => TaskPatch { assignment_index_id: Some(self.items()[n].id), ..Default::default() },
            None => TaskPatch { unlink: true, ..Default::default() },
        };
        self.modal = None;
        self.send_other(Cmd::EditTask { id: task_id, patch });
    }

    pub(super) fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.att_sel = 0;
        self.refresh_view(false);
    }

    /// `r` refreshes what is on screen, `R` also everything behind it (assignments, the feed's sources).
    /// Nothing is blanked: the old data stays up until the new arrives.
    fn refresh(&mut self, everything: bool) {
        if everything {
            self.refresh_core(true);
        }
        self.refresh_view(true);
        self.toast(if everything { "Refreshing everything…" } else { "Refreshing…" }, false);
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
                self.class_sel = clamp_move(self.class_sel, delta, self.classes().len());
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
                let len = self.store.user_files().ready().map_or(0, Vec::len);
                self.file_sel = clamp_move(self.file_sel, delta, len);
                self.file_scroll = 0;
                self.file_offset = scroll_into_view(self.file_sel, self.file_offset, self.list_height());
            }
        }
    }

    pub(super) fn scroll_detail(&mut self, by: isize) {
        let scroll = match self.tab {
            Tab::Assignments => &mut self.detail_scroll,
            Tab::Classes => &mut self.class_scroll,
            Tab::Feed => &mut self.feed_scroll,
            Tab::Files => &mut self.file_scroll,
        };
        *scroll = (*scroll as isize + by).max(0) as usize;
    }

    /// Enter: into the class, open the thing selected, or jump from a feed post to its assignment.
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
        let found = self.rows.iter().position(|r| matches!(r, Row::Item(i) if self.items()[*i].id == id && !self.items()[*i].is_task()));
        if let Some(row) = found {
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
}
