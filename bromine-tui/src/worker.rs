//! The network side. The UI sends [`Cmd`]s and receives [`Event`]s, so it never blocks.
//!
//! Reads (`Cmd::Load`, file downloads) run on a small pool so a screenful of requests overlaps. Writes
//! (status changes, tasks) run one at a time in the order they were made. All of them share one session:
//! when it lapses, one thread renews it (silently, else through the browser) and the rest reuse the result.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use bromine::{Client, Endpoints, Error, SessionStore, Status, TaskPatch, Topic, open_in_browser};
use chrono::NaiveDateTime;

use crate::data::{Key, Payload};
use crate::files;

const READERS: usize = 4;

/// What to do with a file once it is downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    Save,
    Open,
    Print,
}

/// Shown instead of an error when a teacher has not shared the gradebook (HTTP 403).
pub const GRADES_NOT_SHARED: &str = "The teacher has not shared the gradebook for this class.";

#[derive(Debug, Clone)]
pub enum Cmd {
    /// Fetch one thing; answered with [`Event::Loaded`].
    Load(Key),
    SetStatus {
        id: i64,
        status: Status,
        task: bool,
    },
    CreateTask {
        description: String,
        due: NaiveDateTime,
    },
    DeleteTask(i64),
    EditTask {
        id: i64,
        patch: TaskPatch,
    },
    /// Download a file and then save, open or print it.
    Fetch {
        path: String,
        name: String,
        action: FileAction,
    },
    Quit,
}

#[derive(Debug, Clone)]
pub enum Event {
    Loaded {
        key: Key,
        result: Result<Payload, String>,
    },
    StatusSet {
        id: i64,
        status: Status,
        task: bool,
        result: Result<(), String>,
    },
    TaskCreated(Result<i64, String>),
    TaskDeleted {
        id: i64,
        result: Result<(), String>,
    },
    TaskEdited(Result<(), String>),
    /// A download finished: where it was saved (and the action that was asked for).
    Fetched {
        name: String,
        action: FileAction,
        result: Result<PathBuf, String>,
    },
    /// Sign-in is needed; the browser is being opened.
    LoginNeeded,
    LoginUrl {
        url: String,
        opened: bool,
    },
    LoginDone,
    LoginFailed(String),
}

/// Start the worker threads. The returned handle is the router; it ends on [`Cmd::Quit`] or when `rx` closes.
pub fn spawn(store: SessionStore, ep: Endpoints, rx: Receiver<Cmd>, tx: Sender<Event>) -> JoinHandle<()> {
    spawn_with(Box::new(BrowserSessions { store, ep }), rx, tx)
}

/// Where clients come from. The real one reads the saved session, renews it, and falls back to a browser
/// sign-in; tests supply their own.
pub trait Sessions: Send + Sync {
    /// A client for the saved session, if there is one.
    fn saved(&self) -> Result<Option<Arc<Client>>, Error>;
    /// A new client after the session lapsed (or when there was none). May need a person; say so on `events`.
    fn renew(&self, events: &Sender<Event>) -> Result<Arc<Client>, Error>;
}

struct BrowserSessions {
    store: SessionStore,
    ep: Endpoints,
}

impl Sessions for BrowserSessions {
    fn saved(&self) -> Result<Option<Arc<Client>>, Error> {
        match self.store.load()? {
            Some(saved) => Ok(Some(Arc::new(Client::with_endpoints(self.ep.clone(), &saved.tt)?))),
            None => Ok(None),
        }
    }

    /// Mint a new `t` from the saved credentials; failing that, sign in through the browser.
    fn renew(&self, events: &Sender<Event>) -> Result<Arc<Client>, Error> {
        let session = match self.store.refresh(&self.ep) {
            Ok(s) => s,
            Err(Error::LoginRequired(_)) => {
                let _ = events.send(Event::LoginNeeded);
                let tx = events.clone();
                let result = self.store.interactive_login(
                    &self.ep,
                    |url| {
                        let opened = open_in_browser(url);
                        let _ = tx.send(Event::LoginUrl { url: url.to_owned(), opened });
                    },
                    Duration::from_secs(300),
                );
                match result {
                    Ok(s) => {
                        let _ = events.send(Event::LoginDone);
                        s
                    }
                    Err(e) => {
                        let _ = events.send(Event::LoginFailed(e.to_string()));
                        return Err(e);
                    }
                }
            }
            Err(e) => return Err(e),
        };
        Ok(Arc::new(Client::with_endpoints(self.ep.clone(), &session.tt)?))
    }
}

pub fn spawn_with(sessions: Box<dyn Sessions>, rx: Receiver<Cmd>, tx: Sender<Event>) -> JoinHandle<()> {
    let shared = Arc::new(Shared { sessions, tx, auth: Mutex::new(Auth { client: None, generation: 0 }) });
    let (read_tx, read_rx) = channel::<Cmd>();
    let read_rx = Arc::new(Mutex::new(read_rx));
    for n in 0..READERS {
        let (shared, read_rx) = (shared.clone(), read_rx.clone());
        thread(&format!("bromine-read-{n}"), move || {
            // the guard is a temporary: it is released as soon as a command has been taken
            while let Ok(cmd) = lock(&read_rx).recv() {
                shared.run(cmd);
            }
        });
    }
    let (write_tx, write_rx) = channel::<Cmd>();
    let writer = shared.clone();
    thread("bromine-write", move || {
        while let Ok(cmd) = write_rx.recv() {
            writer.run(cmd);
        }
    });
    thread("bromine-router", move || {
        while let Ok(cmd) = rx.recv() {
            let sent = match cmd {
                Cmd::Quit => break,
                Cmd::Load(_) | Cmd::Fetch { .. } => read_tx.send(cmd),
                _ => write_tx.send(cmd),
            };
            if sent.is_err() {
                break;
            }
        }
    })
}

fn thread(name: &str, f: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::Builder::new().name(name.into()).spawn(f).expect("spawn worker thread")
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Auth {
    client: Option<Arc<Client>>,
    /// Bumped on every renewal, so a thread whose request failed can tell whether someone already fixed it.
    generation: u64,
}

struct Shared {
    sessions: Box<dyn Sessions>,
    tx: Sender<Event>,
    auth: Mutex<Auth>,
}

fn msg(e: Error) -> String {
    e.to_string()
}

impl Shared {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e); // the UI is gone: nothing left to tell
    }

    /// The current client, connecting (and if need be signing in) when there is none yet.
    fn client(&self) -> Result<(Arc<Client>, u64), Error> {
        let mut auth = lock(&self.auth);
        if let Some(c) = &auth.client {
            return Ok((c.clone(), auth.generation));
        }
        let client = match self.sessions.saved()? {
            Some(c) => c,
            None => self.sessions.renew(&self.tx)?,
        };
        auth.client = Some(client.clone());
        auth.generation += 1;
        Ok((client, auth.generation))
    }

    /// Replace the client that failed in `generation`, unless another thread already has.
    fn renew(&self, generation: u64) -> Result<(Arc<Client>, u64), Error> {
        let mut auth = lock(&self.auth);
        if auth.generation != generation
            && let Some(c) = &auth.client
        {
            return Ok((c.clone(), auth.generation));
        }
        auth.client = None;
        let client = self.sessions.renew(&self.tx)?;
        auth.client = Some(client.clone());
        auth.generation += 1;
        Ok((client, auth.generation))
    }

    /// Run `f` with a working client; a lapsed session is renewed and `f` retried once.
    fn call<T>(&self, f: impl Fn(&Client) -> Result<T, Error>) -> Result<T, Error> {
        let (client, generation) = self.client()?;
        match f(&client) {
            Err(Error::SessionExpired) => {
                let (client, _) = self.renew(generation)?;
                f(&client)
            }
            other => other,
        }
    }

    fn run(&self, cmd: Cmd) {
        match cmd {
            Cmd::Load(key) => {
                let result = self.load(&key);
                self.send(Event::Loaded { key, result });
            }
            Cmd::SetStatus { id, status, task } => {
                let result = self
                    .call(|c| {
                        if task {
                            c.update_task(id, &TaskPatch { status: Some(status), ..Default::default() })
                        } else {
                            c.set_status(id, status)
                        }
                    })
                    .map_err(msg);
                self.send(Event::StatusSet { id, status, task, result });
            }
            Cmd::CreateTask { description, due } => {
                let r = self.call(|c| c.create_task(&description, due)).map_err(msg);
                self.send(Event::TaskCreated(r));
            }
            Cmd::DeleteTask(id) => {
                let r = self.call(|c| c.delete_task(id)).map_err(msg);
                self.send(Event::TaskDeleted { id, result: r });
            }
            Cmd::EditTask { id, patch } => {
                let r = self.call(|c| c.update_task(id, &patch)).map_err(msg);
                self.send(Event::TaskEdited(r));
            }
            Cmd::Fetch { path, name, action } => {
                let result = self.fetch(&path, action);
                let name = result.as_ref().ok().and_then(|p| p.file_name()).map_or(name, |n| n.to_string_lossy().into_owned());
                self.send(Event::Fetched { name, action, result });
            }
            Cmd::Quit => {}
        }
    }

    fn load(&self, key: &Key) -> Result<Payload, String> {
        self.call(|c| {
            Ok(match key {
                Key::Center => Payload::Center(c.assignment_center()?),
                Key::Classes => Payload::Classes(c.classes()?),
                Key::Feed => Payload::Feed(c.activity_feed()?),
                Key::Detail(id) => Payload::Detail(Box::new(c.assignment(*id)?)),
                Key::ClassInfo(section) => Payload::ClassInfo(Box::new(c.class_info(*section)?)),
                Key::ClassRows(lead) => Payload::ClassRows(c.class_assignments(*lead, 1)?),
                Key::Topics(section) => Payload::Topics(c.topics(*section)?),
                Key::TopicItems { topic, index } => Payload::TopicItems(c.topic_items(&Topic {
                    topic_id: *topic,
                    index_id: *index,
                    name: String::new(),
                    published: None,
                })?),
                Key::News(section) => Payload::News(c.announcements(*section)?),
                Key::ClassFiles(section) => Payload::ClassFiles(c.class_files(*section)?),
                Key::Grades(section) => Payload::Grades(c.grades(*section)?),
                Key::People(section) => Payload::People(c.roster(*section)?),
                Key::UserFiles => Payload::UserFiles(c.user_files()?),
            })
        })
        .map_err(|e| match (key, e) {
            (Key::Grades(_), Error::Refused { .. }) => GRADES_NOT_SHARED.to_owned(),
            (_, e) => msg(e),
        })
    }

    fn fetch(&self, path: &str, action: FileAction) -> Result<PathBuf, String> {
        let file = self.call(|c| c.download(path)).map_err(msg)?;
        if action == FileAction::Print && !files::is_pdf(&file.name) {
            return Err(format!("{} is not a PDF; save or open it instead", file.name));
        }
        let saved = files::download_dir().and_then(|dir| files::save_unique(&dir, &file.name, &file.bytes)).map_err(|e| e.to_string())?;
        match action {
            FileAction::Save => Ok(()),
            FileAction::Open => files::open(&saved),
            FileAction::Print => files::print(&saved),
        }
        .map_err(|e| format!("saved to {} but: {e}", saved.display()))?;
        Ok(saved)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use mockito::{Matcher, Server};
    use serde_json::json;

    use super::*;

    /// Sessions whose first client is already expired; renewing hands out one that works.
    struct Lapsed {
        lms: String,
        renewals: Arc<AtomicUsize>,
    }

    impl Lapsed {
        fn client(&self, tt: &str) -> Arc<Client> {
            let ep = Endpoints { lms: self.lms.clone(), ..Endpoints::default() };
            Arc::new(Client::with_endpoints(ep, tt).unwrap())
        }
    }

    impl Sessions for Lapsed {
        fn saved(&self) -> Result<Option<Arc<Client>>, Error> {
            Ok(Some(self.client("old")))
        }

        fn renew(&self, _events: &Sender<Event>) -> Result<Arc<Client>, Error> {
            self.renewals.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(150)); // long enough for the other readers to pile up
            Ok(self.client("new"))
        }
    }

    fn cookie(tt: &str) -> Matcher {
        Matcher::Regex(format!(r"(^|;\s*)t={tt}(;|$)"))
    }

    #[test]
    fn many_reads_share_one_renewal_when_the_session_lapses() {
        let mut lms = Server::new();
        let _expired = lms
            .mock("GET", Matcher::Regex("^/api/".into()))
            .match_header("cookie", cookie("old"))
            .with_status(401)
            .expect_at_least(1)
            .create();
        let _context = lms
            .mock("GET", Matcher::Regex("^/api/webapp/context".into()))
            .match_header("cookie", cookie("new"))
            .with_header("content-type", "application/json")
            .with_body(
                json!({"UserInfo": {"UserId": 7}, "Groups": [
                    {"GroupName": "CHEMISTRY - 20", "SectionId": 10, "LeadSectionId": 10, "Category": "Science", "OwnerName": "Ms. T"}]})
                .to_string(),
            )
            .create();

        let renewals = Arc::new(AtomicUsize::new(0));
        let (cmd_tx, cmd_rx) = channel();
        let (ev_tx, ev_rx) = channel();
        spawn_with(Box::new(Lapsed { lms: lms.url(), renewals: renewals.clone() }), cmd_rx, ev_tx);
        for _ in 0..12 {
            cmd_tx.send(Cmd::Load(Key::Classes)).unwrap();
        }
        for _ in 0..12 {
            let ev = ev_rx.recv_timeout(Duration::from_secs(10)).expect("an answer");
            let Event::Loaded { key: Key::Classes, result: Ok(Payload::Classes(classes)) } = ev else { panic!("{ev:?}") };
            assert_eq!(classes[0].name, "CHEMISTRY - 20");
        }
        assert_eq!(renewals.load(Ordering::SeqCst), 1, "twelve readers hit the expired session; one renewed it");
        cmd_tx.send(Cmd::Quit).unwrap();
    }

    #[test]
    fn an_unshared_gradebook_is_reported_as_such_not_as_an_error() {
        let mut lms = Server::new();
        let _status = lms
            .mock("GET", Matcher::Regex("^/api/webapp/userstatus".into()))
            .with_header("content-type", "application/json")
            .with_body(r#"{"TokenValid": true}"#)
            .create();
        let _grades = lms.mock("GET", Matcher::Regex("^/api/datadirect/GradeBookMarkingPeriodList".into())).with_status(403).create();
        let _context = lms
            .mock("GET", Matcher::Regex("^/api/webapp/context".into()))
            .with_header("content-type", "application/json")
            .with_body(json!({"UserInfo": {"UserId": 7}, "Groups": []}).to_string())
            .create();
        let (cmd_tx, cmd_rx) = channel();
        let (ev_tx, ev_rx) = channel();
        spawn_with(Box::new(Lapsed { lms: lms.url(), renewals: Arc::new(AtomicUsize::new(0)) }), cmd_rx, ev_tx);
        // "old" is only refused by the first mock above, which is not installed here: any cookie works
        cmd_tx.send(Cmd::Load(Key::Grades(10))).unwrap();
        let ev = ev_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let Event::Loaded { result: Err(msg), .. } = ev else { panic!("{ev:?}") };
        assert_eq!(msg, GRADES_NOT_SHARED);
    }
}
