//! Drawing. Everything clickable is recorded in `app.hits` as it is drawn, so the mouse handler always
//! agrees with what is on screen.

use bromine::{AssignmentDetail, Bucket, CenterItem, Status};
use chrono::NaiveDateTime;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, ClassTab, Focus, GradeEntry, Hits, Load, Modal, ModalAction, Row, Tab};
use crate::fmt;

const ACCENT: Color = Color::Cyan;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SELECTED: Style = Style::new().bg(Color::Indexed(238)).add_modifier(Modifier::BOLD);

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

fn status_glyph(s: Status) -> (&'static str, Color) {
    match s {
        Status::Todo => ("○", Color::White),
        Status::InProgress => ("◐", Color::Yellow),
        Status::Completed => ("●", Color::Green),
        Status::Graded => ("✓", Color::Cyan),
        Status::Overdue => ("!", Color::Red),
        Status::Other(_) => ("?", Color::DarkGray),
    }
}

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

    let open = app.items.iter().filter(|i| i.bucket.is_upcoming() && !i.status.is_done()).count();
    let today = app.items.iter().filter(|i| i.bucket == Bucket::DueToday && !i.status.is_done()).count();
    let mut right = String::new();
    if app.pending > 0 {
        right.push_str(&format!("{} loading  ", SPINNER[app.spinner % SPINNER.len()]));
    }
    if app.center_loaded {
        right.push_str(&format!("{open} open · {today} due today "));
    }
    // only when it fits: on a narrow terminal the tabs matter more
    if (x.saturating_sub(area.x) as usize) + right.width() < area.width as usize {
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right).style(dim()), area);
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

// ---------------------------------------------------------------------------- list helper

/// Render `lines` as clickable rows into `inner`, starting at `offset`. Returns the (rect, row index) pairs.
fn render_rows(f: &mut Frame, inner: Rect, lines: Vec<Line<'static>>, offset: usize, selected: Option<usize>) -> Vec<(Rect, usize)> {
    let mut rects = Vec::new();
    for (n, line) in lines.into_iter().enumerate().skip(offset).take(inner.height as usize) {
        let rect = Rect::new(inner.x, inner.y + (n - offset) as u16, inner.width, 1);
        let style = if selected == Some(n) { SELECTED } else { Style::new() };
        f.render_widget(Paragraph::new(line).style(style), rect);
        rects.push((rect, n));
    }
    rects
}

fn pane(title: Line<'static>, focused: bool) -> Block<'static> {
    Block::bordered().title(title).border_style(if focused { Style::new().fg(ACCENT) } else { dim() })
}

// ---------------------------------------------------------------------------- assignments

fn assignment_line(item: &CenterItem, app: &App, width: usize) -> Line<'static> {
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

fn draw_assignments(app: &mut App, f: &mut Frame, area: Rect) {
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
        let msg = if !app.center_loaded {
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
                    let n = app.rows.iter().filter(|r| matches!(r, Row::Item(i) if app.items[*i].bucket == *b)).count();
                    Line::from(vec![
                        Span::styled(format!(" {} ", b.label()), bold().add_modifier(Modifier::UNDERLINED)),
                        Span::styled(format!("({n})"), dim()),
                    ])
                }
                Row::Item(i) => assignment_line(&app.items[*i], app, inner.width as usize),
            })
            .collect();
        let rects = render_rows(f, inner, lines, app.offset, Some(app.sel));
        // header rows are not clickable targets
        app.hits.rows = rects;
    }
    draw_assignment_detail(app, f, right);
}

struct Detail {
    title: String,
    meta: Vec<Line<'static>>,
    /// The status to highlight, when the buttons apply.
    status: Option<Status>,
    note: Option<String>,
    body: Vec<Line<'static>>,
}

fn build_detail(app: &App, item: &CenterItem, width: usize) -> Detail {
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
    } else if let Some(d) = app.details.get(&item.id) {
        append_detail(d, &mut body, &mut note, &wrap);
    } else if let Some(e) = app.detail_failed.get(&item.id) {
        body.push(Line::styled(format!("Could not load details: {e}"), Style::new().fg(Color::Red)));
    } else {
        body.push(Line::styled("Loading details…", dim()));
    }
    Detail { title: item.name.clone(), meta, status: Some(item.status), note, body }
}

fn append_detail(
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

fn draw_assignment_detail(app: &mut App, f: &mut Frame, area: Rect) {
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

/// Scrollable text in `area`, with the selection's files (if any) pinned underneath as clickable lines.
/// `hit` is where the wheel scrolls it. Returns the scroll position, clamped.
fn draw_body(app: &mut App, f: &mut Frame, area: Rect, hit: Rect, lines: Vec<Line<'static>>, scroll: usize) -> usize {
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

fn draw_status_buttons(app: &mut App, f: &mut Frame, area: Rect, current: Status) {
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

// ---------------------------------------------------------------------------- classes

fn kv(label: &str, value: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{label} "), dim()), Span::raw(value)])
}

fn when_label(d: Option<NaiveDateTime>, now: NaiveDateTime) -> String {
    d.map(|d| fmt::due_label(Some(d), now)).unwrap_or_default()
}

/// One status line for a list that is not ready yet, or `None` when it is (and may still be empty).
fn not_ready<T>(l: Option<&Load<Vec<T>>>) -> Option<Line<'static>> {
    match l {
        None | Some(Load::Loading) => Some(Line::styled("Loading…", dim())),
        Some(Load::Failed(e)) => Some(Line::styled(e.clone(), Style::new().fg(Color::Red))),
        Some(Load::Ready(_)) => None,
    }
}

fn draw_classes(app: &mut App, f: &mut Frame, area: Rect) {
    let [left, right] = Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).areas(area);
    let block = pane(Line::from(format!(" Classes ({}) ", app.classes.len())), app.focus == Focus::Classes);
    let inner = block.inner(left);
    f.render_widget(block, left);
    app.hits.list = inner;
    if app.classes.is_empty() {
        f.render_widget(Paragraph::new("Loading…").style(dim()), inner);
    } else {
        let lines: Vec<Line> = app
            .classes
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
    let divider = Block::new().borders(ratatui::widgets::Borders::TOP).border_style(dim());
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
        let w = label.width() as u16;
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
    if let Some(info) = app.class_info.get(&section_id) {
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
    } else {
        lines.push(Line::styled("Loading…", dim()));
    }
    let scroll = app.class_scroll;
    app.class_scroll = draw_body(app, f, area, area, lines, scroll);
}

fn draw_sub_list(app: &mut App, f: &mut Frame, area: Rect, class: &bromine::Class) {
    let (section, lead) = (class.section_id, class.lead_section_id);
    let w = area.width as usize;
    let now = app.now;
    let lines: Vec<Line<'static>> = match app.class_tab {
        ClassTab::About => return,
        ClassTab::Work => match app.class_rows.get(&lead) {
            None => return note(f, area, "Loading…"),
            Some(rows) if rows.is_empty() => return note(f, area, "No current assignments."),
            Some(rows) => rows
                .iter()
                .map(|r| {
                    let (g, c) = status_glyph(r.status);
                    Line::from(vec![
                        Span::styled(g, Style::new().fg(c)),
                        Span::raw(" "),
                        Span::styled(fmt::fit(&fmt::due_label(r.due, now), 11), dim()),
                        Span::raw(" "),
                        Span::raw(fmt::truncate(&r.name, w.saturating_sub(14))),
                    ])
                })
                .collect(),
        },
        ClassTab::Topics => match (not_ready(app.topics.get(&section)), app.topics.get(&section)) {
            (Some(l), _) => return f.render_widget(Paragraph::new(l), area),
            (None, Some(Load::Ready(t))) if t.is_empty() => return note(f, area, "No topics."),
            (None, Some(Load::Ready(t))) => t
                .iter()
                .map(|t| {
                    Line::from(vec![
                        Span::styled(fmt::fit(&when_label(t.published, now), 11), dim()),
                        Span::raw(" "),
                        Span::raw(fmt::truncate(&t.name, w.saturating_sub(12))),
                    ])
                })
                .collect(),
            _ => return,
        },
        ClassTab::News => match (not_ready(app.news.get(&section)), app.news.get(&section)) {
            (Some(l), _) => return f.render_widget(Paragraph::new(l), area),
            (None, Some(Load::Ready(n))) if n.is_empty() => return note(f, area, "No announcements."),
            (None, Some(Load::Ready(n))) => n
                .iter()
                .map(|n| {
                    Line::from(vec![
                        Span::styled(fmt::fit(&when_label(n.published, now), 11), dim()),
                        Span::raw(" "),
                        Span::raw(fmt::truncate(&n.title, w.saturating_sub(12))),
                    ])
                })
                .collect(),
            _ => return,
        },
        ClassTab::Files => match (not_ready(app.class_files.get(&section)), app.class_files.get(&section)) {
            (Some(l), _) => return f.render_widget(Paragraph::new(l), area),
            (None, Some(Load::Ready(v))) if v.is_empty() => return note(f, area, "No files."),
            (None, Some(Load::Ready(v))) => v
                .iter()
                .map(|x| {
                    Line::from(vec![
                        Span::styled(fmt::fit(&when_label(x.published, now), 11), dim()),
                        Span::raw(" "),
                        Span::styled(fmt::truncate(&x.name, w.saturating_sub(12)), Style::new().fg(Color::Blue)),
                    ])
                })
                .collect(),
            _ => return,
        },
        ClassTab::People => match (not_ready(app.people.get(&section)), app.people.get(&section)) {
            (Some(l), _) => return f.render_widget(Paragraph::new(l), area),
            (None, Some(Load::Ready(v))) if v.is_empty() => return note(f, area, "Nobody listed."),
            (None, Some(Load::Ready(v))) => v
                .iter()
                .map(|p| {
                    let teacher = p.email.is_some();
                    Line::from(vec![
                        Span::styled(fmt::fit(p.role.as_deref().unwrap_or(""), 9), if teacher { Style::new().fg(ACCENT) } else { dim() }),
                        Span::raw(" "),
                        Span::raw(fmt::truncate(&p.name, w.saturating_sub(10))),
                    ])
                })
                .collect(),
            _ => return,
        },
        ClassTab::Grades => match (not_ready(app.grades.get(&section)), app.grades.get(&section)) {
            (Some(l), _) => return f.render_widget(Paragraph::new(wrap_line(l, w)), area),
            (None, Some(Load::Ready(g))) if g.is_empty() => return note(f, area, "No grades yet."),
            _ => (0..app.sub_len())
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
        },
    };
    app.hits.sub_rows = render_rows(f, area, lines, app.sub_offset, Some(app.sub_sel));
}

fn note(f: &mut Frame, area: Rect, text: &str) {
    f.render_widget(Paragraph::new(text.to_owned()).style(dim()), area);
}

/// A message as wrapped lines.
fn wrap_line(l: Line<'static>, w: usize) -> Vec<Line<'static>> {
    let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
    fmt::wrap(&text, w).into_iter().map(|t| Line::styled(t, l.style.patch(l.spans.first().map(|s| s.style).unwrap_or_default()))).collect()
}

fn preview_lines(app: &App, class: &bromine::Class, width: usize) -> Vec<Line<'static>> {
    let wrap = |t: &str| fmt::wrap(t, width).into_iter().map(Line::from).collect::<Vec<_>>();
    let now = app.now;
    let (section, i) = (class.section_id, app.sub_sel);
    let mut out: Vec<Line<'static>> = vec![];
    match app.class_tab {
        ClassTab::About => {}
        ClassTab::Work => {
            let Some(r) = app.selected_work() else { return out };
            out.extend(fmt::wrap(&r.name, width).into_iter().map(|l| Line::styled(l, bold())));
            let mut meta = vec![];
            meta.extend(r.assignment_type.clone().map(|t| Span::styled(t, dim())));
            meta.extend(r.due.map(|d| Span::raw(format!(" · due {}", fmt::long_when(d)))));
            out.push(Line::from(meta));
            out.push(Line::styled("Enter shows it in Assignments", Style::new().fg(ACCENT)));
            out.push(Line::default());
            if let Some(d) = app.details.get(&r.id) {
                let mut note = None;
                append_detail(d, &mut out, &mut note, &wrap);
                if let Some(n) = note {
                    out.insert(4.min(out.len()), Line::styled(n, Style::new().fg(Color::Yellow)));
                }
            } else if let Some(e) = app.detail_failed.get(&r.id) {
                out.push(Line::styled(format!("Could not load details: {e}"), Style::new().fg(Color::Red)));
            } else {
                out.push(Line::styled("Loading details…", dim()));
            }
        }
        ClassTab::Topics => {
            let Some(t) = app.selected_topic() else { return out };
            out.extend(fmt::wrap(&t.name, width).into_iter().map(|l| Line::styled(l, bold())));
            out.extend(t.published.map(|p| Line::styled(format!("Published {}", fmt::long_when(p)), dim())));
            match app.topic_items.get(&t.topic_id) {
                Some(Load::Ready(items)) if items.is_empty() => out.push(Line::styled("Nothing in this topic.", dim())),
                Some(Load::Ready(items)) => {
                    for item in items {
                        out.push(Line::default());
                        if let Some(title) = &item.title {
                            out.extend(fmt::wrap(title, width).into_iter().map(|l| Line::styled(l, bold())));
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
                Some(Load::Failed(e)) => out.push(Line::styled(e.clone(), Style::new().fg(Color::Red))),
                _ => out.push(Line::styled("Loading…", dim())),
            }
        }
        ClassTab::News => {
            let Some(n) = ready_at(app.news.get(&section), i) else { return out };
            out.extend(fmt::wrap(&n.title, width).into_iter().map(|l| Line::styled(l, bold())));
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
            out.extend(fmt::wrap(&x.name, width).into_iter().map(|l| Line::styled(l, bold())));
            out.extend(x.published.map(|p| Line::styled(format!("Published {}", fmt::long_when(p)), dim())));
        }
        ClassTab::People => {
            let Some(p) = ready_at(app.people.get(&section), i) else { return out };
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
                out.extend(fmt::wrap(&r.name, width).into_iter().map(|l| Line::styled(l, bold())));
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
    let _ = now;
    out
}

fn ready_at<T>(l: Option<&Load<Vec<T>>>, i: usize) -> Option<&T> {
    match l {
        Some(Load::Ready(v)) => v.get(i),
        _ => None,
    }
}

// ---------------------------------------------------------------------------- feed

fn draw_feed(app: &mut App, f: &mut Frame, area: Rect) {
    let [left, right] = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let block = pane(Line::from(format!(" Feed ({}) ", app.feed.len())), true);
    let inner = block.inner(left);
    f.render_widget(block, left);
    app.hits.list = inner;
    if app.feed.is_empty() {
        f.render_widget(Paragraph::new("Loading…").style(dim()), inner);
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
                Line::from(vec![
                    Span::styled(fmt::fit(&when, 11), dim()),
                    Span::raw(" "),
                    Span::styled(if p.assignment_id.is_some() { "▸ " } else { "  " }, Style::new().fg(ACCENT)),
                    Span::raw(fmt::truncate(&title, (inner.width as usize).saturating_sub(15))),
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
        lines.push(Line::styled("Enter opens this assignment", Style::new().fg(ACCENT)));
    }
    if let Some(t) = &p.text {
        lines.push(Line::default());
        lines.extend(fmt::wrap(t, width).into_iter().map(Line::from));
    }
    let scroll = app.feed_scroll;
    app.feed_scroll = draw_body(app, f, inner, inner, lines, scroll);
}

// ---------------------------------------------------------------------------- files

fn draw_files(app: &mut App, f: &mut Frame, area: Rect) {
    let [left, right] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    let block = pane(Line::from(" School files "), true);
    let inner = block.inner(left);
    f.render_widget(block, left);
    app.hits.list = inner;
    match &app.user_files {
        None | Some(Load::Loading) => f.render_widget(Paragraph::new("Loading…").style(dim()), inner),
        Some(Load::Failed(e)) => f.render_widget(Paragraph::new(e.clone()).style(Style::new().fg(Color::Red)), inner),
        Some(Load::Ready(v)) if v.is_empty() => {
            f.render_widget(Paragraph::new("The school has not published any files to you.").style(dim()), inner)
        }
        Some(Load::Ready(v)) => {
            let lines: Vec<Line> = v
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
            app.hits.rows = render_rows(f, inner, lines, app.file_offset, Some(app.file_sel));
        }
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

// ---------------------------------------------------------------------------- modals

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

fn button(label: &str, style: Style) -> Span<'static> {
    Span::styled(format!(" {label} "), style.add_modifier(Modifier::BOLD))
}

fn draw_modal(app: &mut App, f: &mut Frame, area: Rect) {
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
                        let it = &app.items[*i];
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
