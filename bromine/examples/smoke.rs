//! Read-only check of the library against the live LMS, using the saved session.
//!   cargo run -p bromine --example smoke

use std::time::Duration;

use bromine::{Client, Endpoints, Error, SessionStore, Status, open_in_browser};

fn main() -> Result<(), Error> {
    let store = SessionStore::default();
    let ep = Endpoints::default();

    let session = match store.refresh(&ep) {
        // headless; also renews the sts-sso session
        Err(Error::LoginRequired(why)) => {
            println!("sign-in needed ({why}); opening the browser (waiting up to 10 minutes)");
            store.interactive_login(
                &ep,
                |url| {
                    println!("sign-in page: {url}");
                    println!("opened: {}", open_in_browser(url));
                },
                Duration::from_secs(600),
            )?
        }
        other => other?,
    };
    println!("session ready for {} (token not shown)", session.username);
    let client = Client::new(&session.tt)?;

    let classes = client.classes()?;
    println!("\n{} classes:", classes.len());
    for c in &classes {
        println!("  {:30} section {} lead {} ({})", c.name, c.section_id, c.lead_section_id, c.teacher.as_deref().unwrap_or("?"));
    }

    let items = client.assignment_center()?;
    println!("\nassignment center: {} items", items.len());
    for i in items.iter().filter(|i| i.bucket.is_upcoming() && !i.status.is_done()) {
        println!(
            "  [{:<13}] {:11} {:<22} {} ({})",
            i.bucket.label(),
            i.status.label(),
            i.class,
            i.name,
            i.due.map(|d| d.format("%a %b %e %l:%M %p").to_string()).unwrap_or_default()
        );
    }

    if let Some(first) = items.iter().find(|i| !i.is_task()) {
        let d = client.assignment(first.id)?;
        println!(
            "\ndetail of '{}': status {}, dropbox {}, {} downloads, {} submissions",
            d.name,
            d.status.label(),
            d.dropbox,
            d.downloads.len(),
            d.submissions.len()
        );
        assert!(matches!(
            d.status,
            Status::Todo | Status::InProgress | Status::Completed | Status::Graded | Status::Overdue | Status::Other(_)
        ));
    }

    let feed = client.activity_feed()?;
    println!("\nfeed: {} posts; newest: {:?}", feed.len(), feed.first().and_then(|f| f.title.clone()));

    if let Some(c) = classes.first() {
        let info = client.class_info(c.section_id)?;
        println!("class info: {} / {} terms", info.name, info.terms.len());
        println!(
            "topics: {}, announcements: {}, class files: {}",
            client.topics(c.section_id)?.len(),
            client.announcements(c.section_id)?.len(),
            client.class_files(c.section_id)?.len()
        );
    }
    for c in &classes {
        match client.grades(c.section_id) {
            Ok(g) => println!("grades {:30} {:?}", c.name, g.first().and_then(|g| g.unweighted_percent()).map(|p| format!("{p:.1}%"))),
            Err(Error::Refused { status, .. }) => println!("grades {:30} not shared with students (HTTP {status})", c.name),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
