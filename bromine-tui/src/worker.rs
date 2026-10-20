//! The network thread. The UI sends [`Cmd`]s and receives [`Event`]s, so it never blocks, and sign-in
//! (refresh, then the browser when that is not enough) is handled here, transparently.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use bromine::{
    Announcement, AssignmentDetail, AssignmentRow, CenterItem, Class, ClassFile, ClassInfo, Client, Endpoints, Error, FeedItem, Grades,
    Person, SessionStore, Status, TaskPatch, Topic, TopicItem, UserFile, open_in_browser,
};
use chrono::NaiveDateTime;
use std::path::PathBuf;

use crate::files;

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
    Center,
    Classes,
    Feed,
    Detail(i64),
    ClassInfo { section_id: i64 },
    ClassAssignments { lead_section_id: i64 },
    SetStatus { id: i64, status: Status, task: bool },
    CreateTask { description: String, due: NaiveDateTime },
    DeleteTask(i64),
    EditTask { id: i64, patch: TaskPatch },
    Topics { section_id: i64 },
    TopicItems(Topic),
    News { section_id: i64 },
    ClassFiles { section_id: i64 },
    Grades { section_id: i64 },
    People { section_id: i64 },
    UserFiles,
    Fetch { path: String, name: String, action: FileAction },
    Quit,
}

#[derive(Debug, Clone)]
pub enum Event {
    Center(Result<Vec<CenterItem>, String>),
    Classes(Result<Vec<Class>, String>),
    Feed(Result<Vec<FeedItem>, String>),
    Detail {
        id: i64,
        result: Result<AssignmentDetail, String>,
    },
    ClassInfo {
        section_id: i64,
        result: Result<ClassInfo, String>,
    },
    ClassAssignments {
        lead_section_id: i64,
        result: Result<Vec<AssignmentRow>, String>,
    },
    /// `previous` lets the UI undo its optimistic update on failure.
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
    Topics {
        section_id: i64,
        result: Result<Vec<Topic>, String>,
    },
    TopicItems {
        topic_id: i64,
        result: Result<Vec<TopicItem>, String>,
    },
    News {
        section_id: i64,
        result: Result<Vec<Announcement>, String>,
    },
    ClassFiles {
        section_id: i64,
        result: Result<Vec<ClassFile>, String>,
    },
    Grades {
        section_id: i64,
        result: Result<Vec<Grades>, String>,
    },
    People {
        section_id: i64,
        result: Result<Vec<Person>, String>,
    },
    UserFiles(Result<Vec<UserFile>, String>),
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

pub fn spawn(store: SessionStore, ep: Endpoints, rx: Receiver<Cmd>, tx: Sender<Event>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("bromine-worker".into())
        .spawn(move || {
            let mut w = Worker { store, ep, client: None, tx };
            while let Ok(cmd) = rx.recv() {
                if matches!(cmd, Cmd::Quit) {
                    break;
                }
                w.run(cmd);
            }
        })
        .expect("spawn worker thread")
}

struct Worker {
    store: SessionStore,
    ep: Endpoints,
    client: Option<Arc<Client>>,
    tx: Sender<Event>,
}

fn msg(e: Error) -> String {
    e.to_string()
}

impl Worker {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e); // the UI is gone: nothing left to tell
    }

    /// Run `f` with a working client. A lapsed session is refreshed (and, failing that, signed in again
    /// through the browser) and `f` is retried once.
    fn call<T>(&mut self, f: impl Fn(&Client) -> Result<T, Error>) -> Result<T, Error> {
        for attempt in 0..2 {
            let client = match self.client.clone() {
                Some(c) => c,
                None => self.connect()?,
            };
            match f(&client) {
                Err(Error::SessionExpired) if attempt == 0 => {
                    self.client = None;
                    self.reauth()?;
                }
                other => return other,
            }
        }
        Err(Error::SessionExpired)
    }

    fn connect(&mut self) -> Result<Arc<Client>, Error> {
        if let Some(saved) = self.store.load()? {
            let c = Arc::new(Client::with_endpoints(self.ep.clone(), &saved.tt)?);
            self.client = Some(c.clone());
            return Ok(c);
        }
        self.reauth()
    }

    fn reauth(&mut self) -> Result<Arc<Client>, Error> {
        let session = match self.store.refresh(&self.ep) {
            Ok(s) => s,
            Err(Error::LoginRequired(_)) => {
                self.send(Event::LoginNeeded);
                let tx = self.tx.clone();
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
                        self.send(Event::LoginDone);
                        s
                    }
                    Err(e) => {
                        self.send(Event::LoginFailed(e.to_string()));
                        return Err(e);
                    }
                }
            }
            Err(e) => return Err(e),
        };
        let c = Arc::new(Client::with_endpoints(self.ep.clone(), &session.tt)?);
        self.client = Some(c.clone());
        Ok(c)
    }

    fn run(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Center => {
                let r = self.call(|c| c.assignment_center()).map_err(msg);
                self.send(Event::Center(r));
            }
            Cmd::Classes => {
                let r = self.call(|c| c.classes()).map_err(msg);
                self.send(Event::Classes(r));
            }
            Cmd::Feed => {
                let r = self.call(|c| c.activity_feed()).map_err(msg);
                self.send(Event::Feed(r));
            }
            Cmd::Detail(id) => {
                let r = self.call(|c| c.assignment(id)).map_err(msg);
                self.send(Event::Detail { id, result: r });
            }
            Cmd::ClassInfo { section_id } => {
                let r = self.call(|c| c.class_info(section_id)).map_err(msg);
                self.send(Event::ClassInfo { section_id, result: r });
            }
            Cmd::ClassAssignments { lead_section_id } => {
                let r = self.call(|c| c.class_assignments(lead_section_id, 1)).map_err(msg);
                self.send(Event::ClassAssignments { lead_section_id, result: r });
            }
            Cmd::SetStatus { id, status, task } => {
                let r = self
                    .call(|c| {
                        if task {
                            c.update_task(id, &TaskPatch { status: Some(status), ..Default::default() })
                        } else {
                            c.set_status(id, status)
                        }
                    })
                    .map_err(msg);
                self.send(Event::StatusSet { id, status, task, result: r });
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
            Cmd::Topics { section_id } => {
                let r = self.call(|c| c.topics(section_id)).map_err(msg);
                self.send(Event::Topics { section_id, result: r });
            }
            Cmd::TopicItems(topic) => {
                let r = self.call(|c| c.topic_items(&topic)).map_err(msg);
                self.send(Event::TopicItems { topic_id: topic.topic_id, result: r });
            }
            Cmd::News { section_id } => {
                let r = self.call(|c| c.announcements(section_id)).map_err(msg);
                self.send(Event::News { section_id, result: r });
            }
            Cmd::ClassFiles { section_id } => {
                let r = self.call(|c| c.class_files(section_id)).map_err(msg);
                self.send(Event::ClassFiles { section_id, result: r });
            }
            Cmd::Grades { section_id } => {
                let r = match self.call(|c| c.grades(section_id)) {
                    Err(Error::Refused { .. }) => Err(GRADES_NOT_SHARED.to_owned()),
                    other => other.map_err(msg),
                };
                self.send(Event::Grades { section_id, result: r });
            }
            Cmd::People { section_id } => {
                let r = self.call(|c| c.roster(section_id)).map_err(msg);
                self.send(Event::People { section_id, result: r });
            }
            Cmd::UserFiles => {
                let r = self.call(|c| c.user_files()).map_err(msg);
                self.send(Event::UserFiles(r));
            }
            Cmd::Fetch { path, name, action } => {
                let result = self.fetch(&path, action);
                let name = result.as_ref().ok().and_then(|p| p.file_name()).map_or(name, |n| n.to_string_lossy().into_owned());
                self.send(Event::Fetched { name, action, result });
            }
            Cmd::Quit => {}
        }
    }

    fn fetch(&mut self, path: &str, action: FileAction) -> Result<PathBuf, String> {
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
