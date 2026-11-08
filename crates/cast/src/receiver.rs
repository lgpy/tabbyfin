//! The device's own namespace: `urn:x-cast:com.google.cast.receiver`.
//!
//! This is how a device is asked what it's running, how an app is launched or stopped and how its
//! volume is set. Every request is answered by a `RECEIVER_STATUS` carrying the same `requestId`,
//! which is what [`Channel::request`] matches on. Failures arrive the same way, as `LAUNCH_ERROR`
//! or `INVALID_REQUEST`, and are mapped to typed [`Error`]s.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Channel, Error, NS_RECEIVER, Result};

/// The device itself, as opposed to an app it runs.
pub const RECEIVER_ID: &str = "receiver-0";

/// What the device reports about itself: the apps running and its volume.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReceiverStatus {
    pub applications: Vec<Application>,
    pub volume: Option<Volume>,
}

impl ReceiverStatus {
    /// The running app with this id, if any.
    pub fn application(&self, app_id: &str) -> Option<&Application> {
        self.applications.iter().find(|app| app.app_id == app_id)
    }
}

/// One app running on the device. `transport_id` is where its messages go.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Application {
    pub app_id: String,
    pub session_id: String,
    pub transport_id: String,
    pub display_name: Option<String>,
    pub namespaces: Vec<Namespace>,
    pub status_text: Option<String>,
}

impl Application {
    /// Whether the app listens on `namespace`, e.g. the Jellyfin receiver's
    /// `urn:x-cast:com.connectsdk`. A launch that worked but doesn't list it can't be driven.
    pub fn has_namespace(&self, namespace: &str) -> bool {
        self.namespaces.iter().any(|entry| entry.name == namespace)
    }
}

/// A namespace an app listens on.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Namespace {
    pub name: String,
}

/// The device's volume, level in 0.0..=1.0.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Volume {
    pub level: Option<f32>,
    pub muted: Option<bool>,
}

/// Asks the device what it's running.
pub async fn get_status(channel: &Channel) -> Result<ReceiverStatus> {
    let reply = channel
        .request(NS_RECEIVER, RECEIVER_ID, json!({"type": "GET_STATUS"}))
        .await?;
    status_from(reply)
}

/// Launches `app_id` (or joins it if it's already running) and returns the app the device reports,
/// from which its `transport_id` and `session_id` come. `LAUNCH_ERROR` and `INVALID_REQUEST`
/// become [`Error::Launch`] and [`Error::InvalidRequest`].
pub async fn launch(channel: &Channel, app_id: &str) -> Result<Application> {
    let reply = channel
        .request(NS_RECEIVER, RECEIVER_ID, json!({"type": "LAUNCH", "appId": app_id}))
        .await?;
    let status = status_from(reply)?;
    status
        .applications
        .into_iter()
        .find(|app| app.app_id == app_id)
        .ok_or_else(|| Error::Device(format!("launch of {app_id} reported no application")))
}

/// Stops the app with this `session_id`: the device goes back to its idle screen.
pub async fn stop(channel: &Channel, session_id: &str) -> Result<ReceiverStatus> {
    let reply = channel
        .request(
            NS_RECEIVER,
            RECEIVER_ID,
            json!({"type": "STOP", "sessionId": session_id}),
        )
        .await?;
    status_from(reply)
}

/// Sets the device's volume, 0.0..=1.0. This is the TV's volume, not the receiver's.
pub async fn set_volume_level(channel: &Channel, level: f32) -> Result<ReceiverStatus> {
    let reply = channel
        .request(
            NS_RECEIVER,
            RECEIVER_ID,
            json!({"type": "SET_VOLUME", "volume": {"level": level}}),
        )
        .await?;
    status_from(reply)
}

/// Mutes or unmutes the device.
pub async fn set_muted(channel: &Channel, muted: bool) -> Result<ReceiverStatus> {
    let reply = channel
        .request(
            NS_RECEIVER,
            RECEIVER_ID,
            json!({"type": "SET_VOLUME", "volume": {"muted": muted}}),
        )
        .await?;
    status_from(reply)
}

fn status_from(reply: Value) -> Result<ReceiverStatus> {
    match reply.get("type").and_then(Value::as_str) {
        Some("RECEIVER_STATUS") => {
            let status = reply.get("status").cloned().unwrap_or(Value::Null);
            serde_json::from_value(status).map_err(Error::from)
        }
        Some("LAUNCH_ERROR") => Err(Error::Launch {
            reason: reason_of(&reply),
        }),
        Some("INVALID_REQUEST") => Err(Error::InvalidRequest {
            reason: reason_of(&reply),
        }),
        other => Err(Error::Device(format!("expected a receiver status, got {other:?}"))),
    }
}

fn reason_of(reply: &Value) -> String {
    reply
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("no reason given")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_parsed_tolerantly() {
        let status: ReceiverStatus = serde_json::from_value(json!({
            "applications": [{
                "appId": "F007D354",
                "displayName": "Jellyfin",
                "namespaces": [{"name": "urn:x-cast:com.connectsdk"}, {"name": "urn:x-cast:com.google.cast.media"}],
                "sessionId": "abc-123",
                "statusText": "Ready To Cast",
                "transportId": "abc-123",
                "somethingNew": {"nested": true}
            }],
            "volume": {"level": 0.5, "muted": false}
        }))
        .unwrap();
        let app = status.application("F007D354").unwrap();
        assert_eq!(app.transport_id, "abc-123");
        assert!(app.has_namespace("urn:x-cast:com.connectsdk"));
        assert!(!app.has_namespace("nope"));
        assert_eq!(status.volume.as_ref().and_then(|v| v.level), Some(0.5));

        // Missing fields fall back to their defaults.
        let status: ReceiverStatus = serde_json::from_value(json!({})).unwrap();
        assert!(status.applications.is_empty());
        assert!(status.volume.is_none());
    }

    #[test]
    fn errors_are_mapped() {
        let launch = status_from(json!({"type": "LAUNCH_ERROR", "requestId": 2, "reason": "NOT_FOUND"}));
        assert!(matches!(launch, Err(Error::Launch { ref reason }) if reason == "NOT_FOUND"));
        let invalid = status_from(json!({"type": "INVALID_REQUEST", "requestId": 3}));
        assert!(matches!(invalid, Err(Error::InvalidRequest { .. })));
        let other = status_from(json!({"type": "MEDIA_STATUS"}));
        assert!(matches!(other, Err(Error::Device(_))));
    }
}
