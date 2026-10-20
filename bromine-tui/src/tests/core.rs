use super::*;

// ------------------------------------------------------------------ rendering

#[test]
fn the_assignment_view_shows_groups_rows_counts_and_details() {
    let (mut app, _rx) = loaded();
    let s = screen(&mut app, 110, 30);
    for want in [
        "bromine",
        "1 Assignments",
        "2 Classes",
        "3 Feed",
        "Due today",
        "Due tomorrow",
        "Due next week",
        "Due later",
        "Post-Lab: Distillation",
        "Study for the Health exam",
        "task",
        "8 open · 1 due today",
        "Details",
        "Status",
        "To do",
        "In progress",
        "Done",
    ] {
        assert!(s.contains(want), "missing {want:?} in:\n{s}");
    }
    assert!(!s.contains("hw 4"), "past work is hidden until p: \n{s}");
    assert!(s.contains("today 8:00a") && s.contains("tmrw 8:00a"), "relative due times:\n{s}");
    assert!(s.contains("‼ Unit 3 Test"), "major assignments are marked:\n{s}");
    assert!(s.contains("late"), "flags show:\n{s}");
}

#[test]
fn rendering_never_panics_at_any_size() {
    for (w, h) in [(120, 40), (80, 24), (40, 10), (20, 5), (10, 3), (3, 3), (1, 1), (0, 0)] {
        let (mut app, _rx) = loaded();
        for tab in ['1', '2', '3', '4'] {
            press(&mut app, tab);
            screen(&mut app, w, h);
        }
        press(&mut app, '2');
        for _ in ClassTab::ALL {
            press(&mut app, '>');
            screen(&mut app, w, h);
        }
        for opener in ['?', 'n', 'e', 'l'] {
            press(&mut app, '1');
            press(&mut app, opener);
            screen(&mut app, w, h);
            app.on_key(key(KeyCode::Esc));
            app.modal = None;
        }
        app.on_event(Event::LoginUrl { url: "https://example.org/".repeat(10), opened: false });
        screen(&mut app, w, h);
    }
}

#[test]
fn empty_and_loading_states_say_so() {
    let (tx, _rx) = channel();
    let mut app = App::new(tx, now(), Store::new(None));
    assert!(screen(&mut app, 100, 20).contains("Loading"));
    deliver(&mut app, Key::Center, Payload::Center(vec![]));
    assert!(screen(&mut app, 100, 20).contains("Nothing to show"));
}

// ------------------------------------------------------------------ keyboard

#[test]
fn start_asks_for_the_first_screens_data_and_selecting_fetches_details_once() {
    let (tx, rx) = channel();
    let mut app = App::new(tx, now(), Store::new(None));
    app.start();
    assert_eq!(loads(&rx), [Key::Center, Key::Classes, Key::Feed]);
    deliver(&mut app, Key::Center, Payload::Center(demo::items(now())));
    assert_eq!(loads(&rx), [Key::Detail(1)], "the first item's details are fetched");
    press(&mut app, 'j');
    assert_eq!(loads(&rx), [Key::Detail(2)]);
    press(&mut app, 'k');
    assert!(sent(&rx).is_empty(), "item 1's details are already on their way, so no second request");
}

#[test]
fn arrows_skip_headers_and_the_ends_stop() {
    let (mut app, _rx) = loaded();
    assert_eq!(selected_id(&app), 1);
    press(&mut app, 'j');
    assert_eq!(selected_id(&app), 2);
    press(&mut app, 'j'); // crosses the "Due tomorrow" header
    assert_eq!(selected_id(&app), 3);
    app.on_key(key(KeyCode::Down));
    assert_eq!(selected_id(&app), 4);
    press(&mut app, 'G');
    assert_eq!(selected_id(&app), 8);
    press(&mut app, 'j');
    assert_eq!(selected_id(&app), 8, "the end stops");
    press(&mut app, 'g');
    assert_eq!(selected_id(&app), 1);
    press(&mut app, 'k');
    assert_eq!(selected_id(&app), 1, "the top stops, and never lands on a header");
    app.on_key(key(KeyCode::PageDown));
    assert!(matches!(app.rows[app.sel], Row::Item(_)));
}

#[test]
fn the_selection_stays_visible_when_the_list_is_taller_than_the_screen() {
    let (mut app, _rx) = loaded();
    screen(&mut app, 100, 10); // a short list pane
    press(&mut app, 'G');
    let s = screen(&mut app, 100, 10);
    assert!(s.contains("Periods 1 & 2 Assessment"), "the last item scrolled into view:\n{s}");
    press(&mut app, 'g');
    assert!(screen(&mut app, 100, 10).contains("Due today"), "scrolled back to the first section header");
}

#[test]
fn status_keys_are_optimistic_and_undone_if_the_lms_refuses() {
    let (mut app, rx) = loaded();
    sent(&rx);
    press(&mut app, 'j');
    press(&mut app, 'j'); // item 3, to do
    assert_eq!(item_status(&app, 3), Status::Todo);
    press(&mut app, 'c');
    assert_eq!(item_status(&app, 3), Status::Completed, "shown immediately");
    assert!(sent(&rx).iter().any(|c| matches!(c, Cmd::SetStatus { id: 3, status: Status::Completed, task: false })));
    app.on_event(Event::StatusSet { id: 3, status: Status::Completed, task: false, result: Err("takes a file submission".into()) });
    assert_eq!(item_status(&app, 3), Status::Todo, "reverted");
    assert!(app.toast.as_ref().is_some_and(|t| t.error && t.text.contains("file submission")));

    press(&mut app, 'i');
    app.on_event(Event::StatusSet { id: 3, status: Status::InProgress, task: false, result: Ok(()) });
    assert_eq!(item_status(&app, 3), Status::InProgress);
    assert!(app.toast.as_ref().is_some_and(|t| !t.error && t.text.contains("in progress")));
    sent(&rx);
    press(&mut app, 'i');
    assert!(sent(&rx).iter().all(|c| !matches!(c, Cmd::SetStatus { .. })), "setting the same status again does nothing");
}

#[test]
fn tasks_use_task_updates_and_the_detail_pane_reflects_the_status() {
    let (mut app, rx) = loaded();
    press(&mut app, 'G');
    for _ in 0..3 {
        press(&mut app, 'k');
    }
    assert_eq!(selected_id(&app), 900);
    sent(&rx);
    press(&mut app, 'c');
    assert!(sent(&rx).iter().any(|c| matches!(c, Cmd::SetStatus { id: 900, task: true, .. })));
    assert!(screen(&mut app, 110, 30).contains("Your own to-do"));
}

#[test]
fn filter_narrows_by_name_or_class_and_escape_clears_it() {
    let (mut app, _rx) = loaded();
    press(&mut app, '/');
    type_text(&mut app, "lab");
    let names: Vec<_> =
        app.rows.iter().filter_map(|r| if let Row::Item(i) = r { Some(app.items()[*i].name.clone()) } else { None }).collect();
    assert_eq!(names, ["Post-Lab: Distillation", "Pre-Lab: Heat of Fusion"]);
    assert!(screen(&mut app, 100, 20).contains("/lab"));
    app.on_key(key(KeyCode::Enter));
    press(&mut app, 'q'); // typing is over, so q is a command again
    assert!(app.should_quit);

    let (mut app, _rx) = loaded();
    press(&mut app, '/');
    type_text(&mut app, "spanish");
    assert_eq!(app.rows.len(), 2, "one header and the one upcoming Spanish item: {:?}", app.rows);
    app.on_key(key(KeyCode::Esc));
    assert!(app.filter.is_empty() && !app.filtering);
    assert!(app.rows.len() > 2);
}

#[test]
fn p_and_h_toggle_past_work_and_completed_items() {
    let (mut app, _rx) = loaded();
    let count = |app: &App| app.rows.iter().filter(|r| matches!(r, Row::Item(_))).count();
    assert_eq!(count(&app), 9, "the 8 open items and the completed one due today");
    press(&mut app, 'p');
    assert_eq!(count(&app), 11);
    assert!(screen(&mut app, 110, 40).contains("hw 4"));
    press(&mut app, 'h');
    assert_eq!(app.items().iter().filter(|i| i.status.is_done()).count(), 3);
    assert_eq!(count(&app), 8, "hiding done removes the completed one due today and the two completed/graded past ones");
}

#[test]
fn new_task_modal_validates_then_sends_and_refreshes() {
    let (mut app, rx) = loaded();
    sent(&rx);
    press(&mut app, 'n');
    let Some(Modal::NewTask(nt)) = &app.modal else { panic!("modal") };
    assert_eq!(nt.due.text, "2026-10-01", "defaults to tomorrow");
    assert!(screen(&mut app, 100, 30).contains("New task"));

    app.on_key(key(KeyCode::Enter)); // empty description
    let Some(Modal::NewTask(nt)) = &app.modal else { panic!("still open") };
    assert!(nt.error.is_some());
    assert!(sent(&rx).is_empty());

    type_text(&mut app, "buy glue");
    app.on_key(key(KeyCode::Tab));
    for _ in 0..10 {
        app.on_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "soon");
    app.on_key(key(KeyCode::Enter));
    let Some(Modal::NewTask(nt)) = &app.modal else { panic!("still open") };
    assert!(nt.error.as_deref().is_some_and(|e| e.starts_with("Due")), "{:?}", nt.error);

    for _ in 0..4 {
        app.on_key(key(KeyCode::Backspace));
    }
    type_text(&mut app, "fri 3pm");
    app.on_key(key(KeyCode::Enter));
    assert!(app.modal.is_none());
    let due = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap().and_hms_opt(15, 0, 0).unwrap();
    assert!(matches!(&sent(&rx)[..], [Cmd::CreateTask { description, due: d }] if description == "buy glue" && *d == due));
    app.on_event(Event::TaskCreated(Ok(1)));
    assert!(matches!(&sent(&rx)[..], [Cmd::Load(Key::Center)]), "the center is reloaded to show it");
}

#[test]
fn new_task_modal_escape_cancels_and_editing_keys_work() {
    let (mut app, rx) = loaded();
    sent(&rx);
    press(&mut app, 'n');
    type_text(&mut app, "abc");
    app.on_key(key(KeyCode::Left));
    press(&mut app, 'X');
    app.on_key(key(KeyCode::Home));
    app.on_key(key(KeyCode::Delete));
    let Some(Modal::NewTask(nt)) = &app.modal else { panic!() };
    assert_eq!(nt.desc.text, "bXc");
    app.on_key(key(KeyCode::Esc));
    assert!(app.modal.is_none() && sent(&rx).is_empty());
}

#[test]
fn only_tasks_can_be_deleted_and_only_after_confirming() {
    let (mut app, rx) = loaded();
    sent(&rx);
    press(&mut app, 'x');
    assert!(app.modal.is_none());
    assert!(app.toast.as_ref().is_some_and(|t| t.error && t.text.contains("Only your own tasks")));

    press(&mut app, 'G');
    for _ in 0..3 {
        press(&mut app, 'k');
    }
    press(&mut app, 'x');
    assert!(matches!(&app.modal, Some(Modal::ConfirmDelete { id: 900, .. })));
    assert!(screen(&mut app, 100, 30).contains("Delete task?"));
    press(&mut app, 'n');
    assert!(app.modal.is_none() && sent(&rx).iter().all(|c| !matches!(c, Cmd::DeleteTask(_))));

    press(&mut app, 'x');
    press(&mut app, 'y');
    assert!(sent(&rx).iter().any(|c| matches!(c, Cmd::DeleteTask(900))));
    app.on_event(Event::TaskDeleted { id: 900, result: Ok(()) });
    assert!(app.items().iter().all(|i| i.id != 900));
}

#[test]
fn classes_enter_filters_the_assignments_and_the_feed_opens_an_assignment() {
    let (mut app, rx) = loaded();
    sent(&rx);
    press(&mut app, '2');
    let asked = loads(&rx);
    assert!(asked.contains(&Key::ClassInfo(101)) && asked.contains(&Key::ClassRows(101)), "{asked:?}");
    assert!(screen(&mut app, 110, 30).contains("BIOLOGY - 20"));
    press(&mut app, 'j');
    press(&mut app, 'j');
    press(&mut app, 'f');
    assert_eq!(app.tab, Tab::Assignments);
    assert_eq!(app.filter, "PRECALCULUS");
    assert!(app.rows.iter().all(|r| match r {
        Row::Item(i) => app.items()[*i].class.starts_with("PRECALCULUS"),
        Row::Header(_) => true,
    }));

    let (mut app, _rx) = loaded();
    press(&mut app, '3');
    let s = screen(&mut app, 110, 30);
    assert!(s.contains("Socratic Seminar") && s.contains("Enter opens this assignment") && s.contains("Lab goggles"), "{s}");
    app.on_key(key(KeyCode::Enter));
    assert_eq!((app.tab, selected_id(&app)), (Tab::Assignments, 5));
    press(&mut app, '3');
    press(&mut app, 'j');
    app.on_key(key(KeyCode::Enter)); // a post that is not an assignment: nothing happens
    assert_eq!(app.tab, Tab::Feed);
}

#[test]
fn class_data_is_fetched_once_and_rendered() {
    let (mut app, rx) = loaded();
    sent(&rx);
    press(&mut app, '2');
    assert_eq!(loads(&rx), [Key::ClassInfo(101), Key::ClassRows(101)]);
    press(&mut app, '1');
    press(&mut app, '2');
    assert!(sent(&rx).is_empty(), "already pending");
    deliver(&mut app, Key::ClassInfo(101), Payload::ClassInfo(Box::new(demo::class_info(101))));
    deliver(&mut app, Key::ClassRows(101), Payload::ClassRows(vec![]));
    assert!(screen(&mut app, 110, 30).contains("No current assignments"));
    press(&mut app, '<');
    assert_eq!(app.class_tab, ClassTab::About);
    let s = screen(&mut app, 110, 30);
    assert!(
        s.contains("Teacher")
            && s.contains("Ms. Alvarez")
            && s.contains("Room")
            && s.contains("404")
            && s.contains("First Semester, Second Semester"),
        "{s}"
    );
}

#[test]
fn the_login_modal_shows_progress_the_url_and_failure() {
    let (mut app, _rx) = loaded();
    app.on_event(Event::LoginNeeded);
    assert!(screen(&mut app, 100, 30).contains("Opening the sign-in page"));
    app.on_event(Event::LoginUrl { url: "https://id.blackbaud.com/authorize?x=1".into(), opened: true });
    let s = screen(&mut app, 100, 30);
    assert!(s.contains("Sign in") && s.contains("browser window was opened") && s.contains("https://id.blackbaud.com/authorize"), "{s}");
    app.on_event(Event::LoginUrl { url: "https://id.blackbaud.com/a".into(), opened: false });
    assert!(screen(&mut app, 100, 30).contains("open this address yourself"));
    app.on_event(Event::LoginFailed("no callback received within 300s".into()));
    assert!(screen(&mut app, 100, 30).contains("no callback received"));
    press(&mut app, 'r');
    assert!(app.modal.is_none());
    app.on_event(Event::LoginNeeded);
    app.on_event(Event::LoginDone);
    assert!(app.modal.is_none() && app.toast.as_ref().is_some_and(|t| t.text == "Signed in"));
}

#[test]
fn help_opens_and_any_key_closes_it() {
    let (mut app, _rx) = loaded();
    press(&mut app, '?');
    let s = screen(&mut app, 100, 30);
    assert!(s.contains("Keys") && s.contains("new / edit") && s.contains("Mouse:"), "{s}");
    press(&mut app, 'z');
    assert!(app.modal.is_none());
}

#[test]
fn ctrl_c_and_q_quit_but_typing_q_in_a_field_does_not() {
    let (mut app, _rx) = loaded();
    press(&mut app, 'n');
    press(&mut app, 'q');
    assert!(!app.should_quit);
    app.on_key(key(KeyCode::Esc));
    app.on_key(KeyEvent {
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    });
    assert!(app.should_quit);
}

// ------------------------------------------------------------------ mouse

#[test]
fn clicking_tabs_switches_views() {
    let (mut app, rx) = loaded();
    screen(&mut app, 110, 30);
    sent(&rx);
    let (rect, tab) = app.hits.tabs[1];
    assert_eq!(tab, Tab::Classes);
    click(&mut app, rect.x + 1, rect.y);
    assert_eq!(app.tab, Tab::Classes);
    assert!(!sent(&rx).is_empty(), "class data requested");
    screen(&mut app, 110, 30);
    let (rect, _) = app.hits.tabs[2];
    click(&mut app, rect.x + rect.width - 1, rect.y);
    assert_eq!(app.tab, Tab::Feed);
    click(&mut app, 109, 0); // empty space on the bar
    assert_eq!(app.tab, Tab::Feed);
}

#[test]
fn clicking_a_row_selects_it_but_clicking_a_header_does_not() {
    let (mut app, _rx) = loaded();
    screen(&mut app, 110, 30);
    let (rect, row) = app.hits.rows[5]; // rows: H i1 i2 H i3 i4 ...
    assert_eq!(row, 5);
    click(&mut app, rect.x + 4, rect.y);
    assert_eq!(selected_id(&app), 4);
    assert_eq!(app.sel, 5);
    let header = app.hits.rows[3].0;
    click(&mut app, header.x + 2, header.y);
    assert_eq!(selected_id(&app), 4, "a header is not selectable");
    click(&mut app, 0, 29); // outside everything
    assert_eq!(selected_id(&app), 4);
}

#[test]
fn clicking_a_status_button_sets_that_status() {
    let (mut app, rx) = loaded();
    screen(&mut app, 110, 30);
    sent(&rx);
    assert_eq!(app.hits.buttons.len(), 3);
    let (rect, status) = app.hits.buttons[2];
    assert_eq!(status, Status::Completed);
    assert_eq!(selected_id(&app), 1);
    click(&mut app, rect.x + 2, rect.y);
    assert_eq!(item_status(&app, 1), Status::Completed);
    assert!(sent(&rx).iter().any(|c| matches!(c, Cmd::SetStatus { id: 1, status: Status::Completed, task: false })));
    screen(&mut app, 110, 30);
    let (rect, _) = app.hits.buttons[0];
    click(&mut app, rect.x, rect.y);
    assert_eq!(item_status(&app, 1), Status::Todo);
}

#[test]
fn the_wheel_scrolls_the_list_without_moving_the_selection_and_the_detail_pane_separately() {
    let (mut app, _rx) = loaded();
    screen(&mut app, 100, 10);
    let list = app.hits.list;
    assert!(app.rows.len() > list.height as usize, "a list taller than the pane");
    wheel(&mut app, list.x + 2, list.y + 2, true);
    assert!(app.offset > 0 && app.offset <= app.rows.len() - list.height as usize, "scrolled, and not past the end: {}", app.offset);
    assert_eq!(selected_id(&app), 1, "the selection did not move");
    wheel(&mut app, list.x + 2, list.y + 2, false);
    assert_eq!(app.offset, 0);

    {
        let d = demo::detail(&app.items()[0]);
        deliver(&mut app, Key::Detail(1), Payload::Detail(Box::new(d)));
    }
    screen(&mut app, 100, 14);
    let detail = app.hits.detail;
    wheel(&mut app, detail.x + 2, detail.y + 1, true);
    assert!(app.detail_scroll > 0, "the wheel over the detail pane scrolls it");
    assert_eq!(app.offset, 0);
    press(&mut app, 'j');
    assert_eq!(app.detail_scroll, 0, "a new selection starts at the top");
}

#[test]
fn modal_buttons_are_clickable() {
    let (mut app, rx) = loaded();
    press(&mut app, 'n');
    screen(&mut app, 100, 30);
    sent(&rx);
    type_text(&mut app, "x");
    let cancel = app.hits.modal_buttons.iter().find(|(_, a)| *a == crate::app::ModalAction::Cancel).unwrap().0;
    click(&mut app, cancel.x + 1, cancel.y);
    assert!(app.modal.is_none() && sent(&rx).is_empty());

    press(&mut app, 'n');
    type_text(&mut app, "click task");
    screen(&mut app, 100, 30);
    let submit = app.hits.modal_buttons.iter().find(|(_, a)| *a == crate::app::ModalAction::Submit).unwrap().0;
    click(&mut app, submit.x + 2, submit.y);
    assert!(matches!(&sent(&rx)[..], [Cmd::CreateTask { description, .. }] if description == "click task"));

    press(&mut app, '?');
    screen(&mut app, 100, 30);
    click(&mut app, 50, 15);
    assert!(app.modal.is_none(), "click anywhere dismisses help");
}

#[test]
fn clicks_and_wheel_are_ignored_behind_a_modal() {
    let (mut app, _rx) = loaded();
    screen(&mut app, 110, 30);
    let (rect, _) = app.hits.tabs[2];
    app.on_event(Event::LoginNeeded);
    screen(&mut app, 110, 30);
    click(&mut app, rect.x + 1, rect.y);
    assert_eq!(app.tab, Tab::Assignments);
    wheel(&mut app, 5, 5, true);
    assert_eq!(app.offset, 0);
}

// ------------------------------------------------------------------ worker events

#[test]
fn a_reloaded_center_keeps_the_selection_and_pending_counts_balance() {
    let (mut app, rx) = loaded();
    press(&mut app, 'j');
    press(&mut app, 'j');
    press(&mut app, 'j');
    assert_eq!(selected_id(&app), 4);
    sent(&rx);
    deliver(&mut app, Key::Center, Payload::Center(demo::items(now())));
    assert_eq!(selected_id(&app), 4, "same item stays selected after a refresh");

    let (mut app, _rx) = loaded();
    let before = app.busy();
    assert!(before > 0, "the first item's details and the feed sources are in flight");
    {
        let d = demo::detail(&app.items()[0]);
        deliver(&mut app, Key::Detail(1), Payload::Detail(Box::new(d)));
    }
    assert_eq!(app.busy(), before - 1, "an answered read is no longer in flight");
    app.on_event(Event::LoginNeeded);
    assert_eq!(app.busy(), before - 1, "sign-in progress events are not command results");
}

#[test]
fn details_show_description_files_submission_and_grade_and_correct_the_lists_status() {
    let (mut app, _rx) = loaded();
    let mut d = demo::detail(&app.items()[1]);
    d.status = Status::Completed;
    press(&mut app, 'j'); // item 2: has a typed submission in the demo data
    deliver(&mut app, Key::Detail(2), Payload::Detail(Box::new(d)));
    let s = screen(&mut app, 120, 36);
    for want in ["Read books 8 & 9", "Complete the work for", "Files", "Handout.pdf", "Your submission", "sea monsters"] {
        assert!(s.contains(want), "missing {want:?}:\n{s}");
    }

    // a detail whose own status differs from the list's wins
    let mut d = demo::detail(&app.items()[0]);
    d.status = Status::Todo;
    deliver(&mut app, Key::Detail(1), Payload::Detail(Box::new(d)));
    assert_eq!(item_status(&app, 1), Status::Todo);

    // a file-submission assignment explains why Done is refused
    press(&mut app, 'j');
    press(&mut app, 'j');
    assert_eq!(selected_id(&app), 4);
    {
        let d = demo::detail(&app.items()[3]);
        deliver(&mut app, Key::Detail(4), Payload::Detail(Box::new(d)));
    }
    assert!(screen(&mut app, 120, 36).contains("File submission"));

    fail(&mut app, Key::Detail(5), "boom");
    press(&mut app, 'j');
    assert!(screen(&mut app, 120, 36).contains("Could not load details: boom"));
}

#[test]
fn errors_become_toasts_and_toasts_expire() {
    let (mut app, _rx) = loaded();
    fail(&mut app, Key::Center, "network down");
    assert!(screen(&mut app, 100, 20).contains("network down"));
    for _ in 0..61 {
        app.tick();
    }
    assert!(app.toast.is_none());
}

#[test]
fn confirm_delete_buttons_are_clickable() {
    let (mut app, rx) = loaded();
    press(&mut app, 'G');
    for _ in 0..3 {
        press(&mut app, 'k');
    }
    assert_eq!(selected_id(&app), 900);

    press(&mut app, 'x');
    screen(&mut app, 100, 30);
    sent(&rx);
    let cancel = app.hits.modal_buttons.iter().find(|(_, a)| *a == crate::app::ModalAction::Cancel).unwrap().0;
    click(&mut app, cancel.x + 2, cancel.y);
    assert!(app.modal.is_none() && sent(&rx).is_empty(), "Cancel closes without deleting");

    press(&mut app, 'x');
    screen(&mut app, 100, 30);
    let delete = app.hits.modal_buttons.iter().find(|(_, a)| *a == crate::app::ModalAction::Submit).unwrap().0;
    click(&mut app, delete.x + 2, delete.y);
    assert!(app.modal.is_none());
    assert!(matches!(&sent(&rx)[..], [Cmd::DeleteTask(900)]));
}

#[test]
fn on_a_narrow_terminal_the_tabs_win_over_the_counters() {
    let (mut app, _rx) = loaded();
    let narrow = screen(&mut app, 50, 12);
    let top = narrow.lines().next().unwrap();
    assert!(top.contains("1 Assignments") && top.contains("2 Classes") && top.contains("3 Feed"), "{top:?}");
    assert!(!top.contains("open"), "the counter is dropped, not drawn over the tabs: {top:?}");
    assert!(screen(&mut app, 120, 12).lines().next().unwrap().contains("8 open · 1 due today"));
}
