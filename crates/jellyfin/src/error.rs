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

impl Error {
    /// Whether the server refused a range request as past the end of its file (416). A resumed
    /// download gets it when its `.part` already holds everything, or more than the file now has.
    pub fn is_range_not_satisfiable(&self) -> bool {
        matches!(self, Error::Status { status: 416, .. })
    }

    /// Whether the request failed because the server's TLS certificate was refused (expired,
    /// self-signed, or for another name). Something answers TLS at that address, so it must not
    /// be tried again over plain HTTP.
    pub fn is_certificate(&self) -> bool {
        let Error::Http(err) = self else { return false };
        let mut source = Some(err as &(dyn std::error::Error + 'static));
        while let Some(err) = source {
            if matches!(tls_error(err), Some(rustls::Error::InvalidCertificate(_))) {
                return true;
            }
            source = err.source();
        }
        false
    }
}

/// rustls's error out of `err`, which is one itself or an `io::Error` carrying it. The handshake's
/// failure arrives as an `io::Error` inside another, and `source()` skips over what an `io::Error`
/// wraps, so each level is opened by hand.
fn tls_error<'a>(err: &'a (dyn std::error::Error + 'static)) -> Option<&'a rustls::Error> {
    if let Some(tls) = err.downcast_ref::<rustls::Error>() {
        return Some(tls);
    }
    let mut inner = err.downcast_ref::<std::io::Error>()?.get_ref()?;
    loop {
        if let Some(tls) = inner.downcast_ref::<rustls::Error>() {
            return Some(tls);
        }
        inner = inner.downcast_ref::<std::io::Error>()?.get_ref()?;
    }
}

pub type Result<T> = std::result::Result<T, Error>;
