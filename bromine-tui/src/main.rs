//! A mouse-and-keyboard terminal UI for the Hunter Schools Blackbaud LMS.

mod app;
mod demo;
mod files;
mod fmt;
mod ui;
mod worker;

#[cfg(test)]
mod tests;

use std::io;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use bromine::{Endpoints, SessionStore};
use chrono::Local;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event as Term};
use ratatui::crossterm::execute;

use app::App;
use worker::{Cmd, Event};

const HELP: &str = "bromine-tui: browse and update your assignments in the terminal

Usage: bromine-tui [--demo]

  --demo   run against a fake in-memory LMS (no network, no sign-in)

Keys: press ? inside the app. Signs in through your browser when the saved session has lapsed
(needs capture.user.js in your userscript manager).";

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return Ok(());
    }
    if let Some(bad) = args.iter().find(|a| *a != "--demo") {
        eprintln!("unknown argument {bad:?}\n\n{HELP}");
        std::process::exit(2);
    }
    let demo = args.iter().any(|a| a == "--demo");

    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let now = Local::now().naive_local();
    if demo {
        demo::spawn(cmd_rx, ev_tx, now);
    } else {
        let mut store = SessionStore::default();
        if store.username().is_none() {
            store = store.with_username(ask_username()?);
        }
        worker::spawn(store, Endpoints::default(), cmd_rx, ev_tx);
    }
    let mut app = App::new(cmd_tx.clone(), now);
    app.start();

    // Put the terminal back even if we panic.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal, &mut app, ev_rx);
    restore();
    let _ = cmd_tx.send(Cmd::Quit); // the worker may be mid-request or waiting on a sign-in: do not join it
    result
}

/// First run, before the terminal UI starts: the sign-in needs the school username (set `BB_USERNAME` to skip this).
fn ask_username() -> io::Result<String> {
    use std::io::Write;
    loop {
        print!("School username (the part of your school email before the @): ");
        io::stdout().flush()?;
        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            return Err(io::Error::other("no username: set BB_USERNAME"));
        }
        let name = line.trim().split('@').next().unwrap_or("").trim().to_owned();
        if !name.is_empty() {
            return Ok(name);
        }
    }
}

fn restore() {
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
}

fn run(terminal: &mut DefaultTerminal, app: &mut App, events: Receiver<Event>) -> io::Result<()> {
    let tick = Duration::from_millis(100);
    let mut last_tick = Instant::now();
    loop {
        app.now = Local::now().naive_local();
        terminal.draw(|f| ui::draw(app, f))?;
        if app.should_quit {
            return Ok(());
        }
        if event::poll(tick.saturating_sub(last_tick.elapsed()))? {
            match event::read()? {
                Term::Key(key) => app.on_key(key),
                Term::Mouse(mouse) => app.on_mouse(mouse),
                _ => {} // resize etc.: the next draw adapts
            }
        }
        while let Ok(ev) = events.try_recv() {
            app.on_event(ev);
        }
        if last_tick.elapsed() >= tick {
            app.tick();
            last_tick = Instant::now();
        }
    }
}
