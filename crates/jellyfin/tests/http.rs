//! The client against a mocked server: what goes over the wire for playback negotiation and
//! reporting, and how HTTP failures map to errors. The live checks against real servers are
//! the `smoke` example; these run anywhere.

use std::time::Duration;

use jellyfin::{Client, ClientInfo, Error, PlayMethod, PlaybackInfoRequest, PlaybackReport, Ticks};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

async fn client(server: &MockServer) -> Client {
    let info = ClientInfo {
        client: "jelly-gpui-tests".into(),
        device: "tests".into(),
        device_id: "test-device".into(),
        version: "0.1.0".into(),
    };
    Client::new(&server.uri(), info)
        .expect("mock server URL")
        .with_token(TOKEN)
}

fn body(request: &Request) -> Value {
    serde_json::from_slice(&request.body).expect("a JSON body")
}

fn report() -> PlaybackReport {
    PlaybackReport {
        item_id: "item".into(),
        media_source_id: "source".into(),
        play_session_id: Some("session".into()),
        position: Ticks::from(Duration::from_secs(90)),
        is_paused: false,
        audio_stream_index: Some(1),
        subtitle_stream_index: Some(-1),
        play_method: PlayMethod::DirectPlay,
        volume_level: 80,
        is_muted: false,
    }
}

/// Start, a paused progress report and stop, in that order, each with the session's
/// identity and the token in the `Authorization` header (legacy auth is off in 12.x).
#[tokio::test]
async fn playback_reports_carry_the_session_and_position() {
    let server = MockServer::start().await;
    for route in [
        "/Sessions/Playing",
        "/Sessions/Playing/Progress",
        "/Sessions/Playing/Stopped",
    ] {
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    let client = client(&server).await;
    let mut report = report();
    client.report_playback_start(&report).await.expect("start");
    report.is_paused = true;
    report.position = Ticks::from(Duration::from_secs(95));
    client.report_playback_progress(&report).await.expect("progress");
    client.report_playback_stopped(&report).await.expect("stop");

    let requests = server.received_requests().await.expect("recording is on");
    let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();
    assert_eq!(
        paths,
        [
            "/Sessions/Playing",
            "/Sessions/Playing/Progress",
            "/Sessions/Playing/Stopped"
        ]
    );
    for request in &requests {
        let auth = request.headers["authorization"].to_str().expect("ASCII");
        assert!(auth.starts_with("MediaBrowser "), "{auth}");
        assert!(auth.contains(&format!(r#"Token="{TOKEN}""#)), "{auth}");
        assert!(auth.contains(r#"DeviceId="test-device""#), "{auth}");
    }

    let start = body(&requests[0]);
    assert_eq!(start["ItemId"], "item");
    assert_eq!(start["MediaSourceId"], "source");
    assert_eq!(start["PlaySessionId"], "session");
    assert_eq!(start["PositionTicks"], json!(900_000_000));
    assert_eq!(start["IsPaused"], json!(false));
    assert_eq!(start["PlayMethod"], "DirectPlay");
    assert_eq!(start["AudioStreamIndex"], json!(1));
    assert_eq!(start["SubtitleStreamIndex"], json!(-1));
    assert_eq!(start["VolumeLevel"], json!(80));
    assert_eq!(start["CanSeek"], json!(true));

    let progress = body(&requests[1]);
    assert_eq!(progress["IsPaused"], json!(true));
    assert_eq!(progress["PositionTicks"], json!(950_000_000));

    // `PlaybackStopInfo` has no paused, volume or track fields.
    let stop = body(&requests[2]);
    assert_eq!(stop["PositionTicks"], json!(950_000_000));
    assert_eq!(stop["PlaySessionId"], "session");
    assert!(
        stop.get("IsPaused").is_none() && stop.get("VolumeLevel").is_none(),
        "{stop}"
    );
}

/// The negotiation asks for direct play with transcoding allowed, under the bitrate cap, and
/// sends a device profile with subtitle profiles (without them 12.2 refuses direct play).
#[tokio::test]
async fn playback_info_offers_direct_play_under_the_bitrate_cap() {
    let server = MockServer::start().await;
    let fixture = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/12.2/playback_info_transcode.json"),
    )
    .expect("fixture");
    Mock::given(method("POST"))
        .and(path("/Items/item/PlaybackInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(fixture, "application/json"))
        .mount(&server)
        .await;
    let client = client(&server).await;
    let request = PlaybackInfoRequest {
        media_source_id: Some("source".into()),
        start_position: Some(Ticks::from(Duration::from_secs(30))),
        audio_stream_index: Some(2),
        subtitle_stream_index: None,
        max_streaming_bitrate: Some(8_000_000),
        prefer_hevc: false,
    };
    let info = client.playback_info("item", &request).await.expect("playback info");
    assert!(info.play_session_id.is_some());
    assert!(info.media_sources[0].transcoding_url.is_some());

    let requests = server.received_requests().await.expect("recording is on");
    let sent = body(&requests[0]);
    assert_eq!(sent["MediaSourceId"], "source");
    assert_eq!(sent["StartTimeTicks"], json!(300_000_000));
    assert_eq!(sent["AudioStreamIndex"], json!(2));
    assert!(sent.get("SubtitleStreamIndex").is_none(), "{sent}");
    assert_eq!(sent["MaxStreamingBitrate"], json!(8_000_000));
    assert_eq!(sent["EnableDirectPlay"], json!(true));
    assert_eq!(sent["EnableTranscoding"], json!(true));
    let profile = &sent["DeviceProfile"];
    assert_eq!(profile["MaxStreamingBitrate"], json!(8_000_000));
    assert!(!profile["SubtitleProfiles"].as_array().expect("a list").is_empty());
    assert_eq!(profile["TranscodingProfiles"][0]["Protocol"], "hls");
}

#[tokio::test]
async fn a_transcode_is_stopped_by_device_and_play_session() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/Videos/ActiveEncodings"))
        .and(query_param("deviceId", "test-device"))
        .and(query_param("playSessionId", "session"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .await
        .stop_transcoding("session")
        .await
        .expect("stop transcoding");
}

/// The statuses the app tells apart: a rejected token signs out, 403 explains whose a
/// playlist is, 503 is a server still starting.
#[tokio::test]
async fn http_failures_map_to_their_errors() {
    let server = MockServer::start().await;
    for (status, id) in [(401, "a"), (403, "b"), (404, "c"), (503, "d"), (500, "e")] {
        Mock::given(path(format!("/Items/{id}")))
            .respond_with(ResponseTemplate::new(status).set_body_string("nope"))
            .mount(&server)
            .await;
    }
    let client = client(&server).await;
    assert!(matches!(client.delete_item("a").await, Err(Error::Unauthorized)));
    assert!(matches!(client.delete_item("b").await, Err(Error::Forbidden)));
    assert!(matches!(client.delete_item("c").await, Err(Error::NotFound)));
    assert!(matches!(client.delete_item("d").await, Err(Error::ServerStarting)));
    match client.delete_item("e").await {
        Err(Error::Status { status, message }) => {
            assert_eq!(status, 500);
            assert_eq!(message, "nope");
        }
        other => panic!("expected a status error, got {other:?}"),
    }
}

/// The connection test downloads test data and times it: 500 kB in about a second is about
/// 4 Mbit/s.
#[tokio::test]
async fn the_bitrate_test_times_the_download() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Playback/BitrateTest"))
        .and(query_param("size", "500000"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(vec![0u8; 500_000])
                .set_delay(Duration::from_secs(1)),
        )
        .mount(&server)
        .await;
    let bits = client(&server).await.bitrate_test(500_000).await.expect("bitrate test");
    assert!((3_000_000..4_100_000).contains(&bits), "{bits} bit/s");
}
