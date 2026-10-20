//! Text fields and the dialogs that use them.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
