//! Client library for a Blackbaud LMS (built for Hunter Schools): sign-in, session storage, and a typed
//! API for classes, assignments, tasks, files and grades.
//!
//! ```no_run
//! use bromine::{Client, Endpoints, SessionStore};
//!
//! let store = SessionStore::default();
//! let session = store.refresh(&Endpoints::default())?;   // headless; Err(LoginRequired) if expired
//! let client = Client::new(&session.tt)?;
//! for item in client.assignment_center()? {
//!     println!("{:?} {} due {:?}", item.status, item.name, item.due);
//! }
//! # Ok::<(), bromine::Error>(())
//! ```
//!
//! The session file is saved at (`~/.bromine-mcp/session.json`).

pub mod auth;
pub mod capture;
pub mod client;
pub mod cookies;
pub mod error;
pub mod models;
pub mod session;
pub mod text;

mod json;

pub use auth::{Credentials, Endpoints};
pub use client::{Client, Download, TaskPatch, sanitize_filename};
pub use error::{Error, Result};
pub use models::*;
pub use session::{SavedSession, SessionStore, open_in_browser};
