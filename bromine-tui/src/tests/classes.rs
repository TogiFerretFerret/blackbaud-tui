use super::*;

// ------------------------------------------------------------------ the class views, files, task editing

#[test]
fn every_class_view_fetches_lazily_and_renders() {
    let (mut app, rx) = loaded();
    answer_all(&mut app, &rx); // what the feed asked for: every class's announcements and topics
    press(&mut app, '2');
    assert_eq!(loads(&rx), [Key::ClassInfo(101), Key::ClassRows(101)]);

    press(&mut app, '>');
    assert_eq!(app.class_tab, ClassTab::Topics);
    let asked = loads(&rx);
    assert_eq!(asked, [Key::TopicItems { topic: 1, index: 11 }], "the topics are already here; the first one's content is fetched");
    assert!(screen(&mut app, 110, 30).contains("Loading"), "its preview says so");
    deliver(&mut app, Key::TopicItems { topic: 1, index: 11 }, Payload::TopicItems(demo::topic_items(1)));
    let s = screen(&mut app, 110, 30);
    for want in
        ["Unit 2: Reactions", "Unit 1: Foundations", "Reading guide", "https://phet.colorado.edu/", "Work through this before Friday"]
    {
        assert!(s.contains(want), "topics view missing {want:?}:\n{s}");
    }
    assert!(s.contains("▸ Reading guide.pdf"), "the topic's file is offered below:\n{s}");

    for (tab, wants) in [
        (ClassTab::News, &["Room change", "Welcome back", "Thursday we meet in room 307"][..]),
        (ClassTab::Files, &["Syllabus.pdf", "Lab safety.docx", "▸ Syllabus.pdf"][..]),
        (ClassTab::Grades, &["Quarter 1", "29/33", "Lab 1", "9/10", "Quiz: Moles"][..]),
        (ClassTab::People, &["Teacher", "Student", "Ada Student", "Grace Student"][..]),
    ] {
        while app.class_tab != tab {
            press(&mut app, '>');
        }
        answer_all(&mut app, &rx);
        let s = screen(&mut app, 110, 30);
        for want in wants {
            assert!(s.contains(want), "{tab:?} missing {want:?}:\n{s}");
        }
    }
    let before = sent(&rx);
    assert!(before.is_empty());
    press(&mut app, '<');
    press(&mut app, '>');
    assert!(sent(&rx).is_empty(), "nothing is fetched twice");
}

#[test]
fn the_grades_view_explains_an_unshared_gradebook_and_totals_are_unweighted() {
    let (mut app, rx) = loaded();
    press(&mut app, '2');
    for _ in 0..3 {
        press(&mut app, 'j'); // SPANISH III, whose gradebook is not shared
    }
    class_view(&mut app, &rx, ClassTab::Grades);
    let s = screen(&mut app, 110, 30);
    assert!(s.contains("has not shared the gradebook"), "{s}");
    press(&mut app, 'k');
    press(&mut app, 'k');
    press(&mut app, 'k');
    answer_all(&mut app, &rx);
    press(&mut app, 'l'); // into the list
    assert_eq!(app.focus, Focus::Sub);
    assert!(screen(&mut app, 110, 30).contains("Unweighted"), "the period line explains the total");
    press(&mut app, 'j');
    let s = screen(&mut app, 110, 30);
    assert!(s.contains("Score") && s.contains("9 / 10") && s.contains("(90%)"), "{s}");
}

#[test]
fn arrows_move_the_focused_column_and_the_class_resets_the_sublist() {
    let (mut app, rx) = loaded();
    class_view(&mut app, &rx, ClassTab::Topics);
    assert_eq!((app.focus, app.class_sel, app.sub_sel), (Focus::Classes, 0, 0));
    press(&mut app, 'j');
    assert_eq!(app.class_sel, 1, "down moves the class list while it has focus");
    press(&mut app, 'k');
    app.on_key(key(KeyCode::Right));
    assert_eq!(app.focus, Focus::Sub);
    sent(&rx);
    press(&mut app, 'j');
    assert_eq!((app.class_sel, app.sub_sel), (0, 1));
    let asked = loads(&rx);
    assert_eq!(asked, [Key::TopicItems { topic: 2, index: 12 }], "selecting a topic fetches it");
    press(&mut app, 'j');
    assert_eq!(app.sub_sel, 1, "the end stops");
    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.focus, Focus::Classes);
    press(&mut app, 'j');
    assert_eq!((app.class_sel, app.sub_sel), (1, 0), "another class starts at the top of its list");
}

#[test]
fn a_work_row_previews_its_details_and_enter_opens_it_in_assignments() {
    let (mut app, rx) = loaded();
    class_view(&mut app, &rx, ClassTab::Work);
    answer_all(&mut app, &rx);
    let s = screen(&mut app, 110, 30);
    assert!(s.contains("Post-Lab: Distillation") && s.contains("Pre-Lab: Heat of Fusion"), "{s}");
    assert!(s.contains("Show your reasoning") && s.contains("▸ Handout.pdf"), "the preview shows the details and the file:\n{s}");
    press(&mut app, 'l');
    press(&mut app, 'j');
    app.on_key(key(KeyCode::Enter));
    assert_eq!((app.tab, selected_id(&app)), (Tab::Assignments, 6));
}

#[test]
fn files_open_save_and_print_after_confirming() {
    let (mut app, rx) = loaded();
    press(&mut app, '1');
    sent(&rx);
    app.on_key(key(KeyCode::Down)); // header skipped: first item
    answer_all(&mut app, &rx);
    assert!(screen(&mut app, 110, 30).contains("▸ Handout.pdf"));
    press(&mut app, 'o');
    press(&mut app, 'd');
    let asked = sent(&rx);
    assert!(
        matches!(asked.as_slice(), [Cmd::Fetch { action: FileAction::Open, name, path }, Cmd::Fetch { action: FileAction::Save, .. }]
            if name == "Handout.pdf" && path == "/ftpimages/602/download/download_1.pdf"),
        "{asked:?}"
    );
    press(&mut app, 'P');
    assert!(matches!(app.modal, Some(Modal::ConfirmPrint { .. })));
    assert!(screen(&mut app, 100, 30).contains("Double-sided"));
    press(&mut app, 'n');
    assert!(app.modal.is_none() && sent(&rx).is_empty(), "declining prints nothing");
    press(&mut app, 'P');
    press(&mut app, 'y');
    assert!(matches!(sent(&rx).as_slice(), [Cmd::Fetch { action: FileAction::Print, .. }]));

    // a click on the file line opens it
    screen(&mut app, 110, 30);
    let (rect, _) = app.hits.attachments[0];
    click(&mut app, rect.x + 3, rect.y);
    assert!(matches!(sent(&rx).as_slice(), [Cmd::Fetch { action: FileAction::Open, .. }]));

    // nothing selected that has a file: a gentle message
    press(&mut app, '3');
    press(&mut app, 'o');
    assert!(app.toast.as_ref().is_some_and(|t| t.error && t.text.contains("no file")));
    assert!(sent(&rx).is_empty());

    app.on_event(Event::Fetched { name: "a.pdf".into(), action: FileAction::Print, result: Ok("/home/x/Downloads/a.pdf".into()) });
    assert!(app.toast.as_ref().unwrap().text.contains("double-sided"));
    app.on_event(Event::Fetched { name: "a.pdf".into(), action: FileAction::Save, result: Ok("/home/x/Downloads/a (2).pdf".into()) });
    assert!(app.toast.as_ref().unwrap().text.contains("/home/x/Downloads/a (2).pdf"));
    app.on_event(Event::Fetched { name: "a.docx".into(), action: FileAction::Print, result: Err("a.docx is not a PDF".into()) });
    assert!(app.toast.as_ref().is_some_and(|t| t.error));
}

#[test]
fn the_files_tab_lists_the_schools_files_and_opens_them() {
    let (mut app, rx) = loaded();
    sent(&rx);
    press(&mut app, '4');
    assert_eq!(loads(&rx), [Key::UserFiles]);
    assert!(screen(&mut app, 110, 30).contains("Loading"));
    press(&mut app, '1');
    press(&mut app, '4');
    assert!(sent(&rx).is_empty(), "requested once");
    deliver(&mut app, Key::UserFiles, Payload::UserFiles(demo::user_files()));
    let s = screen(&mut app, 110, 30);
    assert!(s.contains("Fall schedule.pdf") && s.contains("Permission form.pdf") && s.contains("Forms"), "{s}");
    press(&mut app, 'j');
    app.on_key(key(KeyCode::Enter));
    let asked = sent(&rx);
    assert!(
        matches!(asked.as_slice(), [Cmd::Fetch { action: FileAction::Open, name, path }] if name == "Permission form.pdf" && path.contains("f=2")),
        "{asked:?}"
    );
    fail(&mut app, Key::UserFiles, "boom");
    assert!(screen(&mut app, 110, 30).contains("Fall schedule.pdf"), "a failed refresh keeps what we had");
}

#[test]
fn a_view_with_nothing_to_show_reports_the_error() {
    let (tx, _rx) = channel();
    let mut app = App::new(tx, now(), Store::new(None));
    press(&mut app, '4');
    fail(&mut app, Key::UserFiles, "boom");
    assert!(screen(&mut app, 110, 30).contains("boom"));
    assert!(matches!(app.store.user_files(), Load::Failed("boom")));
}

#[test]
fn tasks_can_be_edited_and_only_tasks() {
    let (mut app, rx) = loaded();
    sent(&rx);
    press(&mut app, 'e');
    assert!(app.modal.is_none(), "an assignment cannot be edited");
    assert!(app.toast.as_ref().is_some_and(|t| t.error));
    while app.selected_item().is_none_or(|i| !i.is_task()) {
        press(&mut app, 'j');
    }
    sent(&rx);
    press(&mut app, 'e');
    let Some(Modal::NewTask(nt)) = &app.modal else { panic!("edit modal expected") };
    assert_eq!((nt.editing, nt.desc.text.as_str(), nt.due.text.as_str()), (Some(900), "Study for the Health exam", "2026-10-01 23:00"));
    assert!(screen(&mut app, 100, 30).contains("Edit task"));
    for _ in 0..4 {
        press(&mut app, 'x'); // typed at the end
    }
    app.on_key(key(KeyCode::Tab));
    for _ in 0..20 {
        app.on_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "fri 3pm");
    app.on_key(key(KeyCode::Enter));
    let asked = sent(&rx);
    let [Cmd::EditTask { id: 900, patch }] = asked.as_slice() else { panic!("{asked:?}") };
    assert_eq!(patch.description.as_deref(), Some("Study for the Health examxxxx"));
    assert_eq!(patch.due, Some(NaiveDate::from_ymd_opt(2026, 10, 2).unwrap().and_hms_opt(15, 0, 0).unwrap()));
    app.on_event(Event::TaskEdited(Ok(())));
    assert!(matches!(sent(&rx).as_slice(), [Cmd::Load(Key::Center)]), "the list is reloaded");

    press(&mut app, 'e');
    app.on_key(key(KeyCode::Tab));
    for _ in 0..30 {
        app.on_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "whenever");
    app.on_key(key(KeyCode::Enter));
    assert!(matches!(&app.modal, Some(Modal::NewTask(nt)) if nt.error.is_some()), "a bad date keeps the dialog open");
    assert!(sent(&rx).is_empty());
}

#[test]
fn a_task_can_be_linked_to_an_assignment_or_unlinked() {
    let (mut app, rx) = loaded();
    sent(&rx);
    press(&mut app, 'l');
    assert!(app.modal.is_none() && app.toast.as_ref().is_some_and(|t| t.error), "assignments cannot be linked");
    while app.selected_item().is_none_or(|i| !i.is_task()) {
        press(&mut app, 'j');
    }
    sent(&rx);
    press(&mut app, 'l');
    let s = screen(&mut app, 100, 30);
    assert!(s.contains("Link task to") && s.contains("(no class or assignment)") && s.contains("Unit 3 Test"), "{s}");
    assert!(app.link_choices("").iter().flatten().all(|i| !app.items()[*i].is_task()), "tasks are not offered");
    type_text(&mut app, "unit 3");
    app.on_key(key(KeyCode::Down));
    app.on_key(key(KeyCode::Enter));
    let asked = sent(&rx);
    let [Cmd::EditTask { id: 900, patch }] = asked.as_slice() else { panic!("{asked:?}") };
    assert_eq!((patch.assignment_index_id, patch.unlink), (Some(7), false));

    press(&mut app, 'l');
    app.on_key(key(KeyCode::Enter)); // the first choice is "no link"
    let asked = sent(&rx);
    assert!(
        matches!(asked.as_slice(), [Cmd::EditTask { id: 900, patch }] if patch.unlink && patch.assignment_index_id.is_none()),
        "{asked:?}"
    );

    // by mouse: click a row
    press(&mut app, 'l');
    screen(&mut app, 100, 30);
    let (rect, _) = *app.hits.modal_rows.iter().find(|(_, n)| *n == 1).unwrap();
    click(&mut app, rect.x + 3, rect.y);
    assert!(app.modal.is_none());
    assert!(matches!(sent(&rx).as_slice(), [Cmd::EditTask { patch, .. }] if patch.assignment_index_id.is_some()));
    press(&mut app, 'l');
    app.on_key(key(KeyCode::Esc));
    assert!(app.modal.is_none() && sent(&rx).is_empty());
}

#[test]
fn the_mouse_drives_the_class_views() {
    let (mut app, rx) = loaded();
    class_view(&mut app, &rx, ClassTab::Work);
    screen(&mut app, 110, 30);
    let (r, _) = *app.hits.sub_tabs.iter().find(|(_, t)| *t == ClassTab::News).unwrap();
    click(&mut app, r.x + 1, r.y);
    assert_eq!(app.class_tab, ClassTab::News);
    assert!(loads(&rx).is_empty(), "the feed already fetched it");
    screen(&mut app, 110, 30);
    let (r, i) = app.hits.sub_rows[1];
    click(&mut app, r.x + 2, r.y);
    assert_eq!((app.focus, app.sub_sel, i), (Focus::Sub, 1, 1));
    assert!(screen(&mut app, 110, 30).contains("Welcome back"));
    let (r, i) = app.hits.rows[2];
    click(&mut app, r.x + 2, r.y);
    assert_eq!((app.focus, app.class_sel, i), (Focus::Classes, 2, 2));
    assert_eq!(app.sub_sel, 0, "a different class starts at the top");
}

#[test]
fn every_view_renders_at_every_size_with_everything_loaded() {
    for (w, h) in [(160, 50), (110, 30), (80, 24), (50, 12), (30, 8), (12, 4), (3, 3), (1, 1)] {
        let (mut app, rx) = loaded();
        for tab in ['1', '2', '3', '4'] {
            press(&mut app, tab);
            answer_all(&mut app, &rx);
            screen(&mut app, w, h);
        }
        press(&mut app, '2');
        press(&mut app, 'l');
        for _ in 0..8 {
            press(&mut app, '>');
            answer_all(&mut app, &rx);
            screen(&mut app, w, h);
            press(&mut app, 'j');
            screen(&mut app, w, h);
        }
        press(&mut app, '1');
        while app.selected_item().is_none_or(|i| !i.is_task()) {
            press(&mut app, 'j');
        }
        for opener in ['e', 'l', 'P'] {
            press(&mut app, opener);
            screen(&mut app, w, h);
            app.modal = None;
        }
        press(&mut app, 'l');
        type_text(&mut app, "zzz");
        screen(&mut app, w, h);
    }
}
