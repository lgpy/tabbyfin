use jellyfin::models::{PublicSystemInfo, UserDto};
use jellyfin::{Client, ClientInfo};

/// A signed-in user on a server.
#[derive(Clone)]
pub struct Session {
    /// Authenticated client.
    pub client: Client,
    pub user: UserDto,
    pub server: PublicSystemInfo,
}

pub fn client_info(device_id: &str) -> ClientInfo {
    let device = gethostname::gethostname().to_string_lossy().into_owned();
    ClientInfo {
        client: "jelly-gpui".into(),
        device: if device.is_empty() { "Desktop".into() } else { device },
        device_id: device_id.to_owned(),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

/// User-facing message for an API error.
pub fn describe_error(err: &jellyfin::Error) -> String {
    use jellyfin::Error;
    match err {
        Error::InvalidUrl(_) => "That doesn't look like a valid server address.".into(),
        Error::Http(e) if e.is_timeout() => "The server took too long to respond.".into(),
        Error::Http(e) if e.is_connect() => "Couldn't reach the server. Check the address and your connection.".into(),
        Error::Http(_) => "The connection to the server failed.".into(),
        Error::ServerStarting => "The server is still starting up.".into(),
        Error::Unauthorized => "Your session has expired. Please sign in again.".into(),
        Error::Forbidden => "This account isn't allowed to do that.".into(),
        Error::NotFound => "The server couldn't find what was requested.".into(),
        Error::Decode { .. } => "The server sent a response this app doesn't understand.".into(),
        Error::Socket(_) => "The live connection to the server failed.".into(),
        Error::Status { status, .. } => format!("The server returned an error ({status})."),
    }
}
