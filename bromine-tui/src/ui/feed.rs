//! The Feed view: school posts, class announcements and new topics, newest first.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::widgets::*;
use crate::app::App;
use crate::data::{FeedKind, Key};
use crate::fmt;

fn tag(kind: FeedKind) -> (&'static str, Color) {
    match kind {
        FeedKind::Post => ("post ", Color::Magenta),
        FeedKind::News => ("news ", Color::Blue),
        FeedKind::Topic => ("topic", Color::Green),
    }
}

pub fn draw_feed(app: &mut App, f: &mut Frame, area: Rect) {
    let [left, right] = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let block = pane(Line::from(format!(" Feed ({}) ", app.feed.len())), true);
    let inner = block.inner(left);
    f.render_widget(block, left);
    app.hits.list = inner;
    if app.feed.is_empty() {
        let text = if app.store.fetched(&Key::Feed).is_some() { "Nothing here yet." } else { "Loading…" };
        f.render_widget(Paragraph::new(text).style(dim()), inner);
    } else {
        let lines: Vec<Line> = app
            .feed
            .iter()
            .map(|p| {
                let when = p.when.map(|w| fmt::due_label(Some(w), app.now)).unwrap_or_default();
                let title = p
                    .title
                    .clone()
                    .or_else(|| p.text.as_ref().and_then(|t| t.lines().next().map(str::to_owned)))
                    .unwrap_or_else(|| "(post)".into());
                let (label, color) = tag(p.kind);
                Line::from(vec![
                    Span::styled(fmt::fit(&when, 11), dim()),
                    Span::raw(" "),
                    Span::styled(label, Style::new().fg(color)),
                    Span::styled(if p.assignment_id.is_some() { " ▸ " } else { "   " }, Style::new().fg(ACCENT)),
                    Span::raw(fmt::truncate(&title, (inner.width as usize).saturating_sub(21))),
                ])
            })
            .collect();
        app.hits.rows = render_rows(f, inner, lines, app.feed_offset, Some(app.feed_sel));
    }

    let block = pane(Line::from(" Post "), false);
    let inner = block.inner(right);
    f.render_widget(block, right);
    app.hits.detail = inner;
    let Some(p) = app.feed.get(app.feed_sel).cloned() else { return };
    let width = inner.width.saturating_sub(1) as usize;
    let mut lines: Vec<Line> = Vec::new();
    if let Some(t) = &p.title {
        lines.extend(fmt::wrap(t, width).into_iter().map(|l| Line::styled(l, bold())));
    }
    let mut meta = Vec::new();
    meta.extend(p.from.clone().map(|f| Span::styled(f, Style::new().fg(Color::Blue))));
    meta.extend(p.class.clone().map(|c| Span::styled(format!(" · {c}"), dim())));
    meta.extend(p.when.map(|w| Span::styled(format!(" · {}", fmt::long_when(w)), dim())));
    lines.push(Line::from(meta));
    if p.assignment_id.is_some() {
        lines.push(Line::styled("Enter opens this assignment", Style::new().fg(ACCENT).add_modifier(Modifier::empty())));
    }
    if let Some(t) = &p.text {
        lines.push(Line::default());
        lines.extend(fmt::wrap(t, width).into_iter().map(Line::from));
    }
    let scroll = app.feed_scroll;
    app.feed_scroll = draw_body(app, f, inner, inner, lines, scroll);
}
