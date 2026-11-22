use jellyfin::models::{PublicSystemInfo, UserDto};
use jellyfin::{Client, ClientInfo};

use crate::i18n::tr;

/// A signed-in user on a server.
#[derive(Clone)]
pub struct Session {
    /// Authenticated client.
    pub client: Client,
    pub user: UserDto,
    pub server: PublicSystemInfo,
}

impl Session {
    /// This user on this server, as [`crate::config::SavedServer::credential_account`].
    pub fn account_key(&self) -> String {
        crate::config::account_key(self.server.id.as_deref().unwrap_or_default(), &self.user.id)
    }
}

pub fn client_info(device_id: &str) -> ClientInfo {
    #[cfg(not(target_os = "android"))]
    let device = gethostname::gethostname().to_string_lossy().into_owned();
    #[cfg(target_os = "android")]
    let device = crate::android::device_name().unwrap_or_default();
    ClientInfo {
        client: "Tabbyfin".into(),
        device: if device.is_empty() {
            if cfg!(target_os = "android") {
                "Android"
            } else {
                "Desktop"
            }
            .into()
        } else {
            device
        },
        device_id: device_id.to_owned(),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

/// User-facing message for an API error.
pub fn describe_error(err: &jellyfin::Error) -> String {
    use jellyfin::Error;
    match err {
        Error::InvalidUrl(_) => tr!("shell-error-invalid-url"),
        Error::Http(e) if e.is_timeout() => tr!("shell-error-timeout"),
        Error::Http(e) if e.is_connect() => tr!("shell-error-connect"),
        Error::Http(_) => tr!("shell-error-broken"),
        Error::ServerStarting => tr!("shell-error-starting"),
        Error::Unauthorized => tr!("shell-error-expired"),
        Error::Forbidden => tr!("shell-error-forbidden"),
        Error::NotFound => tr!("shell-error-not-found"),
        Error::Decode { .. } => tr!("shell-error-decode"),
        Error::Stalled => tr!("shell-error-stalled"),
        Error::Socket(_) => tr!("shell-error-socket"),
        Error::Status { status, .. } => tr!("shell-error-status", status = status.to_string()),
    }
}
