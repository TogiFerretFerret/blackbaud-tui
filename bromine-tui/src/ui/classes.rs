//! The Classes view: the class list, and per class its About, Work, Topics, News, Files, Grades and People.

use bromine::Class;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::assignments::append_detail;
use super::widgets::*;
use crate::app::{App, ClassTab, Focus, GradeEntry};
use crate::data::Load;
use crate::fmt;

/// What a class's sub-list shows: lines to select from, or one message in their place.
enum SubList {
    Message(Line<'static>),
    Rows(Vec<Line<'static>>),
}

fn list_of<T>(l: Load<'_, Vec<T>>, empty: &str, row: impl Fn(&T) -> Line<'static>) -> SubList {
    match l {
        Load::Loading => SubList::Message(Line::styled("Loading…", dim())),
        Load::Failed(e) => SubList::Message(Line::styled(e.to_owned(), Style::new().fg(Color::Red))),
        Load::Ready(v) if v.is_empty() => SubList::Message(Line::styled(empty.to_owned(), dim())),
        Load::Ready(v) => SubList::Rows(v.iter().map(row).collect()),
    }
}

fn dated(when: Option<chrono::NaiveDateTime>, now: chrono::NaiveDateTime, text: &str, width: usize, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(fmt::fit(&when_label(when, now), 11), dim()),
        Span::raw(" "),
        Span::styled(fmt::truncate(text, width.saturating_sub(12)), style),
    ])
}

pub fn draw_classes(app: &mut App, f: &mut Frame, area: Rect) {
    let [left, right] = Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).areas(area);
    let block = pane(Line::from(format!(" Classes ({}) ", app.classes().len())), app.focus == Focus::Classes);
    let inner = block.inner(left);
    f.render_widget(block, left);
    app.hits.list = inner;
    if app.classes().is_empty() {
        f.render_widget(Paragraph::new("Loading…").style(dim()), inner);
    } else {
        let lines: Vec<Line> = app
            .classes()
            .iter()
            .map(|c| {
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(fmt::fit(&fmt::short_class(&c.name, 99), (inner.width as usize).saturating_sub(2)), bold()),
                ])
            })
            .collect();
        app.hits.rows = render_rows(f, inner, lines, app.class_offset, Some(app.class_sel));
    }

    let class = app.selected_class().cloned();
    let title = match &class {
        Some(c) => format!(" {}{} ", c.name, c.teacher.as_ref().map(|t| format!(" · {t}")).unwrap_or_default()),
        None => " Class ".into(),
    };
    let block = pane(Line::from(title), app.focus == Focus::Sub);
    let inner = block.inner(right);
    f.render_widget(block, right);
    let Some(class) = class else { return };
    let [bar, content] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
    draw_class_tabs(app, f, bar);

    if app.class_tab == ClassTab::About {
        return draw_about(app, f, content, class.section_id);
    }
    let [list, preview] = Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(content);
    app.hits.sub_list = list;
    draw_sub_list(app, f, list, &class);
    let divider = Block::new().borders(Borders::TOP).border_style(dim());
    let preview_inner = divider.inner(preview);
    f.render_widget(divider, preview);
    let lines = preview_lines(app, &class, preview_inner.width.saturating_sub(1) as usize);
    let scroll = app.class_scroll;
    app.class_scroll = draw_body(app, f, preview_inner, preview_inner, lines, scroll);
}

fn draw_class_tabs(app: &mut App, f: &mut Frame, area: Rect) {
    let mut spans = vec![];
    let mut x = area.x;
    for tab in ClassTab::ALL {
        let label = format!(" {} ", tab.title());
        let w = label.chars().count() as u16;
        if x + w > area.right() {
            break;
        }
        app.hits.sub_tabs.push((Rect::new(x, area.y, w, 1), tab));
        spans.push(if tab == app.class_tab {
            Span::styled(label, Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD))
        } else {
            Span::styled(label, dim())
        });
        x += w;
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_about(app: &mut App, f: &mut Frame, area: Rect, section_id: i64) {
    let width = area.width.saturating_sub(1) as usize;
    let mut lines: Vec<Line> = vec![];
    match app.store.class_info.get(&section_id) {
        Some(e) => {
            let info = &e.value;
            lines.push(Line::styled(info.name.clone(), bold()));
            lines.extend(info.teacher.clone().map(|t| kv("Teacher", t)));
            lines.extend(info.room.clone().map(|r| kv("Room", r)));
            lines.extend(info.school_year.clone().map(|y| kv("Year", y)));
            let terms: Vec<String> = info.terms.iter().filter_map(|t| t.name.clone()).collect();
            if !terms.is_empty() {
                lines.push(kv("Terms", terms.join(", ")));
            }
            if let Some(d) = &info.description {
                lines.push(Line::default());
                lines.extend(fmt::wrap(d, width).into_iter().map(Line::from));
            }
        }
        None => lines.push(Line::styled("Loading…", dim())),
    }
    let scroll = app.class_scroll;
    app.class_scroll = draw_body(app, f, area, area, lines, scroll);
}

fn sub_list(app: &App, class: &Class, w: usize) -> SubList {
    let (section, lead, now) = (class.section_id, class.lead_section_id, app.now);
    match app.class_tab {
        ClassTab::About => SubList::Rows(vec![]),
        ClassTab::Work => list_of(app.store.class_rows(lead), "No current assignments.", |r| {
            let (g, c) = status_glyph(r.status);
            Line::from(vec![
                Span::styled(g, Style::new().fg(c)),
                Span::raw(" "),
                Span::styled(fmt::fit(&fmt::due_label(r.due, now), 11), dim()),
                Span::raw(" "),
                Span::raw(fmt::truncate(&r.name, w.saturating_sub(14))),
            ])
        }),
        ClassTab::Topics => list_of(app.store.topics(section), "No topics.", |t| dated(t.published, now, &t.name, w, Style::new())),
        ClassTab::News => list_of(app.store.news(section), "No announcements.", |n| dated(n.published, now, &n.title, w, Style::new())),
        ClassTab::Files => {
            list_of(app.store.class_files(section), "No files.", |x| dated(x.published, now, &x.name, w, Style::new().fg(Color::Blue)))
        }
        ClassTab::People => list_of(app.store.people(section), "Nobody listed.", |p| {
            Line::from(vec![
                Span::styled(fmt::fit(p.role.as_deref().unwrap_or(""), 9), if p.email.is_some() { Style::new().fg(ACCENT) } else { dim() }),
                Span::raw(" "),
                Span::raw(fmt::truncate(&p.name, w.saturating_sub(10))),
            ])
        }),
        ClassTab::Grades => match app.store.grades(section) {
            Load::Ready(g) if !g.is_empty() => SubList::Rows(
                (0..app.sub_len())
                    .filter_map(|i| app.grade_entry(i))
                    .map(|e| match e {
                        GradeEntry::Period(p) => {
                            let total = p.unweighted_total().map(|(e, m)| format!("  {e}/{m} · {:.0}%", e / m * 100.0)).unwrap_or_default();
                            Line::from(vec![
                                Span::styled(p.period.clone(), bold().add_modifier(Modifier::UNDERLINED)),
                                Span::styled(total, dim()),
                            ])
                        }
                        GradeEntry::Row(r) => {
                            let score = match (r.points, r.max_points) {
                                (Some(p), Some(m)) => format!("{p}/{m}"),
                                (None, Some(m)) => format!("–/{m}"),
                                _ => String::new(),
                            };
                            Line::from(vec![
                                Span::raw("  "),
                                Span::raw(fmt::fit(&r.name, w.saturating_sub(12))),
                                Span::styled(format!("{score:>9}"), if r.points.is_some() { Style::new().fg(Color::Cyan) } else { dim() }),
                            ])
                        }
                    })
                    .collect(),
            ),
            other => list_of(other, "No grades yet.", |_: &bromine::Grades| Line::default()),
        },
    }
}

fn draw_sub_list(app: &mut App, f: &mut Frame, area: Rect, class: &Class) {
    match sub_list(app, class, area.width as usize) {
        SubList::Message(m) => f.render_widget(Paragraph::new(m).wrap(Wrap { trim: true }), area),
        SubList::Rows(lines) => app.hits.sub_rows = render_rows(f, area, lines, app.sub_offset, Some(app.sub_sel)),
    }
}

fn preview_lines(app: &App, class: &Class, width: usize) -> Vec<Line<'static>> {
    let wrap = |t: &str| fmt::wrap(t, width).into_iter().map(Line::from).collect::<Vec<_>>();
    let (section, i) = (class.section_id, app.sub_sel);
    let title = |out: &mut Vec<Line<'static>>, t: &str| out.extend(fmt::wrap(t, width).into_iter().map(|l| Line::styled(l, bold())));
    let mut out: Vec<Line<'static>> = vec![];
    match app.class_tab {
        ClassTab::About => {}
        ClassTab::Work => {
            let Some(r) = app.selected_work() else { return out };
            title(&mut out, &r.name);
            let mut meta = vec![];
            meta.extend(r.assignment_type.clone().map(|t| Span::styled(t, dim())));
            meta.extend(r.due.map(|d| Span::raw(format!(" · due {}", fmt::long_when(d)))));
            out.push(Line::from(meta));
            out.push(Line::styled("Enter shows it in Assignments", Style::new().fg(ACCENT)));
            out.push(Line::default());
            match app.store.detail(r.id) {
                Load::Ready(d) => {
                    let mut note = None;
                    append_detail(d, &mut out, &mut note, &wrap);
                    if let Some(n) = note {
                        out.insert(4.min(out.len()), Line::styled(n, Style::new().fg(Color::Yellow)));
                    }
                }
                Load::Failed(e) => out.push(Line::styled(format!("Could not load details: {e}"), Style::new().fg(Color::Red))),
                Load::Loading => out.push(Line::styled("Loading details…", dim())),
            }
        }
        ClassTab::Topics => {
            let Some(t) = app.selected_topic() else { return out };
            title(&mut out, &t.name);
            out.extend(t.published.map(|p| Line::styled(format!("Published {}", fmt::long_when(p)), dim())));
            match app.store.topic_items(t) {
                Load::Ready(items) if items.is_empty() => out.push(Line::styled("Nothing in this topic.", dim())),
                Load::Ready(items) => {
                    for item in items {
                        out.push(Line::default());
                        if let Some(t) = &item.title {
                            title(&mut out, t);
                        }
                        if let Some(file) = item.file.as_ref().filter(|_| item.path.is_some()) {
                            out.push(Line::styled(format!("file: {file}"), Style::new().fg(Color::Blue)));
                        }
                        out.extend(
                            item.url.clone().map(|u| Line::styled(u, Style::new().fg(Color::Blue).add_modifier(Modifier::UNDERLINED))),
                        );
                        if let Some(text) = &item.text {
                            out.extend(wrap(text));
                        }
                    }
                }
                Load::Failed(e) => out.push(Line::styled(e.to_owned(), Style::new().fg(Color::Red))),
                Load::Loading => out.push(Line::styled("Loading…", dim())),
            }
        }
        ClassTab::News => {
            let Some(n) = app.store.news(section).ready().and_then(|v| v.get(i)) else { return out };
            title(&mut out, &n.title);
            let mut meta = vec![];
            meta.extend(n.author.clone().map(|a| Span::styled(a, Style::new().fg(Color::Blue))));
            meta.extend(n.published.map(|p| Span::styled(format!(" · {}", fmt::long_when(p)), dim())));
            out.push(Line::from(meta));
            if let Some(t) = &n.text {
                out.push(Line::default());
                out.extend(wrap(t));
            }
        }
        ClassTab::Files => {
            let Some(x) = app.selected_class_file() else { return out };
            title(&mut out, &x.name);
            out.extend(x.published.map(|p| Line::styled(format!("Published {}", fmt::long_when(p)), dim())));
        }
        ClassTab::People => {
            let Some(p) = app.store.people(section).ready().and_then(|v| v.get(i)) else { return out };
            out.push(Line::styled(p.name.clone(), bold()));
            out.extend(p.role.clone().map(|r| kv("Role", r)));
            out.extend(p.grad_year.clone().map(|y| kv("Class of", y)));
            out.extend(p.email.clone().map(|e| kv("Email", e)));
        }
        ClassTab::Grades => match app.grade_entry(i) {
            Some(GradeEntry::Period(p)) => {
                out.push(Line::styled(p.period.clone(), bold()));
                match p.unweighted_total() {
                    Some((e, m)) => {
                        out.push(kv("Points", format!("{e} / {m}  ({:.1}%)", e / m * 100.0)));
                        out.push(Line::default());
                        out.extend(wrap(
                            "Unweighted: the school hides the weighting, so this is plain points earned over points possible.",
                        ));
                    }
                    None => out.push(Line::styled("Nothing scored yet.", dim())),
                }
            }
            Some(GradeEntry::Row(r)) => {
                title(&mut out, &r.name);
                out.extend(r.assignment_type.clone().map(|t| Line::styled(t, dim())));
                match (r.points, r.max_points) {
                    (Some(p), Some(m)) => out.push(kv("Score", format!("{p} / {m}  ({:.0}%)", p / m * 100.0))),
                    (None, Some(m)) => out.push(kv("Score", format!("not graded / {m}"))),
                    _ => {}
                }
                out.extend(r.due.map(|d| kv("Due", fmt::long_when(d))));
                if !r.flags.is_empty() {
                    out.push(Line::styled(r.flags.describe(), Style::new().fg(Color::Red)));
                }
                if let Some(c) = &r.comment {
                    out.push(Line::default());
                    out.extend(wrap(c));
                }
            }
            None => {}
        },
    }
    out
}
