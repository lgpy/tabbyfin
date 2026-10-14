use std::fmt;

use crate::client::Query;
use crate::models::PublicSystemInfo;
use crate::{Client, Error, Result};

/// Oldest server version the client supports. See `docs/PLAN.md` §4.4.
pub const MIN_SUPPORTED_VERSION: ServerVersion = ServerVersion {
    major: 10,
    minor: 11,
    patch: 0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl ServerVersion {
    /// Parses `12.2.0`, `10.11`, or `12.0.0-rc1` style versions.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().split(['.', '-']).map(|p| p.parse::<u32>());
        let major = parts.next()?.ok()?;
        let minor = parts.next().and_then(|r| r.ok()).unwrap_or(0);
        let patch = parts.next().and_then(|r| r.ok()).unwrap_or(0);
        Some(Self { major, minor, patch })
    }

    pub fn is_supported(self) -> bool {
        self >= MIN_SUPPORTED_VERSION
    }
}

impl fmt::Display for ServerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl PublicSystemInfo {
    pub fn server_version(&self) -> Option<ServerVersion> {
        self.version.as_deref().and_then(ServerVersion::parse)
    }
}

impl Client {
    /// Probes the server. Doesn't need authentication.
    ///
    /// Returns [`Error::ServerStarting`] while the server is booting: it first serves a
    /// camelCase placeholder (12.x startup UI) and then `503 Jellyfin Server is loading`.
    pub async fn public_info(&self) -> Result<PublicSystemInfo> {
        let info: PublicSystemInfo = self.get("/System/Info/Public", &Query::new()).await?;
        if info.startup_wizard_completed.is_none() {
            return Err(Error::ServerStarting);
        }
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing_and_support() {
        let v = |s| ServerVersion::parse(s).unwrap();
        assert_eq!(
            v("12.2.0"),
            ServerVersion {
                major: 12,
                minor: 2,
                patch: 0
            }
        );
        assert_eq!(
            v("10.11"),
            ServerVersion {
                major: 10,
                minor: 11,
                patch: 0
            }
        );
        assert_eq!(
            v("12.0.0-rc1"),
            ServerVersion {
                major: 12,
                minor: 0,
                patch: 0
            }
        );
        assert!(ServerVersion::parse("abc").is_none());
        assert!(v("10.11.0").is_supported());
        assert!(v("12.2.0").is_supported());
        assert!(!v("10.10.7").is_supported());
    }
}
