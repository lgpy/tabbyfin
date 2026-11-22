//! Keeping the display awake: the XDG desktop portal's `Inhibit` (the only way in a Flatpak,
//! and what GNOME and KDE answer), or `org.freedesktop.ScreenSaver.Inhibit` where there's no
//! portal for it. Both end when the connection does, so a crash can't leave the display held.

use std::collections::HashMap;

use tokio::sync::mpsc;
use zbus::zvariant::{OwnedObjectPath, Value};
use zbus::{Connection, Proxy};

use super::super::Inhibit;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_INHIBIT: &str = "org.freedesktop.portal.Inhibit";
const SCREENSAVER: &str = "org.freedesktop.ScreenSaver";
const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
/// The portal's flag for "idle": no screen blanking or locking.
const INHIBIT_IDLE: u32 = 8;
const REASON: &str = "Playing video";

/// Asks the task that owns the bus connection to hold the inhibition or let go.
pub struct Inhibitor {
    wanted: Option<mpsc::UnboundedSender<bool>>,
}

impl Inhibitor {
    pub fn new() -> Self {
        Self { wanted: None }
    }

    fn send(&mut self, wanted: bool) {
        // The task starts with the first request, so a session that never plays video never
        // opens the bus for this.
        let sender = self.wanted.get_or_insert_with(|| {
            let (sender, receiver) = mpsc::unbounded_channel();
            crate::runtime::detach(hold(receiver));
            sender
        });
        let _ = sender.send(wanted);
    }
}

impl Inhibit for Inhibitor {
    fn acquire(&mut self) {
        self.send(true);
    }

    fn release(&mut self) {
        self.send(false);
    }
}

enum Held {
    Portal(OwnedObjectPath),
    ScreenSaver(u32),
}

/// Applies the requests in order until the [`Inhibitor`] is dropped, which drops the
/// connection and with it whatever is still held.
async fn hold(mut wanted: mpsc::UnboundedReceiver<bool>) {
    let connection = match Connection::session().await {
        Ok(connection) => connection,
        Err(err) => {
            tracing::info!("no session bus, so the display isn't kept awake: {err}");
            return;
        }
    };
    let mut held: Option<Held> = None;
    while let Some(want) = wanted.recv().await {
        match (want, held.take()) {
            (true, None) => held = inhibit(&connection).await,
            (true, Some(current)) => held = Some(current),
            (false, Some(current)) => release(&connection, current).await,
            (false, None) => {}
        }
    }
}

async fn inhibit(connection: &Connection) -> Option<Held> {
    match inhibit_with_portal(connection).await {
        Ok(path) => return Some(Held::Portal(path)),
        Err(err) => tracing::debug!("portal inhibit: {err}"),
    }
    match inhibit_with_screensaver(connection).await {
        Ok(cookie) => Some(Held::ScreenSaver(cookie)),
        Err(err) => {
            tracing::info!("couldn't keep the display awake: {err}");
            None
        }
    }
}

async fn inhibit_with_portal(connection: &Connection) -> zbus::Result<OwnedObjectPath> {
    let proxy = Proxy::new(connection, PORTAL, PORTAL_PATH, PORTAL_INHIBIT).await?;
    let options: HashMap<&str, Value<'_>> = HashMap::from([("reason", Value::from(REASON))]);
    // The window identifier is empty: the portal only shows it in its own prompts.
    proxy.call("Inhibit", &("", INHIBIT_IDLE, options)).await
}

async fn inhibit_with_screensaver(connection: &Connection) -> zbus::Result<u32> {
    let proxy = Proxy::new(connection, SCREENSAVER, SCREENSAVER_PATH, SCREENSAVER).await?;
    proxy.call("Inhibit", &("Tabbyfin", REASON)).await
}

async fn release(connection: &Connection, held: Held) {
    let result: zbus::Result<()> = match held {
        Held::Portal(path) => {
            match Proxy::new(connection, PORTAL, path.as_str(), "org.freedesktop.portal.Request").await {
                Ok(proxy) => proxy.call("Close", &()).await,
                Err(err) => Err(err),
            }
        }
        Held::ScreenSaver(cookie) => match Proxy::new(connection, SCREENSAVER, SCREENSAVER_PATH, SCREENSAVER).await {
            Ok(proxy) => proxy.call("UnInhibit", &(cookie,)).await,
            Err(err) => Err(err),
        },
    };
    if let Err(err) = result {
        tracing::debug!("letting the display sleep: {err}");
    }
}
