//! Linux: the display is kept awake through the XDG portal or `org.freedesktop.ScreenSaver`
//! (`inhibit.rs`), and the media controls are an MPRIS service on the session bus
//! (`mpris.rs`). Both are served by `zbus` on the app's tokio runtime, never the UI thread.

mod inhibit;
mod mpris;

pub use inhibit::Inhibitor;

use super::{Controls, Link};

pub fn controls(_frame: &eframe::Frame, link: &Link) -> Option<Box<dyn Controls>> {
    Some(Box::new(mpris::Mpris::start(Link {
        commands: link.commands.clone(),
        ctx: link.ctx.clone(),
    })))
}
