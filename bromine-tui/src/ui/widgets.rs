//! Small drawing helpers shared by every view.

use bromine::Status;
use chrono::NaiveDateTime;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::App;
use crate::fmt;

pub const ACCENT: Color = Color::Cyan;
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const SELECTED: Style = Style::new().bg(Color::Indexed(238)).add_modifier(Modifier::BOLD);

pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

pub fn status_glyph(s: Status) -> (&'static str, Color) {
    match s {
        Status::Todo => ("○", Color::White),
        Status::InProgress => ("◐", Color::Yellow),
        Status::Completed => ("●", Color::Green),
        Status::Graded => ("✓", Color::Cyan),
        Status::Overdue => ("!", Color::Red),
        Status::Other(_) => ("?", Color::DarkGray),
    }
}

/// Render `lines` as clickable rows into `inner`, starting at `offset`. Returns the (rect, row index) pairs.
pub fn render_rows(f: &mut Frame, inner: Rect, lines: Vec<Line<'static>>, offset: usize, selected: Option<usize>) -> Vec<(Rect, usize)> {
    let mut rects = Vec::new();
    for (n, line) in lines.into_iter().enumerate().skip(offset).take(inner.height as usize) {
        let rect = Rect::new(inner.x, inner.y + (n - offset) as u16, inner.width, 1);
        let style = if selected == Some(n) { SELECTED } else { Style::new() };
        f.render_widget(Paragraph::new(line).style(style), rect);
        rects.push((rect, n));
    }
    rects
}

pub fn pane(title: Line<'static>, focused: bool) -> Block<'static> {
    Block::bordered().title(title).border_style(if focused { Style::new().fg(ACCENT) } else { dim() })
}

/// Scrollable text in `area`, with the selection's files (if any) pinned underneath as clickable lines.
/// `hit` is where the wheel scrolls it. Returns the scroll position, clamped.
pub fn draw_body(app: &mut App, f: &mut Frame, area: Rect, hit: Rect, lines: Vec<Line<'static>>, scroll: usize) -> usize {
    let atts = app.attachments();
    let shown_atts = atts.len().min(4);
    let foot = if atts.is_empty() { 0 } else { (1 + shown_atts) as u16 }.min(area.height.saturating_sub(1));
    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(foot)]).areas(area);
    app.hits.detail = hit;

    let max_scroll = lines.len().saturating_sub(body.height as usize);
    let scroll = scroll.min(max_scroll);
    let visible: Vec<Line> = lines.into_iter().skip(scroll).take(body.height as usize).collect();
    f.render_widget(Paragraph::new(visible), body);
    if max_scroll > 0 && body.height > 0 {
        let hint = format!("{}/{}", scroll + 1, max_scroll + 1);
        let r = Rect::new(body.right().saturating_sub(hint.len() as u16 + 1), body.bottom().saturating_sub(1), hint.len() as u16, 1);
        f.render_widget(Paragraph::new(hint).style(dim()), r);
    }

    if foot > 0 {
        app.att_sel = app.att_sel.min(atts.len() - 1);
        let first = if app.att_sel < shown_atts { 0 } else { app.att_sel + 1 - shown_atts };
        let head = Rect::new(footer.x, footer.y, footer.width, 1);
        f.render_widget(Paragraph::new("Files · click or o open · d save · P print · a next").style(dim()), head);
        for (row, n) in (first..atts.len()).take(footer.height.saturating_sub(1) as usize).enumerate() {
            let r = Rect::new(footer.x, footer.y + 1 + row as u16, footer.width, 1);
            let on = n == app.att_sel;
            let line = Line::from(vec![
                Span::styled(if on { "▸ " } else { "  " }, Style::new().fg(ACCENT)),
                Span::styled(fmt::truncate(&atts[n].name, (footer.width as usize).saturating_sub(2)), Style::new().fg(Color::Blue)),
            ]);
            f.render_widget(Paragraph::new(line).style(if on { SELECTED } else { Style::new() }), r);
            app.hits.attachments.push((r, n));
        }
    }
    scroll
}

pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

pub fn button(label: &str, style: Style) -> Span<'static> {
    Span::styled(format!(" {label} "), style.add_modifier(Modifier::BOLD))
}

pub fn kv(label: &str, value: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{label} "), dim()), Span::raw(value)])
}

pub fn when_label(d: Option<NaiveDateTime>, now: NaiveDateTime) -> String {
    d.map(|d| fmt::due_label(Some(d), now)).unwrap_or_default()
}
