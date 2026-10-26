use std::ffi::{CStr, c_int};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::video::{RenderThread, VideoImport, VideoOutput};
use super::{Api, Core, FORMAT_DOUBLE, FORMAT_FLAG, FORMAT_INT64, RawEndFile, RawLogMessage, RawProperty, cstring};
use crate::events::{EventTranslator, OBSERVED_PROPERTIES};
use crate::{Error, PlayerEvent, Result};

const EVENT_SHUTDOWN: c_int = 1;
const EVENT_LOG_MESSAGE: c_int = 2;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_SEEK: c_int = 20;
const EVENT_PLAYBACK_RESTART: c_int = 21;
const EVENT_PROPERTY_CHANGE: c_int = 22;

/// An mpv core running inside this process, rendering into a [`VideoOutput`].
///
/// Methods block (libmpv calls are synchronous), so call them off the UI thread.
pub struct EmbeddedPlayer {
    core: Arc<Core>,
    video: VideoOutput,
    render_thread: Option<RenderThread>,
    stop: Arc<AtomicBool>,
    events: Option<JoinHandle<()>>,
}

impl EmbeddedPlayer {
    /// Starts mpv with a video output whose frames the UI can take (`import`).
    pub fn spawn(import: VideoImport) -> Result<(Self, mpsc::UnboundedReceiver<PlayerEvent>)> {
        let api = Api::load()?;
        // SAFETY: mpv_create has no preconditions.
        let mpv = unsafe { (api.create)() };
        if mpv.is_null() {
            return Err(Error::LibmpvUnavailable("mpv_create returned null".into()));
        }
        let core = Arc::new(Core { api, mpv });

        let set_option = |name: &str, value: &str| {
            let (name, value) = (cstring(name), cstring(value));
            // SAFETY: valid handle and C strings.
            unsafe { (core.api.set_option_string)(core.mpv, name.as_ptr(), value.as_ptr()) }
        };
        // The app draws the UI, so the user's mpv.conf, scripts (OSC replacements like uosc)
        // and key bindings stay out. Quit after the file ends.
        for (name, value) in [
            ("config", "no"),
            ("load-scripts", "no"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("idle", "once"),
            ("keep-open", "no"),
            // Ride out short network drops: ffmpeg reconnects and resumes the HTTP stream.
            (
                "stream-lavf-o",
                "reconnect=1,reconnect_streamed=1,reconnect_on_network_error=1,reconnect_delay_max=5",
            ),
        ] {
            set_option(name, value);
        }
        // Dev aid: `TABBYFIN_MPV_OPTIONS="demuxer-max-bytes=1MiB;cache=no"` sets extra options.
        #[cfg(debug_assertions)]
        for option in std::env::var("TABBYFIN_MPV_OPTIONS").unwrap_or_default().split(';') {
            if let Some((name, value)) = option.split_once('=') {
                set_option(name.trim(), value.trim());
            }
        }
        // The software renderer reads frames back from the CPU.
        set_option("hwdec", "auto-copy-safe");
        set_option("vo", "libmpv");
        // SAFETY: valid handle, options set.
        core.api
            .check("mpv_initialize", unsafe { (core.api.initialize)(core.mpv) })?;
        // mpv's own log, forwarded to ours. `TABBYFIN_MPV_LOG` picks the level (e.g. `v`).
        let level = cstring(&std::env::var("TABBYFIN_MPV_LOG").unwrap_or_else(|_| "warn".into()));
        // SAFETY: valid handle and C string.
        unsafe { (core.api.request_log_messages)(core.mpv, level.as_ptr()) };

        let formats = [FORMAT_DOUBLE, FORMAT_DOUBLE, FORMAT_FLAG, FORMAT_DOUBLE, FORMAT_FLAG];
        for (id, (name, format)) in OBSERVED_PROPERTIES.iter().zip(formats).enumerate() {
            let name = cstring(name);
            // SAFETY: valid handle and C string.
            let code = unsafe { (core.api.observe_property)(core.mpv, id as u64, name.as_ptr(), format) };
            core.api.check("observe_property", code)?;
        }

        // The render context must exist before a file with video starts.
        let (video, render_thread) = VideoOutput::new(core.clone(), import)?;

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let events = std::thread::Builder::new()
            .name("mpv-events".into())
            .spawn({
                let (core, stop) = (core.clone(), stop.clone());
                move || event_loop(&core, &stop, &events_tx)
            })
            .map_err(Error::Spawn)?;

        Ok((
            Self {
                core,
                video,
                render_thread: Some(render_thread),
                stop,
                events: Some(events),
            },
            events_rx,
        ))
    }

    /// Where video frames come from. Clone it into the UI.
    pub fn video(&self) -> VideoOutput {
        self.video.clone()
    }

    /// Runs an mpv command. JSON values are converted the way the IPC protocol would.
    pub fn command(&self, args: &[Value]) -> Result<()> {
        let strings: Vec<String> = args.iter().map(value_string).collect();
        let refs: Vec<&str> = strings.iter().map(String::as_str).collect();
        self.core.command(&refs)
    }

    pub fn set_property(&self, name: &str, value: &Value) -> Result<()> {
        match value {
            // String lists (e.g. http-header-fields): replace the whole list without splitting
            // entries on commas, which the string form would do.
            Value::Array(items) => {
                self.core.command(&["change-list", name, "clr", ""])?;
                for item in items {
                    self.core
                        .command(&["change-list", name, "append", &value_string(item)])?;
                }
                Ok(())
            }
            value => self.core.set_property(name, &value_string(value)),
        }
    }
}

impl Drop for EmbeddedPlayer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.core.command(&["quit"]);
        // SAFETY: valid handle; wakes a blocked mpv_wait_event.
        unsafe { (self.core.api.wakeup)(self.core.mpv) };
        if let Some(events) = self.events.take() {
            let _ = events.join();
        }
        // Stop rendering; the render context itself is freed with the last `VideoOutput`,
        // before the core (see `Renderer::drop`).
        self.render_thread.take();
    }
}

fn value_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "yes" } else { "no" }.into(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Turns libmpv events into the same JSON the IPC protocol uses, so both backends share
/// [`EventTranslator`].
fn event_loop(core: &Core, stop: &AtomicBool, events: &mpsc::UnboundedSender<PlayerEvent>) {
    let mut translator = EventTranslator::default();
    let mut decoder_logged = false;
    while !stop.load(Ordering::Relaxed) {
        // SAFETY: mpv_wait_event returns a valid event, owned by mpv until the next call.
        let event = unsafe { &*(core.api.wait_event)(core.mpv, 1.0) };
        let message = match event.event_id {
            EVENT_SHUTDOWN => break,
            EVENT_LOG_MESSAGE => {
                // SAFETY: LOG_MESSAGE events carry an mpv_event_log_message with C strings.
                let (prefix, text, level) = unsafe {
                    let message = &*(event.data as *const RawLogMessage);
                    (
                        CStr::from_ptr(message.prefix),
                        CStr::from_ptr(message.text),
                        message.log_level,
                    )
                };
                let (prefix, text) = (prefix.to_string_lossy(), text.to_string_lossy());
                let text = text.trim_end();
                match level {
                    // fatal, error, warn
                    i32::MIN..=30 => tracing::warn!(target: "mpv", "[{prefix}] {text}"),
                    31..=40 => tracing::info!(target: "mpv", "[{prefix}] {text}"),
                    _ => tracing::debug!(target: "mpv", "[{prefix}] {text}"),
                }
                continue;
            }
            EVENT_FILE_LOADED => {
                decoder_logged = false;
                json!({ "event": "file-loaded" })
            }
            EVENT_SEEK => json!({ "event": "seek" }),
            EVENT_PLAYBACK_RESTART => {
                if !decoder_logged {
                    decoder_logged = true;
                    let hwdec = core.get_string("hwdec-current").filter(|h| !h.is_empty() && h != "no");
                    let codec = core.get_string("video-codec").unwrap_or_default();
                    tracing::info!(%codec, "video decoding: {}", hwdec.as_deref().unwrap_or("software"));
                }
                json!({ "event": "playback-restart" })
            }
            EVENT_END_FILE => {
                // SAFETY: END_FILE events carry an mpv_event_end_file.
                let end = unsafe { &*(event.data as *const RawEndFile) };
                let reason = match end.reason {
                    0 => "eof",
                    3 => "quit",
                    4 => "error",
                    5 => "redirect",
                    _ => "stop",
                };
                json!({ "event": "end-file", "reason": reason, "file_error": core.api.error_message(end.error) })
            }
            EVENT_PROPERTY_CHANGE => {
                // SAFETY: PROPERTY_CHANGE events carry an mpv_event_property.
                let property = unsafe { &*(event.data as *const RawProperty) };
                // SAFETY: mpv passes a valid property name.
                let name = unsafe { CStr::from_ptr(property.name) }.to_string_lossy();
                // SAFETY: `data` points to a value of the reported format (or is unused).
                let data = unsafe {
                    match property.format {
                        FORMAT_DOUBLE => json!(*(property.data as *const f64)),
                        FORMAT_FLAG => json!(*(property.data as *const c_int) != 0),
                        FORMAT_INT64 => json!(*(property.data as *const i64)),
                        _ => Value::Null,
                    }
                };
                if data.is_null() {
                    json!({ "event": "property-change", "name": name })
                } else {
                    json!({ "event": "property-change", "name": name, "data": data })
                }
            }
            _ => continue,
        };
        if let Some(event) = translator.translate(&message) {
            let _ = events.send(event);
        }
    }
    let _ = events.send(PlayerEvent::Exited);
}
