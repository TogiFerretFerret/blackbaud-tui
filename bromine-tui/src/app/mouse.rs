//! Mouse handling: clicks and the wheel are mapped back to what was drawn (see [`super::Hits`]).

use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use super::{App, Focus, Modal, ModalAction, Tab};
use crate::worker::{Cmd, FileAction};

impl App {
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
        let files = self.store.user_files().ready().map_or(0, Vec::len);
        let classes = self.classes().len();
        let (offset, total) = match self.tab {
            Tab::Assignments => (&mut self.offset, self.rows.len()),
            Tab::Classes => (&mut self.class_offset, classes),
            Tab::Feed => (&mut self.feed_offset, self.feed.len()),
            Tab::Files => (&mut self.file_offset, files),
        };
        let max = total.saturating_sub(height);
        *offset = ((*offset as isize + by).max(0) as usize).min(max);
    }

    fn click(&mut self, pos: Position) {
        if self.modal.is_some() {
            return self.click_modal(pos);
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
                    if idx < self.classes().len() && idx != self.class_sel {
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
                    if idx != self.file_sel && idx < self.store.user_files().ready().map_or(0, Vec::len) {
                        self.file_sel = idx;
                        self.file_scroll = 0;
                    }
                }
            }
        }
    }

    fn click_modal(&mut self, pos: Position) {
        let action = self.hits.modal_buttons.iter().find(|(r, _)| r.contains(pos)).map(|(_, a)| *a);
        match (action, &self.modal) {
            (Some(ModalAction::Cancel), _) => self.modal = None,
            (Some(ModalAction::Submit), Some(Modal::ConfirmDelete { id, .. })) => {
                let id = *id;
                self.modal = None;
                self.send_other(Cmd::DeleteTask(id));
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
    }
}
