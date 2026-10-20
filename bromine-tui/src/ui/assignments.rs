//! The Assignments view.

use bromine::{AssignmentDetail, CenterItem, Status};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::widgets::*;
use crate::app::{App, Row};
use crate::data::Load;
use crate::fmt;

// ---------------------------------------------------------------------------- assignments

pub fn assignment_line(item: &CenterItem, app: &App, width: usize) -> Line<'static> {
    let (glyph, color) = status_glyph(item.status);
    let done = item.status.is_done();
    let overdue = !done && item.due.is_some_and(|d| d < app.now);
    let base = if done { dim() } else { Style::new() };
    let due_style = if overdue { Style::new().fg(Color::Red) } else { base };
    let class = if item.is_task() { "task".to_owned() } else { fmt::short_class(&item.class, 11) };
    let class_style = if item.is_task() { Style::new().fg(Color::Magenta) } else { Style::new().fg(Color::Blue) };
    let flags = if item.flags.is_empty() { String::new() } else { format!(" {}", item.flags.describe()) };
    let used = 2 + 13 + 12 + flags.width();
    let name = fmt::truncate(&item.name, width.saturating_sub(used));
    let name_style = if item.major { base.add_modifier(Modifier::BOLD) } else { base };
    let mut spans = vec![
        Span::styled(glyph, Style::new().fg(color)),
        Span::raw(" "),
        Span::styled(fmt::fit(&fmt::due_label(item.due, app.now), 12), due_style),
        Span::raw(" "),
        Span::styled(fmt::fit(&class, 11), if done { dim() } else { class_style }),
        Span::raw(" "),
    ];
    if item.major {
        spans.push(Span::styled("‼ ", Style::new().fg(Color::Red)));
    }
    spans.push(Span::styled(name, name_style));
    if !flags.is_empty() {
        spans.push(Span::styled(flags, Style::new().fg(Color::Red)));
    }
    Line::from(spans)
}

pub fn draw_assignments(app: &mut App, f: &mut Frame, area: Rect) {
    let [left, right] = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area);

    let mut title = vec![Span::raw(" Assignments ")];
    if app.show_past {
        title.push(Span::styled("+past ", Style::new().fg(Color::Yellow)));
    }
    if app.hide_done {
        title.push(Span::styled("-done ", Style::new().fg(Color::Yellow)));
    }
    if !app.filter.is_empty() {
        title.push(Span::styled(format!("/{} ", app.filter), Style::new().fg(Color::Yellow)));
    }
    let block = pane(Line::from(title), true);
    let inner = block.inner(left);
    f.render_widget(block, left);
    app.hits.list = inner;
    app.keep_visible(); // the height is only known now (first draw, resizes)

    if app.rows.is_empty() {
        let msg = if app.store.center.is_none() {
            "Loading…"
        } else if app.filter.is_empty() {
            "Nothing to show. (p shows past work, h shows completed)"
        } else {
            "No matches. Esc clears the filter."
        };
        f.render_widget(Paragraph::new(msg).style(dim()), inner);
    } else {
        let lines: Vec<Line> = app
            .rows
            .iter()
            .map(|r| match r {
                Row::Header(b) => {
                    let n = app.rows.iter().filter(|r| matches!(r, Row::Item(i) if app.items()[*i].bucket == *b)).count();
                    Line::from(vec![
                        Span::styled(format!(" {} ", b.label()), bold().add_modifier(Modifier::UNDERLINED)),
                        Span::styled(format!("({n})"), dim()),
                    ])
                }
                Row::Item(i) => assignment_line(&app.items()[*i], app, inner.width as usize),
            })
            .collect();
        let rects = render_rows(f, inner, lines, app.offset, Some(app.sel));
        // header rows are not clickable targets
        app.hits.rows = rects;
    }
    draw_assignment_detail(app, f, right);
}

pub struct Detail {
    title: String,
    meta: Vec<Line<'static>>,
    /// The status to highlight, when the buttons apply.
    status: Option<Status>,
    note: Option<String>,
    body: Vec<Line<'static>>,
}

pub fn build_detail(app: &App, item: &CenterItem, width: usize) -> Detail {
    let mut meta = Vec::new();
    let mut kind = vec![Span::styled(
        match (item.is_task(), item.section_id) {
            (false, _) => item.class.clone(),
            (true, 0) => "Personal task".to_owned(),
            (true, _) => format!("Personal task · {}", fmt::short_class(&item.class, 40)),
        },
        Style::new().fg(if item.is_task() { Color::Magenta } else { Color::Blue }),
    )];
    if let Some(t) = &item.assignment_type
        && !item.is_task()
    {
        kind.push(Span::styled(format!(" · {t}"), dim()));
    }
    if let Some(p) = item.points {
        kind.push(Span::styled(format!(" · {p} pts"), dim()));
    }
    meta.push(Line::from(kind));
    if let Some(due) = item.due {
        let late = !item.status.is_done() && due < app.now;
        meta.push(Line::from(vec![
            Span::raw("Due "),
            Span::styled(fmt::long_when(due), if late { Style::new().fg(Color::Red) } else { Style::new() }),
            Span::styled(if late { "  (past due)" } else { "" }, Style::new().fg(Color::Red)),
        ]));
    }

    let mut body: Vec<Line> = Vec::new();
    let mut note = None;
    let wrap = |text: &str| fmt::wrap(text, width).into_iter().map(Line::from).collect::<Vec<_>>();

    if item.is_task() {
        body.push(Line::styled("Your own to-do. e edits it, l links it to a class, x deletes it.", dim()));
    } else {
        match app.store.detail(item.id) {
            Load::Ready(d) => append_detail(d, &mut body, &mut note, &wrap),
            Load::Failed(e) => body.push(Line::styled(format!("Could not load details: {e}"), Style::new().fg(Color::Red))),
            Load::Loading => body.push(Line::styled("Loading details…", dim())),
        }
    }
    Detail { title: item.name.clone(), meta, status: Some(item.status), note, body }
}

pub fn append_detail(
    d: &AssignmentDetail,
    body: &mut Vec<Line<'static>>,
    note: &mut Option<String>,
    wrap: &dyn Fn(&str) -> Vec<Line<'static>>,
) {
    if d.dropbox && d.submissions.is_empty() {
        *note = Some("File submission: it is marked completed when you submit it in the LMS.".into());
    }
    if let Some(text) = &d.description {
        body.extend(wrap(text));
    } else {
        body.push(Line::styled("(no description)", dim()));
    }
    if let (Some(g), Some(max)) = (d.grade, d.max_points) {
        body.push(Line::default());
        body.push(Line::from(vec![
            Span::styled("Grade ", bold()),
            Span::styled(format!("{g} / {max}"), Style::new().fg(Color::Cyan)),
            Span::styled(d.percent.map(|p| format!("  ({p:.0}%)")).unwrap_or_default(), dim()),
        ]));
        if let Some(c) = &d.comment {
            body.extend(wrap(c));
        }
    }
    for s in &d.submissions {
        body.push(Line::default());
        let when = s.submitted.or(s.saved).map(fmt::long_when).unwrap_or_default();
        body.push(Line::from(vec![
            Span::styled("Your submission ", bold()),
            Span::styled(when, dim()),
            Span::styled(if s.ready { "" } else { "  (draft)" }, Style::new().fg(Color::Yellow)),
        ]));
        if let Some(t) = &s.text {
            body.extend(wrap(t));
        }
    }
}

pub fn draw_assignment_detail(app: &mut App, f: &mut Frame, area: Rect) {
    let block = pane(Line::from(" Details "), false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(item) = app.selected_item().cloned() else {
        f.render_widget(Paragraph::new("Select an assignment.").style(dim()), inner);
        return;
    };
    let width = inner.width.saturating_sub(1) as usize;
    let detail = build_detail(app, &item, width);

    let title = fmt::wrap(&detail.title, width);
    let title_h = title.len().clamp(1, 3) as u16;
    let note_h = detail.note.as_ref().map_or(0, |n| fmt::wrap(n, width).len() as u16);
    let [t_area, meta_area, btn_area, note_area, body_area] = Layout::vertical([
        Constraint::Length(title_h),
        Constraint::Length(detail.meta.len() as u16),
        Constraint::Length(2),
        Constraint::Length(note_h),
        Constraint::Min(1),
    ])
    .areas(inner);

    f.render_widget(Paragraph::new(title.into_iter().take(3).map(|l| Line::styled(l, bold())).collect::<Vec<_>>()), t_area);
    f.render_widget(Paragraph::new(detail.meta), meta_area);
    draw_status_buttons(app, f, btn_area, detail.status.unwrap_or(Status::Todo));
    if let Some(n) = &detail.note {
        f.render_widget(
            Paragraph::new(fmt::wrap(n, width).into_iter().map(|l| Line::styled(l, Style::new().fg(Color::Yellow))).collect::<Vec<_>>()),
            note_area,
        );
    }

    let scroll = app.detail_scroll;
    app.detail_scroll = draw_body(app, f, body_area, inner, detail.body, scroll);
}

pub fn draw_status_buttons(app: &mut App, f: &mut Frame, area: Rect, current: Status) {
    let mut spans = vec![Span::raw("Status ")];
    let mut x = area.x + 7;
    for s in Status::SETTABLE {
        let label = match s {
            Status::Todo => " To do ",
            Status::InProgress => " In progress ",
            _ => " Done ",
        };
        let w = label.width() as u16;
        if x + w > area.right() {
            break;
        }
        app.hits.buttons.push((Rect::new(x, area.y, w, 1), s));
        let (_, color) = status_glyph(s);
        spans.push(if s == current {
            Span::styled(label, Style::new().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD))
        } else {
            Span::styled(label, Style::new().fg(color).add_modifier(Modifier::UNDERLINED))
        });
        spans.push(Span::raw(" "));
        x += w + 1;
    }
    if !current.is_settable() {
        spans.push(Span::styled(format!("({})", current.label()), dim()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
