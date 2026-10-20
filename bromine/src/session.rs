//! The saved session, saved at (`~/.bromine-mcp/session.json`), plus the headless
//! refresh and the interactive browser login that produce it.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::auth::{self, Credentials, Endpoints, LoginAttempt};
use crate::capture::{self, CaptureListener};
use crate::error::{Error, Result};

#[derive(Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub username: String,
    pub tt: String,
    pub credentials: Credentials,
}

impl std::fmt::Debug for SavedSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedSession").field("username", &self.username).field("tt", &"<redacted>").finish_non_exhaustive()
    }
}

/// Where the session lives on disk.
#[derive(Debug, Clone)]
pub struct SessionStore {
    path: PathBuf,
    username: Option<String>,
}

impl Default for SessionStore {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self { path: home.join(".bromine-mcp").join("session.json"), username: None }
    }
}

impl SessionStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), username: None }
    }

    /// Use this username for sign-ins (overrides `$BB_USERNAME` and the saved session).
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `Ok(None)` when there is no (readable) session yet.
    pub fn load(&self) -> Result<Option<SavedSession>> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(serde_json::from_str(&text).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Atomic write with 0600 perms in a 0700 directory: these are live session secrets.
    pub fn save(&self, session: &SavedSession) -> Result<()> {
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        let tmp = self.path.with_extension("tmp");
        let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        file.write_all(serde_json::to_string(session).map_err(|e| Error::Auth(e.to_string()))?.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// The explicit username, else `$BB_USERNAME`, else the saved session's user. `None` means the
    /// caller has to ask the person.
    pub fn username(&self) -> Option<String> {
        self.username
            .clone()
            .or_else(|| std::env::var("BB_USERNAME").ok().filter(|s| !s.is_empty()))
            .or_else(|| self.load().ok().flatten().map(|s| s.username))
    }

    /// Mint a fresh `t` from the saved credentials with no browser, persisting it and any rotated
    /// credentials. [`Error::LoginRequired`] if there is no session or it has fully expired.
    pub fn refresh(&self, ep: &Endpoints) -> Result<SavedSession> {
        let saved = self.load()?.ok_or_else(|| Error::LoginRequired("no saved session".into()))?;
        let (tt, credentials) = auth::mint_tt(ep, &saved.credentials)?;
        let fresh = SavedSession { username: saved.username, tt, credentials };
        self.save(&fresh)?;
        Ok(fresh)
    }

    /// Sign in through the browser and save the session. Blocks until the callback arrives.
    ///
    /// `open_url` receives the sign-in URL once the listener is bound (open it, or show it to the user).
    /// Needs `capture.user.js` installed: it POSTs the B2C callback URL to the listener.
    pub fn interactive_login(&self, ep: &Endpoints, open_url: impl FnOnce(&str), timeout: Duration) -> Result<SavedSession> {
        self.interactive_login_on(ep, capture::PORT, open_url, timeout)
    }

    pub fn interactive_login_on(&self, ep: &Endpoints, port: u16, open_url: impl FnOnce(&str), timeout: Duration) -> Result<SavedSession> {
        let username =
            self.username().ok_or_else(|| Error::Invalid("no username: set BB_USERNAME or use SessionStore::with_username".into()))?;
        let listener = CaptureListener::bind(port)?;
        let (attempt, url) = LoginAttempt::start(ep, &username)?;
        open_url(&url);
        let callback = listener.wait(timeout)?;
        let credentials = attempt.finish(&callback)?;
        let (tt, credentials) = auth::mint_tt(ep, &credentials)?;
        let session = SavedSession { username, tt, credentials };
        self.save(&session)?;
        Ok(session)
    }
}

/// Open `url` in the default browser (`xdg-open` on Linux, `open` on macOS); `false` if it could not be
/// launched. Output is discarded so it cannot scribble over a terminal UI.
pub fn open_in_browser(url: &str) -> bool {
    open::that_detached(url).is_ok()
}
