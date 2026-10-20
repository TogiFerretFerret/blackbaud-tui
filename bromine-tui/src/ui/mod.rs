//! Drawing. Everything clickable is recorded in `app.hits` as it is drawn, so the mouse handler always
//! agrees with what is on screen.

mod assignments;
mod classes;
mod feed;
mod files;
mod modals;
mod widgets;

use bromine::Bucket;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use self::widgets::*;
use crate::app::{App, Focus, Hits, Tab};
use crate::fmt;

use self::assignments::draw_assignments;
use self::classes::draw_classes;
use self::feed::draw_feed;
use self::files::draw_files;
use self::modals::draw_modal;

pub fn draw(app: &mut App, f: &mut Frame) {
    app.hits = Hits::default();
    let area = f.area();
    let [top, body, bottom] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(area);
    draw_top(app, f, top);
    match app.tab {
        Tab::Assignments => draw_assignments(app, f, body),
        Tab::Classes => draw_classes(app, f, body),
        Tab::Feed => draw_feed(app, f, body),
        Tab::Files => draw_files(app, f, body),
    }
    draw_bottom(app, f, bottom);
    if app.modal.is_some() {
        draw_modal(app, f, area);
    }
}

// ---------------------------------------------------------------------------- top & bottom bars

fn draw_top(app: &mut App, f: &mut Frame, area: Rect) {
    let mut spans = vec![Span::styled(" bromine ", Style::new().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)), Span::raw(" ")];
    let mut x = area.x + 10;
    for (n, tab) in Tab::ALL.iter().enumerate() {
        let label = format!(" {} {} ", n + 1, tab.title());
        let w = label.width() as u16;
        app.hits.tabs.push((Rect::new(x, area.y, w, 1), *tab));
        spans.push(if *tab == app.tab {
            Span::styled(label, Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD))
        } else {
            Span::styled(label, dim())
        });
        spans.push(Span::raw(" "));
        x += w + 1;
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);

    let items = app.items();
    let open = items.iter().filter(|i| i.bucket.is_upcoming() && !i.status.is_done()).count();
    let today = items.iter().filter(|i| i.bucket == Bucket::DueToday && !i.status.is_done()).count();
    let mut parts = vec![];
    if app.busy() > 0 {
        parts.push(format!("{} updating", SPINNER[app.spinner % SPINNER.len()]));
    }
    if app.store.center.is_some() {
        parts.push(format!("{open} open · {today} due today"));
    }
    if let Some(at) = app.primary_fetched() {
        parts.push(format!("updated {}", fmt::ago(at, app.now)));
    }
    // as much as fits: on a narrow terminal the tabs matter more
    let used = x.saturating_sub(area.x) as usize;
    while !parts.is_empty() {
        let right = format!("{} ", parts.join("  ·  "));
        if used + right.width() < area.width as usize {
            f.render_widget(Paragraph::new(right).alignment(Alignment::Right).style(dim()), area);
            break;
        }
        parts.remove(0);
    }
}

fn draw_bottom(app: &App, f: &mut Frame, area: Rect) {
    let line = if app.filtering {
        Line::from(vec![
            Span::styled("/", Style::new().fg(ACCENT)),
            Span::raw(app.filter.clone()),
            Span::styled("█", Style::new().fg(ACCENT)),
            Span::styled("   Enter keep · Esc clear", dim()),
        ])
    } else if let Some(t) = &app.toast {
        Line::styled(format!(" {}", t.text), Style::new().fg(if t.error { Color::Red } else { Color::Green }).add_modifier(Modifier::BOLD))
    } else {
        let hint = match app.tab {
            Tab::Assignments => " t/i/c status · n new · e edit · l link · x delete · o/d files · / filter · p past · h hide done · ? help",
            Tab::Classes if app.focus == Focus::Classes => {
                " ↑↓ class · → open it · f its work in Assignments · Tab next view · ? help · q quit"
            }
            Tab::Classes => " ↑↓ select · ←→ class · < > section · Enter open · o open file · d save · P print · ? help",
            Tab::Feed => " ↑↓ select · Enter open the assignment · Tab next view · ? help · q quit",
            Tab::Files => " ↑↓ select · Enter/o open · d save · P print · r refresh · ? help · q quit",
        };
        Line::styled(hint, dim())
    };
    f.render_widget(Paragraph::new(line), area);
}
