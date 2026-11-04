# Android: plan for phones and tablets

**Goal:** Tabbyfin on Android phones and tablets, driven by touch. It's the same Rust and egui app as on the desktop, started through eframe's Android backend, so the API client, the pages, the cozy look (`DESIGN.md`), SyncPlay and remote control come along. There's no separate Kotlin UI.

**Status:** the app runs in the desktop layout: sign-in by touch and the soft keyboard, Home, libraries and item pages, the system Back, and the session kept across restarts (A0 and A1 below), on the emulator and a Honor phone. Playback works on the phone with both players (A3, below): Media3 by default and mpv as a setting, drawing into a SurfaceView under the app's surface, with a touch player page. The touch layout (A4) is done on the phone: a bottom bar on compact screens, the pages fitted to a phone's width, 48-point touch targets and long-press menus. The release build (A5) makes release-signed APKs, but nothing is published until the touch UI has been refined.

- **Phones and tablets first.** Android TV comes later; the keyboard cursor (`browse::Browse`, `keys.rs`) already suits a D-pad, so most of TV is a launcher entry and a 10-foot layout.
- **Devices:** Android 8 (API 26) and newer, where AHardwareBuffer and Vulkan 1.0 are available. arm64-v8a first, x86_64 for the emulator.

## Decision

Both players, as [Findroid](https://github.com/jarnedemeulemeester/findroid) has them (see the video spike below for the measurements):
- **Media3 (ExoPlayer) by default,** with Jellyfin's ffmpeg audio extension (`media3-ffmpeg-decoder`, for DTS, TrueHD and the like) and tunneled playback. It keeps HDR on an HDR screen and uses the least CPU in the app at 4K.
- **libmpv as a setting** (mpv-android's build), for what Media3 plays badly: ASS subtitles with their styling, and anything Android's decoders can't take. It runs with `vo=gpu-next`, `gpu-context=android`, `hwdec=mediacodec`, `ao=aaudio`, `profile=fast` and a `fonts.conf` for the system's fonts.
- **Both draw into a SurfaceView,** never through the CPU: on this phone that's every frame of 4K60 HDR10 at about 1.6 to 1.8 W, where copying the frames to the CPU loses most of them. The SurfaceView sits under the egui surface, which stays transparent over the video so the player page's controls show.
- **Both are backends behind the `Player` API.** Media3's translates the app's commands, as `media3.rs` does in the spike. Track selection, external subtitles and delays get built for it, and its subtitle cues are drawn in egui.

## Scope

**First release:** sign-in (password and Quick Connect, several accounts), Home, libraries, search, item pages, playback with server reporting, resume, audio and subtitle tracks, and Settings. Portrait and landscape on phones and tablets.

**Later:** SyncPlay and remote control (they compile as-is, but need care around the app going to the background), Android TV, picture-in-picture, background audio, downloads for offline viewing (on every platform, not only Android), Chromecast.

**Chromecast** comes next, through Jellyfin's own receiver app (the one the web client and Jellyfin's Android app cast to) rather than Google's Default Media Receiver: the receiver negotiates, transcodes and reports to the server itself, and shows up as a Jellyfin session, so the app's remote control can drive it. It needs a Chromecast or Google TV on the network to test against.

## Build and packaging

- `crates/app` is a library (`lib.rs`) that the desktop's `main.rs` runs. For Android, `cargo ndk … rustc --lib --crate-type cdylib` builds it as `libtabbyfin.so`; the crate itself stays `rlib`, so desktop builds don't link a second copy. `android_main(AndroidApp)` in `lib.rs` points `TABBYFIN_CONFIG_DIR` at the app's private storage, logs to logcat (`adb logcat -s tabbyfin`) and hands the `AndroidApp` to eframe through `NativeOptions`.
- **GameActivity**, through eframe's `android-game-activity` feature: it has the soft keyboard and IME that NativeActivity lacks, and eframe's AccessKit support (TalkBack) only builds with it. `packaging/android/` has the Java `MainActivity` (a bare `GameActivity` that loads the library) and depends on `androidx.games:games-activity:4.4.0`, the version android-activity 0.6 is written against. Don't enable GameActivity's prefab C++ glue; android-activity brings its own.
- **wgpu on Android:** the device asks for the adapter's own limits, since egui-wgpu's desktop defaults are more than phone GPUs (and the emulator) offer; and debug labels and validation are off, since the emulator's Vulkan driver crashes naming objects (`WGPU_DEBUG=1` turns them back on). On the emulator the app picks GL: its software Vulkan draws the first frames, then times out getting every frame after ("Dropped frame with error: Timeout").
- **What winit 0.30 drops** (`android.rs`, with `MainActivity.java` reporting it): the window insets (system bars, cutouts, the soft keyboard) become egui's safe area, and `App::ui` lays every screen out in `ctx.content_rect()` while the backdrop fills the window (`backdrop::fill_window`); the keyboard's text, which GameActivity keeps a copy of, is diffed against what it held last and passed on as Backspace presses, an IME commit and the word being composed; the keyboard's action key (Done, or Search in a search box) is Enter; the focused field's kind switches the keyboard to suit it — the URL layout for a server address, no suggestions for a name, a password keyboard that suggests and learns nothing (`set_ime_editor_info`, then `MainActivity` restarts the input so it rereads it); and the system theme is the activity's night mode. Keys the keyboard sends as key events (Backspace on an empty copy) come through winit. Keyboards that type digits and Backspace as key presses (FUTO, AOSP's) have them applied to the copy in `MainActivity` rather than by GameTextInput, which restarts the input after each and so drops the keyboard back to its letters. The keyboard's copy is only reset when the focus moves to another field, never while typing, which would lose what it typed in between. android-activity 0.6.1 builds a slice from a null pointer when the text is read before the keyboard has had any, so it's read only after Java reports a change.
- **Back and the lifecycle:** `MainActivity` takes the system Back. While the app has somewhere to go (a dialog, menu or focused field, the player, a page under this one; decided each frame in `App::ui`) it becomes Escape, which already closes, leaves or goes back; otherwise the app goes to the background (`moveTaskToBack`) rather than closing. The Rust app can run only once per process (eframe can't start a second event loop, and winit doesn't end the first when the activity is destroyed), so the process exits in `onDestroy`; the next launch restores the session.
- **No autofocus:** the sign-in form doesn't focus its first field at launch, since that opens the keyboard over a form not yet seen.
- **Environment:** an app gets no environment, so the system property `debug.tabbyfin.env` stands in for it: `adb shell setprop debug.tabbyfin.env "RUST_LOG=debug;WGPU_BACKEND=vulkan"`, then restart the app (the value needs quoting inside the adb shell when it has `;`).
- `packaging/android/` is a Gradle project (AGP 9.5, Gradle 9.8, JDK 21, all in `mise.toml` except AGP): the manifest (`io.github.lgpy.tabbyfin`, the Flatpak's ID; `INTERNET`, `WAKE_LOCK`, cleartext HTTP for LAN servers; rotation handled by the activity), the launcher icon: an adaptive icon of two vector drawables, the dark theme's plum backdrop with the mascot's glow, and the mascot inside the mask's 66-dp safe circle, which `make-icon.py` writes from `packaging/tabbyfin.svg` (run it again after changing the mascot). Gradle rather than `cargo-apk` or `xbuild`, because GameActivity, Media3 (if the spike picks it) and any Kotlin glue need it anyway.
- `packaging/android/build.sh [--release | --sign] [--abi …]` builds the library into `src/main/jniLibs/` (git-ignored) and packages it; mise tasks `android:build` (both ABIs) and `android:install` (the connected device's ABI, then starts it). `--release` optimises the library but still makes the debug-signed APK, about 75 MB for both ABIs, so it installs over a debug build. A debug library is ~500 MB of debug info. Gradle repackages in place, so an incremental APK can look much bigger than its contents; `rm -rf packaging/android/build` shrinks it.
- **The release** (`--sign`): the library in release, R8 over the Java side (`proguard-rules.pro` keeps the app's classes, which Rust finds by name), and a release-signed APK per ABI in `build/outputs/apk/release/` (about 60 MB for arm64 and 69 MB for x86_64, with the library's symbol table stripped). The version is the workspace's in `Cargo.toml`; the version code is that version as a number times ten (0.2.0 is 2000), plus 1 for arm64-v8a and 2 for x86_64, and a debug APK ends in 0.
- **The signing key** never enters the repository. `build.sh --sign` reads it from `TABBYFIN_KEYSTORE` (the file), `TABBYFIN_KEYSTORE_PASSWORD`, `TABBYFIN_KEY_ALIAS` and `TABBYFIN_KEY_PASSWORD`; CI will take it from the repository's secrets `ANDROID_KEYSTORE` (the file in base64), `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD`. Losing the key means no update can install over the app again, so keep a backup outside the machine. To make it, and later hand it to CI (`keytool` is the JDK's, under `mise exec --`):

  ```sh
  keytool -genkeypair -keystore tabbyfin-release.jks -alias tabbyfin -keyalg RSA -keysize 4096 -validity 10000
  base64 -w0 tabbyfin-release.jks | gh secret set ANDROID_KEYSTORE
  gh secret set ANDROID_KEYSTORE_PASSWORD
  gh secret set ANDROID_KEY_ALIAS --body tabbyfin
  gh secret set ANDROID_KEY_PASSWORD
  ```
- CI's `android` job builds the `--release` APK on every push and uploads it. The GitHub release doesn't carry the Android APKs yet: that's a `--sign` step on `v*` tags, with the key from the secrets above, added to the release job's `needs` once the touch UI is ready to ship.
- **Stores, later:** the main F-Droid repository builds everything from source, and the APK holds prebuilt native code (mpv-android's libmpv and ffmpeg, Jellyfin's ffmpeg extension for Media3), so it would need libmpv built with mpv-android's buildscripts first. IzzyOnDroid takes the GitHub release's APKs as they are. The Play Store wants an app bundle and its own upload key.

## What the desktop code assumes

Android is `target_os = "android"`, not `"linux"`. Code gated on `target_os = "linux"` (`ash`, `zbus`, the EGL video path) drops out, while code gated on `not(any(windows, target_os = "macos"))` applies and has to be checked case by case.

| Area | Desktop | Android |
|---|---|---|
| Player process (`player::is_host_process`, `host.rs`) | libmpv in a player host process, the same executable re-run | In-process, always (`Player::spawn`): Media3 by default (`media3.rs`, `Media3Bridge.java`), mpv-android's libmpv as the *Player* setting; both draw into `VideoSurface.java`, placed by `android.rs` |
| wgpu backends (`lib.rs`, `BACKENDS`) | Vulkan with GL fallback on Linux | Vulkan with GLES fallback (works on the emulator's SwiftShader Vulkan) |
| Config and logs (`config.rs`, `ProjectDirs`) | Platform config dir | `android_main` sets `TABBYFIN_CONFIG_DIR` to `internal_data_path()`, so config, logs and image cache all go there; the image cache belongs in the cache dir, which the OS may clear |
| Tokens (`credentials.rs`) | System keychain, `tokens/` file fallback | keyring has no Android store, so the `tokens/` fallback in app-private storage; Android Keystore through JNI later |
| Theme (`theme.rs`) | XDG portal on Linux | The activity's night mode, as egui's system theme (`android.rs`) |
| Update check (`update.rs`) | Checks GitHub releases | Off, and its Settings row hidden; the store updates the app |
| Window placement (`window.rs`) | Saved size and position | Not applicable |
| Device name (`session.rs`) | Host name | The phone's model (`ro.product.model`) |

**Lifecycle.** Android destroys the window surface when the app goes to the background and when the screen rotates (unless the activity handles rotation itself, which it should). eframe recreates the surface; the app must pause playback or keep its audio going, keep its tokio tasks alive, and save the config when paused, since the process can be killed without warning after that.

## Touch UI

The cozy look stays. What changes is everything that assumes a mouse, a keyboard and a wide window. New pieces are reusable widgets in `widgets/`, mocked up before they're built.

- **Layout sizes:** `layout.rs` decides them each frame from the width the app lays out in: compact (under 600 dp wide), medium (under 840) and expanded. They apply with touch only (`layout::touch`, Android, or a test's `set_touch`), so the desktop is always expanded. On compact screens a bottom bar (Home, Libraries) replaces Home's *My Media*, the top bar is Back or the icon, the title, a Search button, SyncPlay and the account, and Settings is in the account menu (`docs/PLAN.md`, *Top bar*); pages keep 16 dp from the sides instead of 24; Home's shelves sit closer together; the item page stacks its header (a 240-dp banner, the poster at 60 %, the title at 28 across the page, Play as wide as the page with the other actions under it); the library's toolbar scrolls sideways in its strip and the grid makes its cards as wide as fill a row, three across a phone in portrait (with touch at any size); Settings is a list of its sections, each opening on its own page, with each control under its label; the sign-in column and dialogs fit the width. Shelves keep scrolling sideways, without their arrows.
- **Touch targets:** at least 48 dp. Menu rows (`popup::row_height`) and list rows (`ListRow`) are drawn 48 tall with touch; smaller controls keep their look and answer touches over 48 dp round their centre (`layout::grow`): `RoundButton`, `select`, `segmented`, `tabs`, `stepper`'s buttons, filter pills and switches. Cards and the item page's 42-dp squares are already near or over it.
- **Hover:** the menus open with a tap (a click), and a long press opens what a right click does: a card's or row's item menu, and on the genres and tags popover a pill's clearing (its hint says so with touch). Tooltips and hover tints stay as they are, unseen.
- **Scrolling:** kinetic scrolling by drag. A drag on a shelf is the row's or the page's, by its first 8 points (`widgets/shelf.rs`), so the page scrolls from a shelf too; the page's drag coasts after the release as egui's own does (`smooth_scroll::Coast`, with egui's decay), run by the scroll areas of the pages with shelves (Home, the item page).
- **Text input:** done: the platform (above); the keyboard's kind per field (`keyboard.rs`'s `Kind`: the URL layout for a server address, no suggestions for a name, a Search action key in a search box, and a password keyboard); and the field the keyboard has just opened or grown over scrolling itself back into view, through the scroll area it sits in.
- **Back:** done (above); left: predictive Back's animation.
- **Safe areas:** the insets are applied (above), and the status and navigation bar icons follow the theme: dark on the light one and light on the dark one, still so when the player closes and they come back. Left: the player drawing under the bars.
- **Player page:** done (`views/player/touch.rs`). A tap shows or hides the controls; a double tap on the left or right third skips 10 s, and further taps there in quick succession skip again; a sideways drag seeks, with the time and the trickplay thumbnail in the middle; an up and down drag sets the brightness (left half) or the volume (right half). The volume slider and the fullscreen button are left out. While the player is open the window stays on, in landscape, with the system bars hidden (`VideoSurface.watching`), and the brightness goes back to the system's when it closes. Left: picture-in-picture, a `MediaSession`, and the player drawing under a display cutout.

## Video spike

About a week, on a throwaway branch: both players playing the dev server's media and a real 4K HEVC HDR file on a phone.

**libmpv.** mpv-android's prebuilt `libmpv.so` (with its ffmpeg), loaded by the existing `dlopen` loader in `crates/player/src/libmpv/`. Hardware decoding with `hwdec=mediacodec-copy` first. Zero-copy would render mpv's GL output into an AHardwareBuffer-backed EGLImage and import that buffer into wgpu's Vulkan device (`VK_ANDROID_external_memory_android_hardware_buffer`): a backend beside `egl.rs` and an importer beside `widgets/video/vulkan.rs`, the same shape as the Linux path. Keeps one player, the `Player` API as it is, ASS subtitles and the track handling.

**Media3 (ExoPlayer).** Kotlin glue driven over JNI, its frames going to an `ImageReader` whose AHardwareBuffers are imported the same way, or to a SurfaceView under a transparent egui surface. Better hardware decoding, HDR and audio passthrough, but a second implementation behind the `Player` API, and ASS subtitles are weaker.

**Compared on:** whether it plays at all on the test phone, CPU and battery at 1080p and 4K, dropped frames, seek time, subtitle rendering, APK size, and how much of the `player` crate's API it can keep.

The result goes in a *Decision* section at the top of this file.

### Groundwork on the emulator

On the `spike/android-video` branch, both players play the dev server's 1080p H.264 movie on the emulator, through the `Player` API and the software video path (`Frame::Pixels`). Nothing is measured yet: the emulator renders in software, so CPU, battery and dropped frames wait for a phone.

**libmpv** (the default player on Android):
- `build.sh` takes mpv-android's libraries from its release APKs (`mpv-android.lock`, tag 2026-09-17): `libmpv.so`, ffmpeg's shared libraries and `libc++_shared.so`, but not mpv-android's own JNI glue (`libplayer.so`). The existing loader finds `libmpv.so` by name.
- ffmpeg needs the JavaVM for MediaCodec: `android_main` hands it over (`libmpv::set_java_vm`), and loading libmpv passes it to `av_jni_set_java_vm`.
- With `hwdec=mediacodec-copy`, MediaCodec decodes and mpv copies the frames back; audio goes through AudioTrack, and SRT subtitles render through libass even though mpv warns there's no fontconfig.
- The API is unchanged: tracks, seeking and subtitles work as on the desktop.
- Size: about 32 MB per ABI for arm64 (38 MB for x86_64), stored uncompressed in the APK.

**Media3** (`TABBYFIN_PLAYER=media3`):
- `Media3Bridge.java` runs ExoPlayer 1.8 on its own looper thread. `crates/player/src/media3.rs` turns the app's mpv commands and properties into JSON commands for it (load with headers and start, pause, seek, volume, mute, speed, release), and its callbacks into `PlayerEvent`s. JNI goes through raw `jni-sys`; the bridge class is loaded through the activity's class loader, and its native methods are registered at runtime.
- Frames go to a YUV `ImageReader` sized from the video track (ExoPlayer reports no video size until it has a surface). Rust converts them to RGBA, scaled down to the view: a CPU copy, standing in for the zero-copy import of the reader's AHardwareBuffers.
- Playback, pause, seeking, position and duration reporting, and the end of the file work. Not done: audio and subtitle track selection (`aid`, `sid`), external and embedded subtitles, subtitle and audio delay, and the `sub-*` style properties. Those are logged as ignored.
- On the emulator, the hardware H.264 decoder (`c2.goldfish.h264.decoder`) gives garbage through a CPU-read `ImageReader`, while libmpv's `mediacodec-copy` reads it fine. `TABBYFIN_PLAYER=media3sw` limits Media3 to Android's software decoders, which draw correctly. Whether a phone's decoders behave like this matters only for the CPU path, not for the AHardwareBuffer import.
- Size: a few MB of dex before R8.

### On a phone

**Setup:** a Honor ELI-NX9 (Snapdragon 7 Gen 3, Adreno 720, Android 16, a 1200×2664 HDR10/HLG/HDR10+ screen, on battery), with an optimised build that keeps the dev aids (`SPIKE_DEBUG_ASSERTIONS=1 build.sh --release`). The test files were the dev server's 1080p24 H.264 movie, and Jellyfin's `Test Jellyfin 4K HEVC HDR10 40M.mp4` (3840×2160, 60 fps, Main 10, about 39 Mbps), added to the dev library as "HDR10 Test 4K (2024)".

Each run starts playback with `TABBYFIN_ROUTE`, waits 8 s, then samples for 15 s:
- **CPU:** from `/proc`, in % of one core. Codec means the media.hwcodec, media.codec and media.swcodec processes; system is all 8 cores together.
- **Power:** the battery's current × voltage, from `BatteryManager` once a second.
- **Frames:** the frames handed to the UI per second (`player::stats`), with each player's drop counters.
- **Seek:** two seeks back (the Left key), timed from the seek to the first frame after it.

MagicOS hides app logs from logcat, so the app's log file and `files/spike.log` (`SpikeLog.java`) hold the numbers.

| Run | Shown | Drops | App CPU | Codec CPU | SurfaceFlinger | System CPU | Power |
|---|---|---|---|---|---|---|---|
| Home screen, idle | – | – | 0% | 0% | 0% | 15% | 735 mW |
| libmpv, CPU copy (`mediacodec-copy`, `vo=libmpv`), 1080p24 | 24 fps | 2 | 92% | 11% | 9% | 192% | 1832 mW |
| libmpv, CPU copy, 4K60 HDR10 | 17–20 fps | about 40/s | 147% | 86% | 8% | 297% | 2957 mW |
| libmpv, SurfaceView (`mediacodec`, `vo=gpu`), 1080p24 | yes | 0 | 42% | 9% | 21% | 197% | 1539 mW |
| libmpv, SurfaceView, 4K60 HDR10 | yes | 0 | 92% | 31% | 35% | 255% | 2320 mW |
| Media3, SurfaceView, 1080p24 | yes | 0 | 55% | 16% | 30% | 198% | 1464 mW |
| Media3, SurfaceView, 4K60 HDR10 | yes | 0 | 53% | 28% | 32% | 202% | 1723 mW |

- **Same hardware:** both players decode with Qualcomm's `c2.qti.avc.decoder` and `c2.qti.hevc.decoder`, and neither decoder drops frames.
- **The CPU copy can't keep up at 4K60:** libmpv's copy back to the CPU and its software rendering drop most frames there, at the highest power.
- **The SurfaceView runs** (`TABBYFIN_PLAYER=mpvsv` and `media3sv`, `VideoSurface.java`) put a SurfaceView over the player page, 16:9 across the width, and the player draws straight into it. Both show every frame. At 1080p they cost about the same. At 4K60 HDR10, Media3 uses about 600 mW (25%) less power, and about 40 points less of a core in the app.
- **Media3's CPU path doesn't run on this phone:** Qualcomm's decoders only produce GPU-compressed buffers, even with `USAGE_CPU_READ_OFTEN` asked for, and `Image.getPlanes()` aborts the app. With GPU-only frames (`media3gpu`: a `PRIVATE` `ImageReader`, each frame acquired and released, nothing drawn) it decodes 4K60 at about 1580 mW. That's the starting point for importing its AHardwareBuffers into wgpu.
- **Seek times aren't comparable:** libmpv's (15–50 ms) and Media3's (300–600 ms) count from different events.
- **The app's CPU includes the player page,** which repaints every frame.
- **Subtitles:** libmpv on the SurfaceView renders them itself (the 1080p run showed the SRT cue). Media3 doesn't render any yet.

**What mpv can do on Android** (mpv's manual): `hwdec=mediacodec` without the copy needs `--vo=gpu --gpu-context=android` (mpv drawing to an `android.view.Surface` it owns, as here) or `--vo=mediacodec_embed` (no subtitles, OSD or filters). It also cuts 10-bit to 8, so HDR is tone-mapped to SDR. libmpv can't hand frames to the wgpu UI without the CPU copy. Media3 can either have its AHardwareBuffers imported into wgpu or draw to a SurfaceView, which can keep HDR on an HDR screen.

### How Findroid does it

[Findroid](https://github.com/jarnedemeulemeester/findroid) (a native Kotlin Jellyfin client) ships both players behind Media3's `Player` interface: `MPVPlayer` implements it, so one player screen (Media3's `PlayerView`, whose SurfaceView both draw into) drives either.
- **The default is ExoPlayer,** with tunneled playback on, and Jellyfin's `media3-ffmpeg-decoder` extension for the audio Android lacks (DTS, DTS-HD, TrueHD). Its README says ASS styling is limited.
- **mpv is a setting** (with libmpv built by Findroid's author): `vo=gpu-next` by default (`gpu` as a choice), `gpu-context=android`, `hwdec=mediacodec` (or `mediacodec-copy`, or `no`), `ao=aaudio` (or `audiotrack`, `opensles`). The SurfaceView's surface is attached on creation, with `force-window=yes` and `vo` set; on destruction, `vo=null` and `force-window=no`. Like `mpvsv` here.
- **mpv details we don't have:**
  - A `fonts.conf` in mpv's config dir that points fontconfig at `/system/fonts` and `/product/fonts`, so libass finds the system's fonts.
  - `profile=fast` and a 64 MiB demuxer cache.
  - `hwdec-codecs` limited to the main codecs.
  - An audio session ID, audio focus (pausing when another app takes it), and `audio-set-media-role`.
- **Around the player** (whichever it is): the screen kept on while playing, immersive mode, cutout insets for the controls, picture-in-picture, a `MediaSession`, and touch gestures (`PlayerGestureHelper`).
- **Direct play only** (`static=true`): it posts playback info with a device profile, but doesn't transcode.

### Measured again, with Findroid's settings

- **libmpv** (`mpvsv`): `vo=gpu-next`, `ao=aaudio`, `profile=fast`.
- **Media3** (`media3sv`): tunneling asked for, and Jellyfin's ffmpeg audio extension (`media3-ffmpeg-decoder` 1.8.0+1, `EXTENSION_RENDERER_MODE_ON`).

Three runs each, alternating the players. A fourth round stalled on Wi-Fi (Media3 rendered 71 frames in 20 s) and is left out.

| Run (mean of 3) | Shown | App CPU | Codec CPU | System CPU | Power (each run) |
|---|---|---|---|---|---|
| libmpv, SurfaceView, 1080p24 | every frame | 44% | 8% | 187% | 1308 mW (1239, 1340, 1345) |
| Media3, SurfaceView, 1080p24 | every frame | 58% | 15% | 203% | 1349 mW (1272, 1522, 1254) |
| libmpv, SurfaceView, 4K60 HDR10 | every frame | 91% | 29% | 251% | 1597 mW (1607, 1548, 1637) |
| Media3, SurfaceView, 4K60 HDR10 | every frame | 53% | 27% | 214% | 1770 mW (1944, 1698, 1669) |

- **No power gap left:** with `gpu-next` and AAudio, libmpv draws about as much power as Media3 at 1080p, and slightly less at 4K. The earlier 25% gap at 4K came from libmpv's `vo=gpu` (2320 mW) and AudioTrack. Media3 still uses less of a core in the app at 4K.
- **The 4K clip has no audio track,** so tunneling (which pairs the video with the audio) can't have been active in those runs. At 1080p it made no measurable difference. Whether the decoder actually tunneled wasn't confirmed.
- **Audio** (a copy of the 1080p movie with DTS and TrueHD tracks, made with Jellyfin's ffmpeg: "Audio Codecs Test (2024)"):
  - libmpv decodes both itself.
  - Media3 played the DTS track with this phone's licensed decoder (`c2.DTS.audio_decoder`, which many phones don't have). It reports TrueHD as playable only through the ffmpeg extension, since the phone has no TrueHD decoder.
  - So Media3 needs the extension: about 1.5 MB of native code per ABI. The debug APK with all four of its ABIs grew from 74 to 87 MB.
- **Subtitles:** Media3 without its `PlayerView` would have its cues drawn in egui. mpv renders them itself, ASS included, but needs the `fonts.conf` above for the system's fonts.

**Not checked:** whether HDR actually reaches the screen as HDR (a screenshot can't show it); battery over a long session (the 15 s windows show the draw, not a percentage per hour); a second phone.

## Milestones

| # | Milestone | Effort | Done when |
|---|---|---|---|
| A0 | **Hello Android** | S–M | An eframe window with the cozy backdrop runs on a phone; `android:build` and the CI APK work. Runs on the emulator; a phone and the first CI run are left. |
| A1 | **Platform gaps** | M | The table above is handled; sign-in and Home work by touch, in the desktop layout. Done on the emulator. |
| A2 | **Video spike** | M | The decision is written down. Done (above). |
| A3 | **Playback** | L | Media3 (default) and libmpv (a setting) play on a SurfaceView under egui, with reporting, resume, tracks and subtitles, and the touch player page. Done on the phone. |
| A4 | **Touch layout** | L | Layout sizes, the bottom bar, touch targets, long-press menus, insets and the soft keyboard. Done on the phone. |
| A5 | **Release** | M | A release-signed APK per ABI on each GitHub release. `build.sh --sign` makes them; left: refining the touch UI, then the real key in CI's secrets, CI signing on tags, and the first release. Stores (above) after that. |

After that: Android TV, picture-in-picture, background audio, downloads.

## Testing

- **Emulator, on Windows:** the SDK's emulator, WHPX-accelerated on this machine, runs an x86_64 image at full speed. The AVD `tabbyfin` is a Pixel 7 on API 36 (`system-images;android-36;google_apis;x86_64`), and starts headless, drawing in software (SwiftShader, which still offers Vulkan and GLES):

  ```sh
  emulator -avd tabbyfin -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect
  ```

  It takes about two minutes to boot: `adb wait-for-device` waits for it, then `adb shell getprop sys.boot_completed` reads 1 once the launcher is up. Drive it with `adb shell input` (taps, swipes, text) and take screenshots with `adb exec-out screencap -p`. It's good for the app starting, sign-in and the touch layout, not for judging video. It takes 2–3 GB, so don't run it during a big cargo build, and stop it with `adb emu kill`. `ANDROID_HOME` and the SDK's `platform-tools`, `emulator` and `cmdline-tools/latest/bin` are on the user `PATH`; recreate the AVD with `avdmanager create avd -n tabbyfin -k "system-images;android-36;google_apis;x86_64" -d pixel_7`. `sdkmanager` and `avdmanager` need Java 17 or newer, and the default `java` here is 8, so set `JAVA_HOME` to a 17+ JDK when running them. (On Linux, KVM plays the same part, and the user needs to be in the `kvm` group.)
- **Real phone, over Wi-Fi:** the phone's *Wireless debugging* (Developer options), paired from the same machine with `adb pair` and `adb connect`. This is where playback, the video spike, battery, rotation and the real soft keyboard get tested.
- The `egui_kittest` UI tests keep running on the desktop; `ui_tests/touch.rs` turns touch on in a phone-sized harness for the phone layout (the bars, Settings' sections, the grid, the item header, a long press).
- `TESTING.md` has an Android section for what only a phone can show.

## Risks

- **Text input:** done: the platform (above); the keyboard's kind per field (`keyboard.rs`'s `Kind`: the URL layout for a server address, no suggestions for a name, a Search action key in a search box, and a password keyboard); and the field the keyboard has just opened or grown over scrolling itself back into view, through the scroll area it sits in.
- **The touch layout touches nearly every page.** It's the largest part of the work.
- **Build memory:** Android builds are extra cargo builds on a machine that's already building the desktop app; run one at a time.
