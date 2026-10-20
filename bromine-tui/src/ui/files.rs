//! The Files view.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::widgets::*;
use crate::app::App;
use crate::data::Load;
use crate::fmt;

// ---------------------------------------------------------------------------- files

pub fn draw_files(app: &mut App, f: &mut Frame, area: Rect) {
    let [left, right] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    let block = pane(Line::from(" School files "), true);
    let inner = block.inner(left);
    f.render_widget(block, left);
    app.hits.list = inner;
    let mut message = None;
    let mut lines: Vec<Line<'static>> = vec![];
    match app.store.user_files() {
        Load::Loading => message = Some(Line::styled("Loading…", dim())),
        Load::Failed(e) => message = Some(Line::styled(e.to_owned(), Style::new().fg(Color::Red))),
        Load::Ready(v) if v.is_empty() => message = Some(Line::styled("The school has not published any files to you.", dim())),
        Load::Ready(v) => {
            lines = v
                .iter()
                .map(|x| {
                    Line::from(vec![
                        Span::raw(" "),
                        Span::styled(fmt::fit(x.folder.as_deref().unwrap_or(""), 14), dim()),
                        Span::raw(" "),
                        Span::styled(fmt::truncate(&x.name, (inner.width as usize).saturating_sub(17)), Style::new().fg(Color::Blue)),
                    ])
                })
                .collect();
        }
    }
    match message {
        Some(m) => f.render_widget(Paragraph::new(m), inner),
        None => app.hits.rows = render_rows(f, inner, lines, app.file_offset, Some(app.file_sel)),
    }

    let block = pane(Line::from(" File "), false);
    let inner = block.inner(right);
    f.render_widget(block, right);
    app.hits.detail = inner;
    let Some(x) = app.selected_user_file().cloned() else { return };
    let width = inner.width.saturating_sub(1) as usize;
    let mut lines: Vec<Line> = fmt::wrap(&x.name, width).into_iter().map(|l| Line::styled(l, bold())).collect();
    lines.extend(x.folder.clone().map(|d| kv("Folder", d)));
    lines.push(Line::default());
    lines.push(Line::styled("Enter or o opens it, d saves it to your Downloads folder.", dim()));
    let scroll = app.file_scroll;
    app.file_scroll = draw_body(app, f, inner, inner, lines, scroll);
}
