//! Tests for the app logic and the screens, driven through the same entry points the real loop uses.

use std::sync::mpsc::{Receiver, channel};

use bromine::Status;
use chrono::{NaiveDate, NaiveDateTime};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::app::{App, ClassTab, Focus, Modal, Row, Tab};
use crate::data::{Key, Load, Payload, Store};
use crate::demo;
use crate::ui;
use crate::worker::{Cmd, Event, FileAction};

mod classes;
mod core;
mod flow;

pub(super) fn now() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 9, 30).unwrap().and_hms_opt(17, 40, 0).unwrap() // a Wednesday
}

/// Hand the app an answer, as the worker would.
pub(super) fn deliver(app: &mut App, key: Key, payload: Payload) {
    app.on_event(Event::Loaded { key, result: Ok(payload) });
}

pub(super) fn fail(app: &mut App, key: Key, error: &str) {
    app.on_event(Event::Loaded { key, result: Err(error.to_owned()) });
}

/// An app loaded with the demo data, as if the worker had answered the first three commands.
pub(super) fn loaded() -> (App, Receiver<Cmd>) {
    let (tx, rx) = channel();
    let mut app = App::new(tx, now(), Store::new(None));
    app.start();
    deliver(&mut app, Key::Center, Payload::Center(demo::items(now())));
    deliver(&mut app, Key::Classes, Payload::Classes(demo::classes()));
    deliver(&mut app, Key::Feed, Payload::Feed(demo::feed(now())));
    (app, rx)
}

pub(super) fn sent(rx: &Receiver<Cmd>) -> Vec<Cmd> {
    rx.try_iter().collect()
}

/// The keys of the reads requested since the last look.
pub(super) fn loads(rx: &Receiver<Cmd>) -> Vec<Key> {
    sent(rx).into_iter().filter_map(|c| if let Cmd::Load(k) = c { Some(k) } else { None }).collect()
}

pub(super) fn screen(app: &mut App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ui::draw(app, f)).unwrap();
    let buf = term.backend().buffer();
    (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_owned()).collect::<Vec<_>>().join("\n")
}

pub(super) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }
}

pub(super) fn press(app: &mut App, c: char) {
    app.on_key(key(KeyCode::Char(c)));
}

pub(super) fn type_text(app: &mut App, text: &str) {
    text.chars().for_each(|c| press(app, c));
}

pub(super) fn click(app: &mut App, x: u16, y: u16) {
    app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE });
}

pub(super) fn wheel(app: &mut App, x: u16, y: u16, down: bool) {
    let kind = if down { MouseEventKind::ScrollDown } else { MouseEventKind::ScrollUp };
    app.on_mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
}

pub(super) fn selected_id(app: &App) -> i64 {
    app.selected_item().expect("a selected item").id
}

pub(super) fn item_status(app: &App, id: i64) -> Status {
    app.items().iter().find(|i| i.id == id).unwrap().status
}

/// Answer every read the app has asked for so far from the demo data, until it goes quiet.
pub(super) fn answer_all(app: &mut App, rx: &Receiver<Cmd>) {
    let items = demo::items(now());
    loop {
        let keys = loads(rx);
        if keys.is_empty() {
            return;
        }
        for key in keys {
            let result = demo::payload(&key, &items, now());
            app.on_event(Event::Loaded { key, result });
        }
    }
}

/// Like [`answer_all`], but report what was asked (first round only).
pub(super) fn answered(app: &mut App, rx: &Receiver<Cmd>) -> Vec<Key> {
    let first = loads(rx);
    let items = demo::items(now());
    for key in &first {
        app.on_event(Event::Loaded { key: key.clone(), result: demo::payload(key, &items, now()) });
    }
    answer_all(app, rx);
    first
}

pub(super) fn class_view(app: &mut App, rx: &Receiver<Cmd>, tab: ClassTab) {
    press(app, '2');
    while app.class_tab != tab {
        press(app, '>');
    }
    answer_all(app, rx);
}
