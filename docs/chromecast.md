# Chromecast: plan

**Goal:** cast to a Chromecast or Google TV from the desktop and Android. We cast to Jellyfin's own receiver app ([jellyfin-chromecast](https://github.com/jellyfin/jellyfin-chromecast), the one the web client casts to), not to Google's Default Media Receiver. The receiver does the Jellyfin work itself: it calls `PlaybackInfo` with its own device profile, chooses direct play or a transcode, reports start, progress and stop to the server with our token, and plays the next episode. The app is only the remote: it finds devices, launches the receiver, sends it commands and draws what it reports.

**Status:** milestones 1 to 6 are built (`crates/cast`, `crates/app/src/cast.rs`, `views/casting.rs`, the player's casting view, and Android's glue in `android.rs` and `MainActivity.java`), and tested against the fake device, a real Chromecast and the Android emulator (below, *What a real Chromecast showed*). Left: the rest of `TESTING.md`'s checks on a device — a phone casting to a real TV, and a Google TV. The protocol notes come from reading the receiver's and jellyfin-web's source (jellyfin-chromecast `src/`, jellyfin-web `src/plugins/chromecastPlayer/plugin.js`, Nov 2026).

## Decision

- **We write our own CASTV2 sender, in Rust, and use it on every platform.** There's no Google Cast SDK for the desktop. On Android the SDK would need Play Services, Java glue and a second implementation, so Android uses the same Rust code.
- **We don't use `rust_cast`.** Its TLS pulls in aws-lc-rs, a C/CMake build that's a pain to cross-compile for Android. Its API blocks. It can only send a custom namespace to `"*"`, not to the app's `transportId`. And it drops `MEDIA_STATUS`'s `customData`. The protocol is small: a length-prefixed protobuf with seven fields, JSON payloads and four namespaces, about 400 lines on tokio and rustls (the ring provider reqwest already uses).
- **Discovery uses `mdns-sd`** (pure Rust and maintained). Android also needs a Wi-Fi `MulticastLock` while it discovers.
- **The cast channel is the only way to control the receiver.** The receiver shows up in the dashboard as a `Chromecast` session, but it opens no websocket, so the server can't forward `/Sessions/{id}/Playing…` to it. Our existing remote-control code can't drive it.

## The protocol

### CASTV2 (the device)

- **Transport.** TLS to the port in the mDNS SRV record (8009 for a single device; cast groups use other ports). The device's certificate is self-signed, so we skip verification with a custom rustls `ServerCertVerifier`, and use the IP as the server name.
- **Framing.** Each frame is a 4-byte big-endian length, then a proto2 `CastMessage`:

  | Field | Name | Notes |
  |---|---|---|
  | 1 | `protocol_version` | required; 0 = CASTV2_1_0. Encode it even though it is 0 |
  | 2 | `source_id` | |
  | 3 | `destination_id` | |
  | 4 | `namespace` | |
  | 5 | `payload_type` | required; 0 = string, 1 = binary |
  | 6 | `payload_utf8` | |
  | 7 | `payload_binary` | |

  We encode and decode it by hand, with no codegen. Our sender id is unique per connection (`sender-<uuid>`).
- **Steps** (all payloads are JSON):
  1. `urn:x-cast:com.google.cast.tp.connection` `CONNECT` to `receiver-0`.
  2. `…tp.heartbeat`: send `PING` every 5 s and answer the device's `PING` with `PONG`.
  3. `urn:x-cast:com.google.cast.receiver` `LAUNCH {appId, requestId}`, then wait for the `RECEIVER_STATUS` with the same `requestId`. Errors come back as `LAUNCH_ERROR` or `INVALID_REQUEST`.
  4. From `status.applications[]`, take `transportId` and `sessionId`, and check that `namespaces` lists `urn:x-cast:com.connectsdk`. `status.volume` holds `{level, muted}`.
  5. `CONNECT` to the `transportId`. Messages to the app go to that `transportId`.
- **Volume** is the device's: `SET_VOLUME {volume: {level}}` or `{volume: {muted}}`. **Stop casting** is `STOP {sessionId}`. **Leaving without stopping** is `CLOSE` on the connection namespace to the `transportId`.
- **Sender authentication** isn't needed. Only unpublished apps need device registration, and Jellyfin's apps are published.
- **Discovery** browses `_googlecast._tcp.local.`. TXT key `fn` is the friendly name, `md` the model (`Google Cast Group` for groups) and `id` the device id; `rs` is the app running on the device.

### The Jellyfin receiver (the app on the device)

- **App id.** Per user, from `GET /Users/Me` → `Configuration.CastReceiverId` (the web client's *Google Cast version*). The server's list is `/System/Info` → `CastReceiverApplications`. If the field is missing (older servers), fall back to stable, `F007D354`. Unstable is `6F511C87`.
- **Namespace.** `urn:x-cast:com.connectsdk`, in both directions. The payload is a JSON string.
- **Every message** carries these fields; the receiver rejects one that lacks `command`, `serverAddress` or `accessToken`:
  ```
  { command, options, serverAddress, accessToken, userId, deviceId, serverId, serverVersion,
    receiverName, maxBitrate?, subtitleAppearance? }
  ```
  - `receiverName` becomes the receiver session's device name, and its DeviceId (base64 of the name). Send the device's `fn`, so each TV keeps one session in the dashboard.
  - `maxBitrate` is in bits/s and stays set on the receiver once sent. Without it the receiver runs a bitrate test.
  - The last sender's credentials win.
- **`serverAddress` must be reachable from the TV.** When the session's server URL is loopback (`localhost`, `127.*`, `[::1]`), use `/System/Info/Public` → `LocalAddress` instead, as the web client does. A server on HTTPS with a self-signed certificate can't work: the TV won't trust it.
- **Commands we send:**

  | command | options | Notes |
  |---|---|---|
  | `Identify` | `{}` | Sent right after `CONNECT`, with credentials. The receiver then reports its capabilities to the server. It answers with the current state: a `playbackstop`-typed message while something plays, a `volumechange` with empty ids when idle. |
  | `PlayNow` | `{items, startIndex?, startPositionTicks?, mediaSourceId?, audioStreamIndex?, subtitleStreamIndex?}` | `items` are stubs, `{Id, ServerId, Name, Type, MediaType, IsFolder}`; the receiver fetches each item when it plays it. Positions and tracks apply only to the first item played. A single episode is extended with the rest of the series when the user has next-episode autoplay on. |
  | `PlayNext`, `PlayLast` | `{items}` | **Both append to the end of the queue.** |
  | `Shuffle` | `{items: [item]}` | The receiver picks up to 50 random items from the item. |
  | `Pause`, `Unpause`, `PlayPause`, `Stop`, `NextTrack`, `PreviousTrack` | `{}` | |
  | `Seek` | `{position}` | In seconds, not ticks. |
  | `SetAudioStreamIndex` | `{index}` | Reloads the stream. **Not used:** see *What a real Chromecast showed*. |
  | `SetSubtitleStreamIndex` | `{index}` | `-1` turns subtitles off. **Not used**, as above. |
  | `SetRepeatMode` | `{RepeatMode}` | Not used. After it is sent once, the receiver also sends `repeatmodechange` after every later command. |

  **Never send anything else.** An unknown command falls through to the receiver's play handler, which throws. Volume and mute commands on this namespace do nothing on the receiver; they go through `SET_VOLUME`.
- **What it sends back:** `{type, data?, message?}`, and only to the sender of the last custom message.
  - `type` is one of `playbackstart`, `playbackprogress` (about once a second while playing), `playstatechange`, `volumechange`, `playbackstop`, `playbackerror` (`message` is the error code, e.g. `NoCompatibleStream`), `connectionerror` (`PlaybackInfo` failed) or `error`.
  - `data` holds `ItemId`; `PlayState {PositionTicks, IsPaused, IsMuted, VolumeLevel, AudioStreamIndex, SubtitleStreamIndex, MediaSourceId, PlayMethod, CanSeek, …}`; and `NowPlayingItem {Id, Name, Type, RunTimeTicks, SeriesName, IndexNumber, ParentIndexNumber, MediaStreams, Chapters, image ids and tags}`.
  - There's no queue or playlist index in it.
- **Other senders** (another phone casting to the same TV) get none of those messages. They do get the standard `urn:x-cast:com.google.cast.media` `MEDIA_STATUS`, whose `media.customData` has `itemId`, `mediaSourceId`, the stream indexes, `canClientSeek` and `startPositionTicks`. When `canClientSeek` is false (a non-HLS transcode), the real position is `currentTime + startPositionTicks`. We follow `MEDIA_STATUS` when we aren't the last sender. Seeking and track changes still go through the custom commands, since the standard ones bypass the receiver's stream logic.
- **The token is shared with the receiver.** Signing out revokes it, and the receiver then stops working. Signing out while casting stops the cast first.

### What a real Chromecast showed

Seen on a Chromecast with the stable receiver (2026-11), against the dev server; the fake device in `cast::fake` now behaves the same way.

- **Leaving with `CLOSE` quits the receiver.** It stops when its last sender disconnects on purpose. *Disconnect* and quitting the app only close the socket, which leaves it playing.
- **`RECEIVER_STATUS` without `applications`.** Right after a launch and between apps the device sends statuses with only the volume. Our app is gone only when the list is there without it (usually listing the idle screen, *Backdrop*).
- **Empty ids mean nothing plays.** An idle receiver answers `Identify` with a `volumechange` whose `ItemId` and `NowPlayingItem.Id` are `""`. The end of an item is a `playbackstop` with empty ids too.
- **`SET_VOLUME` is answered in about 0.1 s**, even when the level doesn't change. The receiver sends progress about four times a second, so a sender that stops reading its messages while it awaits an answer stalls: the connection's message stream is unbounded for that reason, and volume requests don't block the loop.
- **The receiver's own track changes break.** `SetAudioStreamIndex`, and `SetSubtitleStreamIndex` when it has to reload (burned-in subtitles), restart the stream from `currentTime × 10⁷`. That's fractional, and the server answers a fractional `StartTimeTicks` with HTTP 400, so the TV shows `NoCompatibleStream`. Its stop reports have the same flaw: the server keeps the TV's session "playing" after it stopped.
- **A `PlayNow` over playback can leave *Ready to cast* on top.** Its handling of the interrupted stream can land after the new one started and show the idle screen over it for good, while the stream plays on underneath. Jellyfin's Android app does the same. So other tracks and versions are a `Stop`, then (once its `playbackstop` came, plus 1 s) a `PlayNow` from the current position in whole ticks (`Services::cast_reload`).
- **A frame is at most 64 KB.** A `PlayNow` with about 300 episodes is 51 KB, with 400 it's 67 KB. The app cuts a longer queue to what fits; the channel refuses a bigger message with `FrameTooLarge` instead of ending the connection.

## Code

### `crates/cast` (new, no UI dependencies)

- `codec.rs`: `CastMessage` encode and decode, and the frame reader and writer.
- `channel.rs`: the TLS connection to a device. A tokio task owns the socket, runs the heartbeat, and matches `requestId`s to replies. Apps' messages arrive on a stream.
- `receiver.rs`: `GET_STATUS`, `LAUNCH`, `STOP` and `SET_VOLUME`, with typed `ReceiverStatus`, `Application` and `Volume`.
- `discovery.rs`: `mdns-sd` browsing into a list of `Device {id, name, model, address, port, running_app}` that updates as devices come and go.
- `jellyfin.rs`: the receiver's contract as types. `Envelope` and `Command` (only the commands above, so an unknown one can't be sent), `PlayRequest`, `ItemStub`, and `ReceiverEvent` with `PlayState` and the now-playing item.
- `examples/cast.rs`: the live check. It lists devices, launches the receiver, sends `Identify` and prints what comes back; `--play <id>` casts an item from the dev server (`mise run cast`).
- `Cargo.toml`: `tokio`, `tokio-rustls` (ring), `mdns-sd`, `serde_json`, `uuid`.

### `crates/app`

- `cast.rs`: `Cast` in `Services`.
  - Discovery runs while signed in, and its results come back through a `task::channel` drained in `Services::tick`. An offline session discovers nothing and casts nothing.
  - `CastSession` holds the device, the connection, the app's `transportId`, and `Casting`, the state the UI draws: item, position (extrapolated between reports, as `NowPlaying` is), paused, duration, streams, the chosen audio and subtitle, and volume and mute (from `RECEIVER_STATUS` and `volumechange`). `reconnecting` is set while a dropped connection is being retried.
  - The connection's task (`run`/`join`) launches the receiver, then relays until it ends; a lost connection retries with backoff (`Reconnect`), and the `Identify` after each rejoin restores the state. Another sender's `MEDIA_STATUS` is followed once the custom messages go quiet.
  - It also holds the credentials it sends: the token, `serverAddress` with the loopback fix, and the receiver id from `/Users/Me`.
- `impl Services`: casting redirects the existing entry points, the same way SyncPlay does with `in_group()`.
  - `play`, `play_all_with`, `enqueue` and `play_now` become `PlayNow` or `PlayLast`. Items become stubs; `PlayOptions`' start, version, audio and subtitle go in the `PlayRequest`.
  - `control(UiCommand)`:

    | UiCommand | Sent as |
    |---|---|
    | `TogglePause` | `PlayPause` |
    | `SetPause` | `Pause` / `Unpause` |
    | `SeekTo` | `Seek` |
    | `SeekBy` | `Seek`, from the extrapolated position |
    | `SetVolume`, `SetMute`, `ToggleMute` | `SET_VOLUME` |
    | `Audio` | `SetAudioStreamIndex` |
    | `Subtitle` | `SetSubtitleStreamIndex` |
    | `Stop` | `Stop` |

    Speed, the delays and subtitle size aren't sent: the receiver has no controls for them.
  - `play_next` and `play_previous` become `NextTrack` and `PreviousTrack`.
  - Remote control from another client goes through the same entry points, so the dashboard's *Play on Tabbyfin* casts while we're casting.
- **Starting a cast while something plays here hands it over.** We stop the local session, after its stop report, then `PlayNow` the same item at its position with its version and tracks, and the queue as the rest of `items`.
- **Stopping the cast** has two options: *Stop casting* (`STOP` the app) or *Disconnect* (leave it playing, `CLOSE`). Quitting the app disconnects, as the web client does.
- **Casting and SyncPlay don't mix.** While in a group the cast button says why it's off, and joining a group while casting asks to stop the cast.
- **Errors** become toasts: launch failed, the device dropped, and `playbackerror` / `connectionerror` (with a hint about the server address the TV was given).
- **Config:** remember the last device id and show it first in the picker. Add an optional *Chromecast bitrate* setting, sent as `maxBitrate`.

### UI

The look below was settled with mockups.

- A **cast button** (`cast` icon, `cast_connected` and lit while casting) sits in the top bar just left of SyncPlay. It shows only when a device has been found or something is casting.
  - **Its panel** is built like SyncPlay's (`popup::panel`). Its heading is *Cast to a TV*, with a *Casting* status dot while casting, and a line saying what the panel does.
  - **The devices** are listed under an *On this network* overline, as one `list_row::block` of `ListRow`s. Each row has a TV icon (a group's is `speaker_group`), the device's name, and its model or *Group* at the right (mDNS doesn't say how many TVs a group has).
  - **The TV in use** is highlighted, with a `brand` `cast_connected` icon and *Casting*.
  - **A TV whose ad says Jellyfin's receiver is running** says *Reconnect to <name>* rather than *Cast to <name>*, with *Jellyfin is running* where the model goes: picking it brings back what the TV plays.
  - **While casting**, a quiet *Disconnect* and a quiet-danger *Stop casting* sit under the list.
- **While casting**, the player page draws `Casting` instead of `NowPlaying`.
  - **It keeps today's player layout.** Where the video was, it shows the item's backdrop, dimmed (`over_video::VEIL`). In the middle, a `cast_connected` mark in a dark circle sits over *Playing on <TV>* (`xl`) and the TV's model (`sm`, muted).
  - **The top bar is today's**, with a lit `cast_connected` button at its right end.
  - **The bottom bar is today's**, minus the settings button and fullscreen: play and pause, the 10 s skips, the TV's volume, the clock, previous and next, the version picker, chapters, audio and subtitles.
  - **It leaves out** speed, delays, subtitle size and trickplay. Trickplay can come later, since the thumbnails come from the server.
- **Back leaves the player while the cast keeps going.** A **now-casting bar** then brings it back with a click.
  - **On the desktop** it floats over the bottom of every page: at most 640 wide, 68 tall and 24 up from the bottom, in a popup's frame with radius 18.
  - **Its contents:** a 16:9 thumbnail, the episode's name (`sm`), and a `brand` cast mark beside *Series · on <TV>* (`xs`, muted). At the right end are the time and a pause button. The progress runs along its bottom edge in `brand` over `inset`.
  - **On a phone** it's full width less 8 on each side, 64 tall, sitting just above the bottom bar. It names only the TV and has no time.

### Android

- Add `CHANGE_WIFI_MULTICAST_STATE` to the manifest. The Java `MainActivity` holds a `WifiManager.MulticastLock` (`tabbyfin-cast`, not reference counted) while discovery runs, called through JNI (`jni-sys`, as the player crate does): `android.rs` has the calls, and a guard in `cast.rs` takes the lock before the mDNS browse starts and lets it go when the browse is dropped (sign-out, an offline session, quitting). Both are logged (`cast: multicast lock held=true`), since a phone that finds no TV is usually a lock that wasn't held. If `mdns-sd` proves unreliable on phones, `NsdManager` takes its place behind `discovery.rs`.
- **The volume keys set the TV's volume while casting**, a step a press (5 %, as the player's own keys), and Android's own volume UI has them when nothing is casting. `MainActivity.dispatchKeyEvent` asks Rust first: `nativeCastVolume(direction)` answers whether the app is casting (a flag the app updates each frame, like Back's) and, if it is, queues the step, which `App::logic` drains into the `UiCommand::SetVolume` the casting player's volume keys send. A held key's repeats each count as a press, and the press's release is taken with it. Each key is logged at debug level with whether the app was casting. There's no toast; the casting player's volume bar shows the TV's level as the receiver reports it.
- **In the background the cast keeps playing on the TV.** The connection runs on tokio, detached from frames: the heartbeat is the channel's own task, commands go into an unbounded channel, and updates into an unbounded `task::channel` that `Services::tick` drains in `App::logic`. Android stops drawing an app's frames once it's in the background, so `tick` may not run at all; nothing the connection does waits for it, and the reports that piled up are taken in one frame when the app comes back. Our connection may still be dropped there, so returning to the app reconnects (below).
- **When the cast ends while the app is backgrounded** — the receiver stopped, or the retries ran out — that shows when the app comes back: the same toast (*<TV> stopped casting*, or *Lost the connection*) and the player closes, which is the ended state the foreground would have shown.

## Staying connected

- **A dropped connection is retried with backoff.** The connection task keeps the command channel, so the UI and the session go on as before: after a loss it waits 1 s, then 2 s, 4 s, 8 s and 16 s (capped, five tries, or until *Stop casting* or *Disconnect* comes in, which ends it at once). Each try connects to the device and asks `GET_STATUS`; if the Jellyfin receiver is still listed, it `CONNECT`s to that app's `transportId` and sends `Identify`, whose answer restores the item, position and pause state. If the receiver isn't running any more, the cast ends as *<TV> stopped casting*; if the tries run out, with the usual *Lost the connection* toast. While it retries, the now-casting bar and the casting player say *Reconnecting to <TV>…* and the cast panel's status dot says *Reconnecting*. Commands made in the meantime (volume, pause) aren't replayed: the `Identify` answer is the truth. A reconnect doesn't launch anything over a running receiver.
- **At launch, a device whose ad says the receiver is running offers *Reconnect*.** The signal is mDNS's `rs` TXT field, the app the device advertises, which flow discovery already carries: no extra connection per device, and the device re-announces it when its app changes. A device whose `rs` names Jellyfin gets a *Reconnect to <name>* row (detail *Jellyfin is running*) in the cast panel, in place of *Cast to*. Picking it is the same connect as casting from idle — the receiver is joined rather than launched, and `Identify` brings back what it plays. Nothing reconnects on its own: at launch it's an offer, and in a running cast it's the retry above.
- **Another sender takes over** when the custom messages stop: the receiver answers only the sender of the last custom message. Once 3 s pass without one, the app follows the TV's own `MEDIA_STATUS` instead (the item and its `customData`, the position, paused and playing), kept from the last broadcast so the switch needs no new message; a status that arrived before the last custom one isn't mistaken for it. It keeps following, including an item that isn't ours, until the receiver answers this sender again — either a custom message arrives or we send a command, which makes us the last sender again.
- **Signing out stops the cast and any retry**: the receiver plays with this session's token, which signing out revokes.
- **Offline there's no casting.** Discovery doesn't start, the panel and the button aren't shown, and a cast can't be started; the session that replaces an offline one when the server comes back discovers normally.

## Milestones

1. **Channel:** codec, TLS, heartbeat, the receiver namespace and discovery, plus `examples/cast.rs` launching the receiver on a real device.
2. **Jellyfin receiver:** the typed messages and `--play` casting a dev item, with progress printed.
3. **App service:** `Cast` in `Services`, the redirects, the handover, the errors and the config.
4. **UI:** the cast panel, the casting player page and the now-casting bar.
5. **Staying connected:** reconnect, the reconnect offer at launch, other senders, and sign-out.
6. **Android:** the multicast lock, the volume keys, and the background.

## Testing

- **Unit tests:** the codec against frames captured from a device; `Envelope`, `PlayRequest` and `ReceiverEvent` against JSON taken from the receiver's source and from a device; and the `UiCommand` → `Command` mapping.
- **A fake device** for tests: `crates/cast/tests/` runs a TLS server with a self-signed certificate that speaks the connection, heartbeat and receiver namespaces and records the custom messages. It answers `PlayNow` with scripted `playbackprogress`, keeps the receiver running across a dropped connection and answers the next `Identify` with what it plays, can be taken off the network, and broadcasts the device's own `MEDIA_STATUS` for another sender. The app's UI tests drive the casting player page and the redirects against it, with no network beyond loopback.
- **Real hardware:** the checks are in `TESTING.md`, with how to reach the dev server from the TV.

## Risks

- **Only one Chromecast tried.** The frame limit and an HTTP server played from the HTTPS receiver page are confirmed on it; a Google TV, a cast group and a heartbeat held for an hour are not.
- **Receiver quirks we can't fix:**
  - `PlayNext` appends to the end of the queue.
  - Only the last sender gets the custom messages.
  - Two TVs with the same name share one dashboard session.
  - `SetRepeatMode`'s lingering burst of `repeatmodechange` messages.
- **mDNS on locked-down networks** (guest Wi-Fi, client isolation) finds nothing. A *connect by IP* field in the cast panel is the escape hatch.
