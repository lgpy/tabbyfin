# Android: plan for phones and tablets

**Goal:** Tabbyfin on Android phones and tablets, driven by touch. It's the same Rust and egui app as on the desktop, started through eframe's Android backend, so the API client, the pages, the cozy look (`DESIGN.md`), SyncPlay and remote control come along. There's no separate Kotlin UI.

**Status:** a draft. Nothing is built yet. The video player is chosen by a spike (below) before playback work starts.

- **Phones and tablets first.** Android TV comes later; the keyboard cursor (`browse::Browse`, `keys.rs`) already suits a D-pad, so most of TV is a launcher entry and a 10-foot layout.
- **Devices:** Android 8 (API 26) and newer, where AHardwareBuffer and Vulkan 1.0 are available. arm64-v8a first, x86_64 for the emulator.

## Scope

**First release:** sign-in (password and Quick Connect, several accounts), Home, libraries, search, item pages, playback with server reporting, resume, audio and subtitle tracks, and Settings. Portrait and landscape on phones and tablets.

**Later:** SyncPlay and remote control (they compile as-is, but need care around the app going to the background), Android TV, picture-in-picture, background audio, downloads for offline viewing, Chromecast.

## Build and packaging

- `crates/app` builds as `crate-type = ["cdylib", "rlib"]`, with an `android_main(app: AndroidApp)` entry beside `main` that sets up logging and hands `app` to eframe through `NativeOptions`.
- eframe's `android-native-activity` feature to start with. `android-game-activity` is the fallback if text input needs it (see *Risks*).
- `cargo-ndk` builds the library; a small Gradle project in `packaging/android/` holds the manifest, the icon (the user's drawing) and the permissions (`INTERNET`, `WAKE_LOCK`, later `POST_NOTIFICATIONS`). Gradle rather than `cargo-apk` or `xbuild`, because Media3 (if the spike picks it) and any Kotlin glue need it anyway.
- mise tasks `android:build` and `android:install`; a CI job building an unsigned debug APK.
- Release: a signing key, then F-Droid (fits GPL-3.0) and possibly the Play Store.

## What the desktop code assumes

Android is `target_os = "android"`, not `"linux"`. Code gated on `target_os = "linux"` (`ash`, `zbus`, the EGL video path) drops out, while code gated on `not(any(windows, target_os = "macos"))` applies and has to be checked case by case.

| Area | Desktop | Android |
|---|---|---|
| Player process (`player::is_host_process`, `host.rs`) | libmpv in a player host process, the same executable re-run | An app can't re-run its own binary; play in-process, as `TABBYFIN_PLAYER=embedded` does today |
| wgpu backends (`main.rs`, `BACKENDS`) | Vulkan with GL fallback on Linux | Vulkan with GLES fallback |
| Config and logs (`config.rs`, `ProjectDirs`) | Platform config dir | `directories` has no Android support; use `AndroidApp::internal_data_path()` |
| Tokens (`credentials.rs`) | System keychain, `tokens/` file fallback | The `tokens/` fallback in app-private storage first; Android Keystore through JNI later |
| Theme (`theme.rs`) | XDG portal on Linux | The system's night mode from android-activity's configuration |
| Update check (`update.rs`) | Checks GitHub releases | Off; the store updates the app |
| Window placement (`window.rs`) | Saved size and position | Not applicable |
| Device name (`gethostname`) | Host name | The phone's model name through JNI |

**Lifecycle.** Android destroys the window surface when the app goes to the background and when the screen rotates (unless the activity handles rotation itself, which it should). eframe recreates the surface; the app must pause playback or keep its audio going, keep its tokio tasks alive, and save the config when paused, since the process can be killed without warning after that.

## Touch UI

The cozy look stays. What changes is everything that assumes a mouse, a keyboard and a wide window. New pieces are reusable widgets in `widgets/`, mocked up before they're built.

- **Layout sizes:** compact (under 600 dp wide), medium and expanded. On compact screens a bottom bar (Home, Libraries, Search, Settings) replaces the top bar's buttons; shelves keep scrolling sideways; the library grid picks its column count from the width.
- **Touch targets:** at least 48 dp for buttons, cards and menu rows. Audit `widgets/` (`round_button`, `list_row`, `segmented`, `select`, `menu`).
- **Hover:** hover states and tooltips have no touch equivalent. The menus that only open by pointer (the item menu, the player's menus, the Settings pickers) need a tap; a long press on a card opens the item menu.
- **Scrolling:** kinetic scrolling by drag. `smooth_scroll.rs` is tuned for wheels; check that dragging a shelf sideways doesn't fight the page's vertical scroll.
- **Text input:** the soft keyboard has to open and close for `widgets::field` (search, sign-in, server address), including IME composition.
- **Back:** the system back gesture closes the top dialog or menu, else pushes `NavRequest::Back`.
- **Safe areas:** insets for the status bar, the gesture bar and display cutouts.
- **Player page** (`views/player.rs`): tap to show and hide the controls, double-tap left or right to skip, drag sideways to seek with trickplay thumbnails, drag up and down for volume (right side) and brightness (left side), landscape while playing, and the screen kept on.

## Video spike

About a week, on a throwaway branch: both players playing the dev server's media and a real 4K HEVC HDR file on a phone.

**libmpv.** mpv-android's prebuilt `libmpv.so` (with its ffmpeg), loaded by the existing `dlopen` loader in `crates/player/src/libmpv/`. Hardware decoding with `hwdec=mediacodec-copy` first. Zero-copy would render mpv's GL output into an AHardwareBuffer-backed EGLImage and import that buffer into wgpu's Vulkan device (`VK_ANDROID_external_memory_android_hardware_buffer`): a backend beside `egl.rs` and an importer beside `widgets/video/vulkan.rs`, the same shape as the Linux path. Keeps one player, the `Player` API as it is, ASS subtitles and the track handling.

**Media3 (ExoPlayer).** Kotlin glue driven over JNI, its frames going to an `ImageReader` whose AHardwareBuffers are imported the same way, or to a SurfaceView under a transparent egui surface. Better hardware decoding, HDR and audio passthrough, but a second implementation behind the `Player` API, and ASS subtitles are weaker.

**Compared on:** whether it plays at all on the test phone, CPU and battery at 1080p and 4K, dropped frames, seek time, subtitle rendering, APK size, and how much of the `player` crate's API it can keep.

The result goes in a *Decision* section at the top of this file.

## Milestones

| # | Milestone | Effort | Done when |
|---|---|---|---|
| A0 | **Hello Android** | S–M | An eframe window with the cozy backdrop runs on a phone; `android:build` and the CI APK work. |
| A1 | **Platform gaps** | M | The table above is handled; sign-in and Home work by touch, in the desktop layout. |
| A2 | **Video spike** | M | The decision is written down. |
| A3 | **Playback** | L | The chosen player plays with reporting, resume and tracks, and the touch player page. |
| A4 | **Touch layout** | L | Layout sizes, the bottom bar, touch targets, long-press menus, insets and the soft keyboard. |
| A5 | **Release** | M | A signed APK on F-Droid (and maybe the Play Store). |

After that: Android TV, picture-in-picture, background audio, downloads.

## Testing

- **Emulator, in WSL:** KVM is available (the user needs to be in the `kvm` group), so an x86_64 emulator runs at full speed. It runs headless (`-no-window`, nothing opens on the desktop) and draws in software (SwiftShader, which still offers Vulkan and GLES). Drive it with `adb shell input` (taps, swipes, text) and take screenshots with `adb exec-out screencap -p`. It's good for the app starting, sign-in and the touch layout, not for judging video. It takes 2–3 GB, so don't run it during a big cargo build.
- **Real phone, over Wi-Fi:** the phone's *Wireless debugging* (Developer options), paired from WSL with `adb pair` and `adb connect`; WSL's mirrored networking puts it on the phone's network. This is where playback, the video spike, battery, rotation and the real soft keyboard get tested.
- The `egui_kittest` UI tests keep running on the desktop; a compact-width variant of the page tests covers the phone layout.
- `TESTING.md` gets an Android section for what only a device can show: rotation, going to the background during playback, the soft keyboard, the back gesture.

## Risks

- **Text input:** egui's soft-keyboard and IME support on Android is the least mature part of the port. If NativeActivity can't do it, GameActivity or a JNI call to `InputMethodManager` can.
- **The touch layout touches nearly every page.** It's the largest part of the work.
- **Build memory:** Android builds are extra cargo builds in an 11 GB WSL; run one at a time.
