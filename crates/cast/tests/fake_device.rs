//! The sender against a fake device on loopback: TLS, heartbeat, the receiver namespace and the
//! Jellyfin contract, as far as a scripted device can take them.

use std::sync::Arc;
use std::time::Duration;

use cast::fake::{APP_DISPLAY_NAME, APP_SESSION_ID, APP_TRANSPORT_ID, FakeDevice};
use cast::jellyfin::{self as jf, Command, Credentials, Envelope, ItemStub, PlayRequest, ReceiverEvent};
use cast::{Channel, ChannelConfig, Error, Message, NS_CONNECTION, NS_RECEIVER, receiver};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::timeout;

fn config() -> ChannelConfig {
    ChannelConfig {
        ping_interval: Duration::from_millis(50),
        idle_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(2),
        connect_timeout: Duration::from_secs(2),
    }
}

async fn connect(device: &FakeDevice) -> (Channel, mpsc::UnboundedReceiver<Message>) {
    Channel::connect_with(device.addr(), config())
        .await
        .expect("the fake device accepts the connection")
}

fn credentials() -> Credentials {
    Credentials {
        server_address: "http://192.168.1.5:8096".into(),
        access_token: "token-123".into(),
        user_id: "user-1".into(),
        device_id: "device-1".into(),
        server_id: "server-1".into(),
        server_version: "12.2.0".into(),
        receiver_name: "Living Room TV".into(),
        max_bitrate: Some(8_000_000),
        subtitle_appearance: Some(json!({"textSize": "large"})),
    }
}

fn item() -> ItemStub {
    ItemStub {
        id: "item-1".into(),
        server_id: "server-1".into(),
        name: "Pilot".into(),
        kind: "Episode".into(),
        media_type: "Video".into(),
        is_folder: false,
    }
}

/// Polls until `check` holds, so tests don't depend on how long a round trip takes.
async fn wait_for(mut check: impl FnMut() -> bool) {
    for _ in 0..300 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not met in time");
}

/// The next message in the app's namespace, parsed as a receiver event.
async fn next_event(messages: &mut mpsc::UnboundedReceiver<Message>) -> ReceiverEvent {
    loop {
        let message = timeout(Duration::from_secs(2), messages.recv())
            .await
            .expect("an event from the receiver")
            .expect("the stream is still open");
        if message.namespace == jf::NAMESPACE {
            assert_eq!(message.source, APP_TRANSPORT_ID);
            return ReceiverEvent::from_value(&message.payload).expect("a receiver event");
        }
    }
}

#[tokio::test]
async fn heartbeat_runs_both_ways() {
    let device = FakeDevice::start().await.unwrap();
    let (_channel, mut messages) = connect(&device).await;
    // The server side of the handshake finishes a moment after the client's.
    wait_for(|| device.connections() == 1).await;

    // The client pings every 50 ms.
    wait_for(|| device.pings() >= 2).await;
    assert_eq!(device.pongs(), 0);

    // The device's own ping is answered with a pong.
    device.send_ping();
    wait_for(|| device.pongs() >= 1).await;

    // Heartbeat traffic is consumed, not handed on as app messages.
    device.send_ping();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(messages.try_recv().is_err());
}

#[tokio::test]
async fn launch_returns_the_app() {
    let device = FakeDevice::start().await.unwrap();
    let (channel, _messages) = connect(&device).await;

    // Nothing is running yet.
    let status = receiver::get_status(&channel).await.unwrap();
    assert!(status.applications.is_empty());
    assert_eq!(status.volume.and_then(|volume| volume.level), Some(0.5));

    let app = receiver::launch(&channel, jf::STABLE_APP_ID).await.unwrap();
    assert_eq!(app.app_id, jf::STABLE_APP_ID);
    assert_eq!(app.transport_id, APP_TRANSPORT_ID);
    assert_eq!(app.session_id, APP_SESSION_ID);
    assert_eq!(app.display_name.as_deref(), Some(APP_DISPLAY_NAME));
    assert!(app.has_namespace(jf::NAMESPACE));

    let status = receiver::get_status(&channel).await.unwrap();
    assert!(status.application(jf::STABLE_APP_ID).is_some());
}

#[tokio::test]
async fn launch_errors_are_typed() {
    let device = FakeDevice::start().await.unwrap();
    let (channel, _messages) = connect(&device).await;
    match receiver::launch(&channel, "not-an-app").await {
        // `INVALID_REQUEST` replies are mapped in `receiver.rs`, which a real device would send.
        Err(Error::Launch { reason }) => assert_eq!(reason, "NOT_FOUND"),
        other => panic!("expected a launch error, got {other:?}"),
    }
}

#[tokio::test]
async fn replies_are_matched_to_their_request_id() {
    let device = FakeDevice::start().await.unwrap();
    // Answer the launch after the status, so arrival order can't be what matches them.
    device.set_launch_delay(Duration::from_millis(300));
    let (channel, _messages) = connect(&device).await;

    let (status, app) = tokio::join!(
        receiver::get_status(&channel),
        receiver::launch(&channel, jf::STABLE_APP_ID)
    );
    assert!(status.unwrap().application(jf::STABLE_APP_ID).is_none());
    assert_eq!(app.unwrap().transport_id, APP_TRANSPORT_ID);
}

#[tokio::test]
async fn volume_and_mute_go_through_the_receiver_namespace() {
    let device = FakeDevice::start().await.unwrap();
    let (channel, _messages) = connect(&device).await;

    let status = receiver::set_volume_level(&channel, 0.25).await.unwrap();
    assert_eq!(status.volume.as_ref().and_then(|volume| volume.level), Some(0.25));
    let status = receiver::set_muted(&channel, true).await.unwrap();
    assert_eq!(status.volume.as_ref().and_then(|volume| volume.muted), Some(true));

    let payloads: Vec<Value> = device
        .messages()
        .into_iter()
        .filter(|message| message.namespace == NS_RECEIVER)
        .map(|message| message.payload)
        .collect();
    assert!(payloads.contains(&json!({"type": "SET_VOLUME", "volume": {"level": 0.25}, "requestId": 1})));
    // Mute and unmute share SET_VOLUME; unmuting sends `muted: false`.
    receiver::set_muted(&channel, false).await.unwrap();
    assert!(device.messages().iter().any(|message| message.payload
        == json!({"type": "SET_VOLUME", "volume": {"muted": false}, "requestId": 3})));
}

#[tokio::test]
async fn the_jellyfin_contract_round_trips() {
    let device = FakeDevice::start().await.unwrap();
    let (channel, mut messages) = connect(&device).await;
    let app = receiver::launch(&channel, jf::STABLE_APP_ID).await.unwrap();
    channel.connect_app(&app.transport_id).await.unwrap();

    let credentials = credentials();
    channel
        .send(
            jf::NAMESPACE,
            &app.transport_id,
            Envelope::new(&credentials, Command::Identify).to_value(),
        )
        .await
        .unwrap();

    // An idle receiver answers Identify with a volumechange about no item (empty ids).
    match next_event(&mut messages).await {
        ReceiverEvent::VolumeChange(event) => assert_eq!(event.data.item_id.as_deref(), Some("")),
        other => panic!("expected a volumechange, got {other:?}"),
    }

    // The app got a CONNECT on its transport id.
    wait_for(|| {
        device
            .messages()
            .iter()
            .any(|message| message.namespace == NS_CONNECTION && message.destination == APP_TRANSPORT_ID)
    })
    .await;

    // The envelope carried the whole contract.
    wait_for(|| !device.envelopes().is_empty()).await;
    let identify = device.envelopes().remove(0);
    assert_eq!(
        identify,
        json!({
            "command": "Identify",
            "options": {},
            "serverAddress": "http://192.168.1.5:8096",
            "accessToken": "token-123",
            "userId": "user-1",
            "deviceId": "device-1",
            "serverId": "server-1",
            "serverVersion": "12.2.0",
            "receiverName": "Living Room TV",
            "maxBitrate": 8_000_000,
            "subtitleAppearance": {"textSize": "large"}
        })
    );

    // PlayNow gets scripted progress back.
    let request = PlayRequest {
        items: vec![item()],
        start_index: Some(0),
        start_position_ticks: Some(0),
        media_source_id: Some("source-1".into()),
        audio_stream_index: Some(1),
        subtitle_stream_index: Some(-1),
    };
    channel
        .send(
            jf::NAMESPACE,
            &app.transport_id,
            Envelope::new(&credentials, Command::PlayNow(request)).to_value(),
        )
        .await
        .unwrap();

    for position in [0i64, 10_000_000, 20_000_000] {
        match next_event(&mut messages).await {
            ReceiverEvent::PlaybackProgress(event) => {
                assert_eq!(event.data.item_id.as_deref(), Some("item-1"));
                let state = event.data.play_state.as_ref().expect("a play state");
                assert_eq!(state.position_ticks, Some(position));
                assert_eq!(state.audio_stream_index, Some(1));
                assert_eq!(state.subtitle_stream_index, Some(-1));
                assert_eq!(state.media_source_id.as_deref(), Some("source-1"));
                let now_playing = event.data.now_playing_item.as_ref().expect("a now-playing item");
                assert_eq!(now_playing.name.as_deref(), Some("Fake item"));
                assert_eq!(now_playing.run_time_ticks, Some(1_440_000_000));
            }
            other => panic!("expected progress at {position}, got {other:?}"),
        }
    }

    // The PlayNow envelope is recorded with the documented options.
    let envelopes = device.envelopes();
    let play = envelopes
        .iter()
        .find(|envelope| envelope["command"] == "PlayNow")
        .expect("PlayNow was recorded");
    assert_eq!(play["options"]["items"][0]["Id"], "item-1");
    assert_eq!(play["options"]["items"][0]["Type"], "Episode");
    assert_eq!(play["options"]["startIndex"], 0);
    assert_eq!(play["options"]["startPositionTicks"], 0);
    assert_eq!(play["options"]["mediaSourceId"], "source-1");
    assert_eq!(play["options"]["audioStreamIndex"], 1);
    assert_eq!(play["options"]["subtitleStreamIndex"], -1);

    // A CLOSE reaches the app's transport (and, like the real receiver, the fake one quits).
    channel.close_app(&app.transport_id).await.unwrap();
    wait_for(|| {
        device.messages().iter().any(|message| {
            message.namespace == NS_CONNECTION
                && message.destination == APP_TRANSPORT_ID
                && message.payload["type"] == "CLOSE"
        })
    })
    .await;
    assert!(!device.is_running());
}

#[tokio::test]
async fn an_unread_stream_doesnt_stall_requests() {
    let device = FakeDevice::start().await.unwrap();
    // The messages are never read, as when the caller is busy awaiting a request.
    let (channel, _messages) = connect(&device).await;
    let app = receiver::launch(&channel, jf::STABLE_APP_ID).await.unwrap();
    channel.connect_app(&app.transport_id).await.unwrap();
    let credentials = credentials();
    let play = PlayRequest {
        items: vec![ItemStub {
            id: "item-1".into(),
            ..ItemStub::default()
        }],
        ..PlayRequest::default()
    };
    // Each PlayNow is answered with three progress events: far more than a bounded stream holds.
    for _ in 0..30 {
        let envelope = Envelope::new(&credentials, Command::PlayNow(play.clone())).to_value();
        channel.send(jf::NAMESPACE, &app.transport_id, envelope).await.unwrap();
    }
    wait_for(|| device.envelopes().len() == 30).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let status = timeout(Duration::from_secs(2), receiver::get_status(&channel))
        .await
        .expect("the reply isn't stuck behind unread messages")
        .unwrap();
    assert!(status.application(jf::STABLE_APP_ID).is_some());
}

#[tokio::test]
async fn a_message_too_big_is_refused_and_the_connection_stays() {
    let device = FakeDevice::start().await.unwrap();
    let (channel, _messages) = connect(&device).await;
    let app = receiver::launch(&channel, jf::STABLE_APP_ID).await.unwrap();
    let big = json!({"command": "PlayNow", "options": {"pad": "x".repeat(cast::MAX_FRAME)}});
    assert!(matches!(
        channel.send(jf::NAMESPACE, &app.transport_id, big).await,
        Err(Error::FrameTooLarge { .. })
    ));
    // Still connected: a request still gets its answer.
    let status = receiver::get_status(&channel).await.unwrap();
    assert!(status.application(jf::STABLE_APP_ID).is_some());
}

#[tokio::test]
async fn stop_ends_the_session() {
    let device = FakeDevice::start().await.unwrap();
    let (channel, _messages) = connect(&device).await;
    let app = receiver::launch(&channel, jf::UNSTABLE_APP_ID).await.unwrap();
    let status = receiver::stop(&channel, &app.session_id).await.unwrap();
    assert!(status.applications.is_empty());
    assert!(
        device
            .messages()
            .iter()
            .any(|message| message.payload == json!({"type": "STOP", "sessionId": APP_SESSION_ID, "requestId": 2}))
    );
}

#[tokio::test]
async fn a_closed_connection_ends_the_stream_and_fails_requests() {
    let device = FakeDevice::start().await.unwrap();
    let (channel, mut messages) = connect(&device).await;
    let channel = Arc::new(channel);
    let app = receiver::launch(&channel, jf::STABLE_APP_ID).await.unwrap();

    // The fake records app commands but never answers them, so this one stays pending.
    let pending = tokio::spawn({
        let channel = channel.clone();
        let transport = app.transport_id.clone();
        async move {
            channel
                .request(jf::NAMESPACE, &transport, json!({"command": "Pause"}))
                .await
        }
    });
    wait_for(|| device.envelopes().iter().any(|envelope| envelope["command"] == "Pause")).await;

    device.close();

    match timeout(Duration::from_secs(2), pending)
        .await
        .expect("the pending request must not hang")
        .expect("the task must not panic")
    {
        Err(Error::Closed) => {}
        other => panic!("expected the request to fail as closed, got {other:?}"),
    }
    // What arrived before the close (the launch's broadcast) can still be read; then it ends.
    timeout(Duration::from_secs(2), async {
        while messages.recv().await.is_some() {}
    })
    .await
    .expect("the message stream must end");
}

#[tokio::test]
async fn a_reconnect_finds_the_receiver_still_playing() {
    let device = FakeDevice::start().await.unwrap();
    let (channel, mut messages) = connect(&device).await;
    let app = receiver::launch(&channel, jf::STABLE_APP_ID).await.unwrap();
    channel.connect_app(&app.transport_id).await.unwrap();
    let credentials = credentials();
    let request = PlayRequest {
        items: vec![item()],
        ..PlayRequest::default()
    };
    channel
        .send(
            jf::NAMESPACE,
            &app.transport_id,
            Envelope::new(&credentials, Command::PlayNow(request)).to_value(),
        )
        .await
        .unwrap();
    for _ in 0..3 {
        assert!(matches!(
            next_event(&mut messages).await,
            ReceiverEvent::PlaybackProgress(_)
        ));
    }

    // The connection drops as a lost Wi-Fi would: the receiver stays up and playing.
    device.close();
    wait_for(|| device.disconnects() >= 1).await;
    assert!(device.is_running());

    // A new connection sees it, and `Identify` answers with what it plays.
    let (channel, mut messages) = connect(&device).await;
    let status = receiver::get_status(&channel).await.unwrap();
    assert!(status.application(jf::STABLE_APP_ID).is_some());
    channel.connect_app(&app.transport_id).await.unwrap();
    channel
        .send(
            jf::NAMESPACE,
            &app.transport_id,
            Envelope::new(&credentials, Command::Identify).to_value(),
        )
        .await
        .unwrap();
    match next_event(&mut messages).await {
        // A real receiver answers a `playbackstop`-typed message carrying the state.
        ReceiverEvent::PlaybackStop(event) => {
            assert_eq!(event.data.item_id.as_deref(), Some("item-1"));
            let state = event.data.play_state.as_ref().expect("a play state");
            assert_eq!(state.position_ticks, Some(20_000_000));
            assert_eq!(state.media_source_id.as_deref(), None);
        }
        other => panic!("expected the state as a playbackstop, got {other:?}"),
    }

    // Another sender's item reaches us the way the device broadcasts it.
    device.send_media_status("other-item", 12., true);
    let message = timeout(Duration::from_secs(2), async {
        loop {
            let message = messages.recv().await.expect("the stream is still open");
            if message.namespace == cast::NS_MEDIA {
                return message;
            }
        }
    })
    .await
    .expect("a media status from the device");
    assert_eq!(message.source, "receiver-0");
    let status = jf::MediaStatus::from_value(&message.payload).unwrap();
    let entry = status.entry().unwrap();
    assert_eq!(entry.item_id(), Some("other-item"));
    assert_eq!(entry.player_state.as_deref(), Some("PAUSED"));
    assert_eq!(entry.position_seconds(), Some(12.0));
}

#[tokio::test]
async fn a_silent_device_times_the_channel_out() {
    let device = FakeDevice::start().await.unwrap();
    device.silence();
    let (_channel, mut messages) = Channel::connect_with(
        device.addr(),
        ChannelConfig {
            ping_interval: Duration::from_millis(20),
            idle_timeout: Duration::from_millis(150),
            ..config()
        },
    )
    .await
    .unwrap();
    assert!(
        timeout(Duration::from_secs(2), messages.recv())
            .await
            .expect("the idle timeout must end the channel")
            .is_none()
    );
}
