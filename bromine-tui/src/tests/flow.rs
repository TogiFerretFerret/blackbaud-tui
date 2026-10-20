//! Freshness: what is shown from the cache, what is refetched when, and that refreshing never blanks a screen.

use chrono::Duration;

use super::*;
use crate::cache;
use crate::data::FeedKind;

/// A store as the cache would hand it back, everything fetched at `fetched`.
fn warm_store(fetched: NaiveDateTime) -> Store {
    let mut s = Store::new(None);
    s.apply(&Key::Center, Payload::Center(demo::items(now())), fetched);
    s.apply(&Key::Classes, Payload::Classes(demo::classes()), fetched);
    s.apply(&Key::Feed, Payload::Feed(demo::feed(now())), fetched);
    for c in demo::classes() {
        s.apply(&Key::News(c.section_id), Payload::News(demo::news(now())), fetched);
        s.apply(&Key::Topics(c.section_id), Payload::Topics(demo::topics(now())), fetched);
    }
    s
}

fn started(store: Store) -> (App, Receiver<Cmd>) {
    let (tx, rx) = channel();
    let mut app = App::new(tx, now(), store);
    app.start();
    (app, rx)
}

fn advance(app: &mut App, by: Duration) {
    app.now += by;
}

#[test]
fn a_fresh_cache_paints_at_once_and_asks_for_nothing_but_the_first_details() {
    let (mut app, rx) = started(warm_store(now()));
    assert_eq!(loads(&rx), [Key::Detail(1)], "everything else is fresh");
    let s = screen(&mut app, 120, 30);
    assert!(s.contains("Post-Lab: Distillation") && s.contains("updated just now"), "{s}");
    assert!(!s.contains("Loading…") && !s.contains("Nothing to show"), "{s}");
}

#[test]
fn a_stale_cache_shows_the_old_data_while_it_refetches_in_the_background() {
    let (mut app, rx) = started(warm_store(now() - Duration::minutes(10)));
    let asked = loads(&rx);
    for key in [Key::Center, Key::Feed, Key::News(101), Key::Topics(105), Key::Detail(1)] {
        assert!(asked.contains(&key), "{key:?} is stale and should be refetched: {asked:?}");
    }
    assert!(!asked.contains(&Key::Classes), "classes only change rarely: {asked:?}");
    let s = screen(&mut app, 120, 30);
    assert!(s.contains("Post-Lab: Distillation"), "the old data is on screen meanwhile:\n{s}");
    assert!(s.contains("updating") && s.contains("updated 10m ago") && !s.contains("Loading…"), "{s}");

    // the fresh answer replaces it without a flicker of "Loading"
    let mut items = demo::items(now());
    items.retain(|i| i.id != 1);
    deliver(&mut app, Key::Center, Payload::Center(items));
    let s = screen(&mut app, 120, 30);
    assert!(!s.contains("Post-Lab: Distillation") && s.contains("updated just now"), "{s}");
}

#[test]
fn r_refreshes_what_is_on_screen_and_only_that() {
    let (mut app, rx) = loaded();
    answer_all(&mut app, &rx);
    press(&mut app, 'r');
    assert!(screen(&mut app, 110, 30).contains("Post-Lab: Distillation"), "nothing is blanked while it loads");
    assert_eq!(answered(&mut app, &rx), [Key::Center, Key::Detail(1)], "the assignment list and the selected assignment");

    press(&mut app, '3');
    press(&mut app, 'r');
    let asked = answered(&mut app, &rx);
    assert!(asked.contains(&Key::Feed) && asked.contains(&Key::News(101)) && asked.contains(&Key::Topics(105)), "{asked:?}");
    assert!(!asked.contains(&Key::Center), "the other views are left alone: {asked:?}");

    press(&mut app, 'R');
    let asked = answered(&mut app, &rx);
    for key in [Key::Center, Key::Classes, Key::Feed, Key::News(103)] {
        assert!(asked.contains(&key), "R refreshes everything, including {key:?}: {asked:?}");
    }
}

#[test]
fn a_refresh_already_on_its_way_is_not_asked_twice() {
    let (mut app, rx) = loaded();
    answer_all(&mut app, &rx);
    press(&mut app, 'r');
    press(&mut app, 'r');
    press(&mut app, 'R');
    let asked = loads(&rx);
    assert_eq!(asked.iter().filter(|k| **k == Key::Center).count(), 1, "{asked:?}");
}

#[test]
fn polling_refetches_only_what_has_gone_stale() {
    let (mut app, rx) = loaded();
    answer_all(&mut app, &rx);
    advance(&mut app, Duration::seconds(30));
    for _ in 0..100 {
        app.tick();
    }
    assert!(loads(&rx).is_empty(), "everything is still fresh");
    advance(&mut app, Duration::seconds(45)); // 75s: the center and the feed (a minute), not the details (five)
    for _ in 0..100 {
        app.tick();
    }
    assert_eq!(loads(&rx), [Key::Center, Key::Feed]);
}

#[test]
fn a_failing_refresh_keeps_the_data_tells_you_once_and_backs_off() {
    let (mut app, rx) = loaded();
    answer_all(&mut app, &rx);
    advance(&mut app, Duration::seconds(70));
    for _ in 0..100 {
        app.tick();
    }
    assert!(loads(&rx).contains(&Key::Center));
    fail(&mut app, Key::Center, "network down");
    assert!(app.toast.as_ref().is_some_and(|t| t.error && t.text.contains("network down")));
    assert!(screen(&mut app, 110, 30).contains("Post-Lab: Distillation"), "the old list stays");

    advance(&mut app, Duration::seconds(30));
    for _ in 0..100 {
        app.tick();
    }
    assert!(!loads(&rx).contains(&Key::Center), "no hammering a server that just failed");
    advance(&mut app, Duration::seconds(31));
    for _ in 0..100 {
        app.tick();
    }
    assert!(loads(&rx).contains(&Key::Center), "it tries again after a minute");
}

#[test]
fn a_stale_detail_is_refetched_when_you_come_back_to_it_but_stays_on_screen() {
    let (mut app, rx) = loaded();
    answer_all(&mut app, &rx);
    advance(&mut app, Duration::minutes(6));
    press(&mut app, 'j');
    assert_eq!(loads(&rx), [Key::Detail(2)], "never fetched");
    press(&mut app, 'k');
    assert_eq!(loads(&rx), [Key::Detail(1)], "six minutes old");
    let s = screen(&mut app, 120, 30);
    assert!(s.contains("Complete the work for") && !s.contains("Loading details"), "{s}");
}

#[test]
fn the_feed_combines_posts_announcements_and_topics_and_remembers_old_posts() {
    let (mut app, rx) = loaded();
    answer_all(&mut app, &rx);
    let kinds = |app: &App| app.feed.iter().map(|f| f.kind).collect::<Vec<_>>();
    assert!(kinds(&app).contains(&FeedKind::News) && kinds(&app).contains(&FeedKind::Topic) && kinds(&app).contains(&FeedKind::Post));
    press(&mut app, '3');
    let s = screen(&mut app, 120, 40);
    for want in ["news", "topic", "post", "New topic: Unit 2: Reactions", "Room change", "Socratic Seminar"] {
        assert!(s.contains(want), "feed missing {want:?}:\n{s}");
    }

    // the server only ever sends the newest ten; what we saw before stays
    let before = app.feed.len();
    let newer = |id: i64, title: &str| bromine::FeedItem {
        id: Some(id),
        ticks: Some(id),
        when: Some(now() + Duration::hours(id)),
        from: Some("Teacher".into()),
        class: Some("BIOLOGY - 20".into()),
        assignment_id: None,
        title: Some(title.into()),
        text: None,
    };
    deliver(&mut app, Key::Feed, Payload::Feed(vec![newer(10, "Brand new post")]));
    assert_eq!(app.feed.len(), before + 1);
    assert_eq!(app.feed[0].title.as_deref(), Some("Brand new post"), "newest first");
    assert!(app.feed.iter().any(|f| f.title.as_deref() == Some("Socratic Seminar")), "the earlier posts are still here");
}

#[test]
fn a_new_post_does_not_move_the_post_you_are_reading() {
    let (mut app, rx) = loaded();
    answer_all(&mut app, &rx);
    press(&mut app, '3');
    press(&mut app, 'j');
    press(&mut app, 'j');
    let reading = app.feed[app.feed_sel].key.clone();
    let post = bromine::FeedItem {
        id: Some(99),
        ticks: Some(99),
        when: Some(now() + Duration::hours(3)),
        from: None,
        class: None,
        assignment_id: None,
        title: Some("Breaking".into()),
        text: None,
    };
    deliver(&mut app, Key::Feed, Payload::Feed(vec![post]));
    assert_eq!(app.feed[app.feed_sel].key, reading, "the selection follows the post, not the row number");
}

#[test]
fn changes_are_saved_to_disk_and_the_next_start_uses_them() {
    let dir = std::env::temp_dir().join(format!("bbtui-flow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("cache.json");
    let (tx, rx) = channel();
    let mut app = App::new(tx, now(), Store::new(Some("student".into()))).with_cache(cache::Writer::spawn(path.clone()));
    app.start();
    deliver(&mut app, Key::Center, Payload::Center(demo::items(now())));
    deliver(&mut app, Key::Classes, Payload::Classes(demo::classes()));
    sent(&rx);
    press(&mut app, 'j');
    press(&mut app, 'c'); // an optimistic change is saved too
    for _ in 0..50 {
        app.tick();
    }
    app.shutdown();

    let stored = cache::load(&path, Some("student")).expect("saved");
    assert_eq!(stored.items().iter().find(|i| i.id == 2).map(|i| i.status), Some(Status::Completed));
    assert!(cache::load(&path, Some("someone else")).is_none());

    let (app2, rx2) = started(stored);
    assert!(app2.items().len() > 5, "shown before any request is answered");
    assert!(loads(&rx2).contains(&Key::Feed), "what the cache lacks is fetched");
    assert!(!loads(&rx2).contains(&Key::Center), "(already drained above)");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_busy_counter_follows_reads_writes_and_downloads() {
    let (mut app, rx) = loaded();
    answer_all(&mut app, &rx);
    assert_eq!(app.busy(), 0);
    press(&mut app, 'c');
    assert_eq!(app.busy(), 1, "a write is in flight");
    app.on_event(Event::StatusSet { id: 1, status: Status::Completed, task: false, result: Ok(()) });
    assert_eq!(app.busy(), 0);
    press(&mut app, 'r');
    assert!(app.busy() > 0);
    answer_all(&mut app, &rx);
    assert_eq!(app.busy(), 0);
}
