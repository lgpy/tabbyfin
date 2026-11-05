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

    /// A download went [`crate::download::STALL_TIMEOUT`] without a byte: the connection is
    /// gone, however long the file still has to go.
    #[error("the download stalled")]
    Stalled,

    /// The session websocket failed to open or broke.
    #[error("websocket: {0}")]
    Socket(String),

    #[error("unexpected response at `{path}`: {source}")]
    Decode { path: String, source: serde_json::Error },
}

pub type Result<T> = std::result::Result<T, Error>;
