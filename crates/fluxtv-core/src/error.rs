/// Errors produced by the FluxTV core.
///
/// Network errors are stored as strings with the request URL stripped, because
/// IPTV URLs embed the user's credentials and error messages end up in logs and UI.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(String),
    #[error("server returned HTTP {0}")]
    Status(u16),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("invalid data: {0}")]
    Parse(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        if let Some(status) = e.status() {
            return Error::Status(status.as_u16());
        }
        let e = e.without_url();
        let mut msg = e.to_string();
        // Include the root cause (e.g. "connection refused"), which reqwest hides behind "error sending request".
        let mut source = std::error::Error::source(&e);
        while let Some(s) = source {
            msg = format!("{msg}: {s}");
            source = s.source();
        }
        Error::Http(msg)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Parse(format!("json: {e}"))
    }
}

impl From<quick_xml::Error> for Error {
    fn from(e: quick_xml::Error) -> Self {
        Error::Parse(format!("xml: {e}"))
    }
}
