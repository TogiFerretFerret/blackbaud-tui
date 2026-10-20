//! Dialogs.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::widgets::*;
use crate::app::{App, Modal, ModalAction};
use crate::fmt;

pub fn draw_modal(app: &mut App, f: &mut Frame, area: Rect) {
    let Some(modal) = app.modal.clone() else { return };
    match modal {
        Modal::Help => {
            let keys = [
                ("↑ ↓  j k", "move"),
                ("PgUp PgDn  g G", "page / top / bottom"),
                ("1 2 3 4  Tab", "switch view (or click a tab)"),
                ("t  i  c", "set status: to do / in progress / completed (or click a button)"),
                ("n  e  l  x", "personal tasks: new / edit text and due date / link to a class / delete"),
                ("/", "filter by name or class"),
                ("p", "show / hide past work"),
                ("h", "hide / show completed"),
                ("[ ]  Ctrl-u Ctrl-d", "scroll the detail pane (or the wheel over it)"),
                ("→ ←  < >", "classes: into the class / back · previous / next section"),
                ("f", "classes: show this class's work in Assignments"),
                ("Enter", "classes: open · feed: open the assignment · files: open"),
                ("o  d  P  a", "open / save / print (PDF, double-sided) the file shown · next file"),
                ("r", "refresh everything"),
                ("q  Ctrl-c", "quit"),
            ];
            let h = keys.len() as u16 + 4;
            let rect = centered(area, 78, h);
            f.render_widget(Clear, rect);
            let block = Block::bordered().title(" Keys ").border_style(Style::new().fg(ACCENT));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let mut lines: Vec<Line> = keys
                .iter()
                .map(|(k, d)| Line::from(vec![Span::styled(format!(" {k:<22}"), Style::new().fg(ACCENT)), Span::raw(*d)]))
                .collect();
            lines.push(Line::default());
            lines.push(Line::styled(" Mouse: click tabs, rows and buttons; wheel scrolls. Any key or click closes this.", dim()));
            f.render_widget(Paragraph::new(lines), inner);
        }
        Modal::ConfirmDelete { name, .. } => {
            let rect = centered(area, 56, 7);
            f.render_widget(Clear, rect);
            let block = Block::bordered().title(" Delete task? ").border_style(Style::new().fg(Color::Red));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let mut lines: Vec<Line> = fmt::wrap(&name, (inner.width as usize).saturating_sub(2))
                .into_iter()
                .take(2)
                .map(|l| Line::styled(format!(" {l}"), bold()))
                .collect();
            lines.push(Line::default());
            f.render_widget(Paragraph::new(lines), inner);
            let y = inner.bottom().saturating_sub(1);
            let (del, cancel) = (Rect::new(inner.x + 1, y, 10, 1), Rect::new(inner.x + 13, y, 10, 1));
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    button("Delete", Style::new().fg(Color::Black).bg(Color::Red)),
                    Span::raw("  "),
                    button("Cancel", Style::new().fg(Color::Black).bg(Color::Gray)),
                    Span::styled("   y / n", dim()),
                ]))
                .alignment(Alignment::Left),
                Rect::new(inner.x + 1, y, inner.width.saturating_sub(1), 1),
            );
            app.hits.modal_buttons =
                vec![(Rect::new(del.x, y, 8, 1), ModalAction::Submit), (Rect::new(cancel.x - 1, y, 8, 1), ModalAction::Cancel)];
        }
        Modal::NewTask(nt) => {
            let rect = centered(area, 60, 11);
            f.render_widget(Clear, rect);
            let block = Block::bordered()
                .title(if nt.editing.is_some() { " Edit task " } else { " New task " })
                .border_style(Style::new().fg(ACCENT));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let field = |label: &str, text: &str, active: bool| {
                vec![
                    Line::styled(format!(" {label}"), if active { Style::new().fg(ACCENT) } else { dim() }),
                    Line::from(vec![
                        Span::raw(" "),
                        Span::styled(
                            fmt::fit(text, (inner.width as usize).saturating_sub(3)),
                            Style::new().add_modifier(if active { Modifier::UNDERLINED } else { Modifier::empty() }),
                        ),
                    ]),
                ]
            };
            let mut lines = field("What needs doing?", &nt.desc.text, nt.field == 0);
            lines.push(Line::default());
            lines.extend(field("Due (2026-10-02 · tomorrow · fri 3pm)", &nt.due.text, nt.field == 1));
            if let Some(e) = &nt.error {
                lines.push(Line::styled(format!(" {e}"), Style::new().fg(Color::Red)));
            }
            f.render_widget(Paragraph::new(lines), inner);
            let y = inner.bottom().saturating_sub(1);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    button(if nt.editing.is_some() { "Save" } else { "Add task" }, Style::new().fg(Color::Black).bg(ACCENT)),
                    Span::raw("  "),
                    button("Cancel", Style::new().fg(Color::Black).bg(Color::Gray)),
                    Span::styled("   Tab next · Enter save · Esc cancel", dim()),
                ])),
                Rect::new(inner.x + 1, y, inner.width.saturating_sub(1), 1),
            );
            app.hits.modal_buttons =
                vec![(Rect::new(inner.x + 1, y, 10, 1), ModalAction::Submit), (Rect::new(inner.x + 13, y, 8, 1), ModalAction::Cancel)];
            let (input, row) = if nt.field == 0 { (&nt.desc, inner.y + 1) } else { (&nt.due, inner.y + 4) };
            let col = inner.x + 1 + input.text.chars().take(input.cursor).collect::<String>().width() as u16;
            f.set_cursor_position((col.min(inner.right().saturating_sub(1)), row));
        }
        Modal::ConfirmPrint { name, .. } => {
            let rect = centered(area, 56, 7);
            f.render_widget(Clear, rect);
            let block = Block::bordered().title(" Print? ").border_style(Style::new().fg(Color::Yellow));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let mut lines: Vec<Line> = fmt::wrap(&name, (inner.width as usize).saturating_sub(2))
                .into_iter()
                .take(2)
                .map(|l| Line::styled(format!(" {l}"), bold()))
                .collect();
            lines.push(Line::styled(" Double-sided on the default printer.", dim()));
            f.render_widget(Paragraph::new(lines), inner);
            let y = inner.bottom().saturating_sub(1);
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    button("Print", Style::new().fg(Color::Black).bg(Color::Yellow)),
                    Span::raw("  "),
                    button("Cancel", Style::new().fg(Color::Black).bg(Color::Gray)),
                    Span::styled("   y / n", dim()),
                ])),
                Rect::new(inner.x + 1, y, inner.width.saturating_sub(1), 1),
            );
            app.hits.modal_buttons =
                vec![(Rect::new(inner.x + 1, y, 7, 1), ModalAction::Submit), (Rect::new(inner.x + 10, y, 8, 1), ModalAction::Cancel)];
        }
        Modal::Link(lp) => {
            let rect = centered(area, 70, 18);
            f.render_widget(Clear, rect);
            let block = Block::bordered().title(" Link task to… ").border_style(Style::new().fg(ACCENT));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let choices = app.link_choices(&lp.filter.text);
            let sel = lp.sel.min(choices.len().saturating_sub(1));
            let [head, list, foot] = Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(inner);
            f.render_widget(Paragraph::new(Line::from(vec![Span::styled(" Search ", dim()), Span::raw(lp.filter.text.clone())])), head);
            let h = list.height as usize;
            let first = if sel < h { 0 } else { sel + 1 - h };
            let mut rows = vec![];
            for (n, choice) in choices.iter().enumerate().skip(first).take(h) {
                let r = Rect::new(list.x, list.y + (n - first) as u16, list.width, 1);
                let text = match choice {
                    None => Line::styled(" (no class or assignment)", dim()),
                    Some(i) => {
                        let it = &app.items()[*i];
                        Line::from(vec![
                            Span::raw(" "),
                            Span::styled(fmt::fit(&fmt::short_class(&it.class, 11), 11), Style::new().fg(Color::Blue)),
                            Span::raw(" "),
                            Span::raw(fmt::truncate(&it.name, (r.width as usize).saturating_sub(14))),
                        ])
                    }
                };
                f.render_widget(Paragraph::new(text).style(if n == sel { SELECTED } else { Style::new() }), r);
                rows.push((r, n));
            }
            app.hits.modal_rows = rows;
            let y = foot.y;
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    button("Cancel", Style::new().fg(Color::Black).bg(Color::Gray)),
                    Span::styled("   type to search · ↑↓ · Enter links · click a row", dim()),
                ])),
                Rect::new(foot.x + 1, y, foot.width.saturating_sub(1), 1),
            );
            app.hits.modal_buttons = vec![(Rect::new(foot.x + 1, y, 8, 1), ModalAction::Cancel)];
            f.set_cursor_position((head.x + 8 + lp.filter.text.width() as u16, head.y));
        }
        Modal::Login { url, opened, failed } => {
            let rect = centered(area, 76, 12);
            f.render_widget(Clear, rect);
            let block = Block::bordered().title(" Sign in ").border_style(Style::new().fg(Color::Yellow));
            let inner = block.inner(rect);
            f.render_widget(block, rect);
            let width = (inner.width as usize).saturating_sub(2);
            let mut lines = vec![];
            if let Some(e) = failed {
                lines.push(Line::styled(" Sign-in failed:", Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)));
                lines.extend(fmt::wrap(&e, width).into_iter().map(|l| Line::styled(format!(" {l}"), Style::new().fg(Color::Red))));
                lines.push(Line::default());
                lines.push(Line::styled(" Press r to try again. Esc closes this.", dim()));
            } else {
                lines.push(Line::styled(" Your session expired. Sign in with Google in your browser.", bold()));
                lines.push(Line::styled(
                    if opened {
                        " A browser window was opened."
                    } else if url.is_some() {
                        " Could not open a browser; open this address yourself:"
                    } else {
                        " Opening the sign-in page…"
                    },
                    dim(),
                ));
                if let Some(u) = url {
                    lines.push(Line::default());
                    lines.extend(fmt::wrap(&u, width).into_iter().map(|l| Line::styled(format!(" {l}"), Style::new().fg(Color::Blue))));
                }
                lines.push(Line::default());
                lines.push(Line::styled(" Needs capture.user.js in your userscript manager. Waiting…", dim()));
            }
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}
