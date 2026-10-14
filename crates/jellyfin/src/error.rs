#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid server URL: {0}")]
    InvalidUrl(String),

    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The server is reachable but still starting up. Retry shortly.
    #[error("the server is still starting up")]
    ServerStarting,

    /// Wrong credentials on login, or an invalid / revoked token on any other request.
    #[error("not authorized")]
    Unauthorized,

    #[error("access denied")]
    Forbidden,

    #[error("not found")]
    NotFound,

    #[error("server returned {status}: {message}")]
    Status { status: u16, message: String },

    #[error("unexpected response at `{path}`: {source}")]
    Decode { path: String, source: serde_json::Error },
}

pub type Result<T> = std::result::Result<T, Error>;
