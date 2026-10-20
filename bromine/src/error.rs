//! One error type for the whole library, split by what a caller can do about it.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// There is no usable session at all: a human has to sign in (see [`crate::session::interactive_login`]).
    #[error("sign-in required: {0}")]
    LoginRequired(String),

    /// The LMS rejected the `t` cookie. Re-mint it ([`crate::session::refresh`]) and retry once.
    #[error("the LMS session expired")]
    SessionExpired,

    /// A step of the login handshake failed for a reason other than "not signed in".
    #[error("authentication failed: {0}")]
    Auth(String),

    /// The request was refused before it was sent (bad input, or a safety rule of this library).
    #[error("{0}")]
    Invalid(String),

    /// A genuine refusal: the session is fine but the LMS said no (for example grades not published).
    #[error("the LMS refused the request: HTTP {status} for {method} {path}")]
    Refused { status: u16, method: String, path: String },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("could not read the LMS response for {what}: {source}")]
    Parse { what: &'static str, source: serde_json::Error },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Is this the "sign in again" class of failure?
    pub fn needs_login(&self) -> bool {
        matches!(self, Error::LoginRequired(_))
    }
}
