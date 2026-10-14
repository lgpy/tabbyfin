use std::time::Duration;

use serde_json::Value;

/// Properties observed on startup. The index is the observe id.
pub(crate) const OBSERVED_PROPERTIES: &[&str] = &["time-pos", "duration", "pause", "volume", "mute"];

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    FileLoaded,
    /// Current position, sent at most about once a second, plus right after seeks.
    Position(Duration),
    Duration(Duration),
    Paused(bool),
    /// Volume in percent (100 is unchanged).
    Volume(f64),
    Muted(bool),
    Seeked,
    EndFile(EndReason),
    /// mpv exited or closed the connection. Always the last event.
    Exited,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EndReason {
    /// Played to the end.
    Eof,
    /// Stopped by the user or a new file.
    Stopped,
    Quit,
    Error(String),
}

/// Turns mpv events (in the shape of mpv's JSON IPC events) into [`PlayerEvent`]s, dropping
/// position updates that are less than a second apart so consumers don't wake up on every frame.
#[derive(Default)]
pub(crate) struct EventTranslator {
    last_position: Option<f64>,
}

impl EventTranslator {
    pub fn translate(&mut self, event: &Value) -> Option<PlayerEvent> {
        match event.get("event")?.as_str()? {
            "file-loaded" => Some(PlayerEvent::FileLoaded),
            "seek" | "playback-restart" => {
                // Let the next position through immediately.
                self.last_position = None;
                (event["event"] == "seek").then_some(PlayerEvent::Seeked)
            }
            "end-file" => {
                let reason = match event.get("reason").and_then(Value::as_str) {
                    Some("eof") => EndReason::Eof,
                    Some("quit") => EndReason::Quit,
                    Some("error") => EndReason::Error(
                        event
                            .get("file_error")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error")
                            .to_owned(),
                    ),
                    _ => EndReason::Stopped,
                };
                Some(PlayerEvent::EndFile(reason))
            }
            "property-change" => self.property_change(event),
            _ => None,
        }
    }

    fn property_change(&mut self, event: &Value) -> Option<PlayerEvent> {
        let data = event.get("data")?;
        match event.get("name")?.as_str()? {
            "time-pos" => {
                let secs = data.as_f64()?.max(0.);
                if self.last_position.is_some_and(|last| (secs - last).abs() < 1.) {
                    return None;
                }
                self.last_position = Some(secs);
                Some(PlayerEvent::Position(Duration::from_secs_f64(secs)))
            }
            "duration" => Some(PlayerEvent::Duration(Duration::from_secs_f64(data.as_f64()?.max(0.)))),
            "pause" => Some(PlayerEvent::Paused(data.as_bool()?)),
            "volume" => Some(PlayerEvent::Volume(data.as_f64()?)),
            "mute" => Some(PlayerEvent::Muted(data.as_bool()?)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn translates_and_throttles() {
        let mut t = EventTranslator::default();
        let pos = |s: f64| json!({"event": "property-change", "id": 0, "name": "time-pos", "data": s});
        assert_eq!(t.translate(&pos(0.0)), Some(PlayerEvent::Position(Duration::ZERO)));
        assert_eq!(t.translate(&pos(0.5)), None);
        assert_eq!(t.translate(&pos(1.2)), Some(PlayerEvent::Position(Duration::from_secs_f64(1.2))));
        assert_eq!(t.translate(&json!({"event": "seek"})), Some(PlayerEvent::Seeked));
        assert_eq!(t.translate(&pos(1.3)), Some(PlayerEvent::Position(Duration::from_secs_f64(1.3))));
        // Unset properties arrive without data.
        assert_eq!(t.translate(&json!({"event": "property-change", "name": "time-pos"})), None);
        assert_eq!(
            t.translate(&json!({"event": "property-change", "name": "pause", "data": true})),
            Some(PlayerEvent::Paused(true))
        );
        assert_eq!(
            t.translate(&json!({"event": "property-change", "name": "volume", "data": 55.0})),
            Some(PlayerEvent::Volume(55.0))
        );
        assert_eq!(
            t.translate(&json!({"event": "end-file", "reason": "eof"})),
            Some(PlayerEvent::EndFile(EndReason::Eof))
        );
        assert_eq!(
            t.translate(&json!({"event": "end-file", "reason": "error", "file_error": "loading failed"})),
            Some(PlayerEvent::EndFile(EndReason::Error("loading failed".into())))
        );
        assert_eq!(t.translate(&json!({"event": "client-message"})), None);
    }
}
