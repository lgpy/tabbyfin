//! The update notice: on launch, at most once a day, asks GitHub for the latest release and
//! reports it when it's newer than this build. Settings can turn the check off.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::config::Config;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/lgpy/tabbyfin/releases/latest";
/// GitHub allows 60 unauthenticated requests an hour; once a day is plenty.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// A release newer than this build.
pub struct Update {
    pub version: String,
    /// The release page.
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Checks for a newer release if the setting allows and a day has passed since the last
/// check. Any failure is logged and reads as no update. Runs on tokio.
pub async fn check() -> Option<Update> {
    // Dev aid: `TABBYFIN_UPDATE_URL` points the check elsewhere and checks every launch.
    let override_url = cfg!(debug_assertions)
        .then(|| std::env::var("TABBYFIN_UPDATE_URL").ok())
        .flatten();
    let mut config = Config::load();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if !config.check_for_updates.unwrap_or(true) {
        return None;
    }
    let due = config
        .update_checked_at
        .is_none_or(|at| now.saturating_sub(at) >= CHECK_EVERY.as_secs());
    if !due && override_url.is_none() {
        return None;
    }
    config.update_checked_at = Some(now);
    if let Err(err) = config.save() {
        tracing::warn!("failed to save config: {err:#}");
    }

    let url = override_url.as_deref().unwrap_or(LATEST_RELEASE_URL);
    let release = match fetch(url).await {
        Ok(release) => release,
        Err(err) => {
            tracing::info!("update check failed: {err:#}");
            return None;
        }
    };
    let current = env!("CARGO_PKG_VERSION");
    tracing::info!(latest = %release.tag_name, current, "checked for updates");
    (!release.draft && !release.prerelease && is_newer(&release.tag_name, current)).then(|| Update {
        version: release.tag_name.trim_start_matches('v').to_owned(),
        url: release.html_url,
    })
}

async fn fetch(url: &str) -> anyhow::Result<Release> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        // GitHub's API refuses requests without one.
        .user_agent(concat!("tabbyfin/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = http
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

/// Whether release `tag` (e.g. `v0.2.0`) is newer than `current` (e.g. `0.1.0`). A pre-release
/// (`0.2.0-rc.1`) is older than its release. Tags that aren't versions are never newer.
fn is_newer(tag: &str, current: &str) -> bool {
    match (parse(tag), parse(current)) {
        (Some(tag), Some(current)) => tag > current,
        _ => false,
    }
}

/// `(major, minor, patch, is_release)`, ordered so that a release follows its pre-releases.
fn parse(version: &str) -> Option<(u64, u64, u64, bool)> {
    let version = version.trim().trim_start_matches('v');
    let (numbers, pre) = match version.split_once('-') {
        Some((numbers, pre)) => (numbers, Some(pre)),
        None => (version, None),
    };
    let mut parts = numbers.split('.').map(|part| part.parse::<u64>().ok());
    let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next().unwrap_or(Some(0))?);
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, pre.is_none()))
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn versions_compare_like_semver() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.1", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
        assert!(is_newer("v0.2.0", "0.2.0-rc.1"));
        assert!(!is_newer("v0.2.0-rc.1", "0.2.0"));
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }
}
