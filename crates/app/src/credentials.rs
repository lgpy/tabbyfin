//! Access token storage. Uses the OS credential store (Keychain, Windows Credential Manager,
//! Secret Service). Where none is available, for example WSL or a minimal Linux session without
//! a Secret Service daemon, falls back to a file only the current user can read.
//!
//! All functions block, so call them off the main thread.

use std::path::PathBuf;

use anyhow::Context;
use keyring::Entry;

const SERVICE: &str = "jelly-gpui";

pub fn load(account: &str) -> Option<String> {
    match entry(account).map(|e| e.get_password()) {
        Some(Ok(token)) => return Some(token),
        Some(Err(keyring::Error::NoEntry)) => {}
        Some(Err(err)) => tracing::warn!("credential store unavailable, using file fallback: {err}"),
        None => {}
    }
    std::fs::read_to_string(fallback_path(account))
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
}

pub fn store(account: &str, token: &str) -> anyhow::Result<()> {
    match entry(account).map(|e| e.set_password(token)) {
        Some(Ok(())) => {
            // Don't leave an older plaintext copy behind.
            let _ = std::fs::remove_file(fallback_path(account));
            return Ok(());
        }
        Some(Err(err)) => tracing::warn!("credential store unavailable, using file fallback: {err}"),
        None => {}
    }
    write_private(&fallback_path(account), token)
}

pub fn delete(account: &str) {
    if let Some(entry) = entry(account) {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(err) => tracing::warn!("failed to delete stored token: {err}"),
        }
    }
    let _ = std::fs::remove_file(fallback_path(account));
}

fn entry(account: &str) -> Option<Entry> {
    match Entry::new(SERVICE, account) {
        Ok(entry) => Some(entry),
        Err(err) => {
            tracing::debug!("no credential store: {err}");
            None
        }
    }
}

fn fallback_path(account: &str) -> PathBuf {
    let file = account.replace(['/', '\\', ':'], "_");
    crate::config::dir().join("tokens").join(file)
}

fn write_private(path: &PathBuf, contents: &str) -> anyhow::Result<()> {
    let dir = path.parent().context("token path has no parent")?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, contents)?;
    Ok(())
}
