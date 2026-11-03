# Android: plan for phones and tablets

**Goal:** Tabbyfin on Android phones and tablets, driven by touch. It's the same Rust and egui app as on the desktop, started through eframe's Android backend, so the API client, the pages, the cozy look (`DESIGN.md`), SyncPlay and remote control come along. There's no separate Kotlin UI.

**Status:** the app runs on the emulator in the desktop layout: sign-in by touch and the soft keyboard, Home, libraries and item pages, the system Back, and the session kept across restarts (A0 and A1 below; neither tried on a phone yet). Not done: the touch layout and playback. The video player is chosen by a spike (below) before playback work starts.

- **Phones and tablets first.** Android TV comes later; the keyboard cursor (`browse::Browse`, `keys.rs`) already suits a D-pad, so most of TV is a launcher entry and a 10-foot layout.
- **Devices:** Android 8 (API 26) and newer, where AHardwareBuffer and Vulkan 1.0 are available. arm64-v8a first, x86_64 for the emulator.

## Scope

**First release:** sign-in (password and Quick Connect, several accounts), Home, libraries, search, item pages, playback with server reporting, resume, audio and subtitle tracks, and Settings. Portrait and landscape on phones and tablets.

**Later:** SyncPlay and remote control (they compile as-is, but need care around the app going to the background), Android TV, picture-in-picture, background audio, downloads for offline viewing, Chromecast.

## Build and packaging

- `crates/app` is a library (`lib.rs`) that the desktop's `main.rs` runs. For Android, `cargo ndk … rustc --lib --crate-type cdylib` builds it as `libtabbyfin.so`; the crate itself stays `rlib`, so desktop builds don't link a second copy. `android_main(AndroidApp)` in `lib.rs` points `TABBYFIN_CONFIG_DIR` at the app's private storage, logs to logcat (`adb logcat -s tabbyfin`) and hands the `AndroidApp` to eframe through `NativeOptions`.
- **GameActivity**, through eframe's `android-game-activity` feature: it has the soft keyboard and IME that NativeActivity lacks, and eframe's AccessKit support (TalkBack) only builds with it. `packaging/android/` has the Java `MainActivity` (a bare `GameActivity` that loads the library) and depends on `androidx.games:games-activity:4.4.0`, the version android-activity 0.6 is written against. Don't enable GameActivity's prefab C++ glue; android-activity brings its own.
- **wgpu on Android:** the device asks for the adapter's own limits, since egui-wgpu's desktop defaults are more than phone GPUs (and the emulator) offer; and debug labels and validation are off, since the emulator's Vulkan driver crashes naming objects (`WGPU_DEBUG=1` turns them back on). On the emulator the app picks GL: its software Vulkan draws the first frames, then times out getting every frame after ("Dropped frame with error: Timeout").
- **What winit 0.30 drops** (`android.rs`, with `MainActivity.java` reporting it): the window insets (system bars, cutouts, the soft keyboard) become egui's safe area, and `App::ui` lays every screen out in `ctx.content_rect()` while the backdrop fills the window (`backdrop::fill_window`); the keyboard's text, which GameActivity keeps a copy of, is diffed against what it held last and passed on as Backspace presses, an IME commit and the word being composed; the keyboard's action key (Done) is Enter; a password field switches the keyboard to a password one, which suggests and learns nothing (`set_ime_editor_info`, then `MainActivity` restarts the input); and the system theme is the activity's night mode. Keys the keyboard sends as key events (Backspace on an empty copy) come through winit. The keyboard's copy is only reset when the focus moves to another field, never while typing, which would lose what it typed in between. android-activity 0.6.1 builds a slice from a null pointer when the text is read before the keyboard has had any, so it's read only after Java reports a change.
- **Back and the lifecycle:** `MainActivity` takes the system Back. While the app has somewhere to go (a dialog, menu or focused field, the player, a page under this one; decided each frame in `App::ui`) it becomes Escape, which already closes, leaves or goes back; otherwise the app goes to the background (`moveTaskToBack`) rather than closing. The Rust app can run only once per process (eframe can't start a second event loop, and winit doesn't end the first when the activity is destroyed), so the process exits in `onDestroy`; the next launch restores the session.
- **No autofocus:** the sign-in form doesn't focus its first field at launch, since that opens the keyboard over a form not yet seen.
- **Environment:** an app gets no environment, so the system property `debug.tabbyfin.env` stands in for it: `adb shell setprop debug.tabbyfin.env "RUST_LOG=debug;WGPU_BACKEND=vulkan"`, then restart the app (the value needs quoting inside the adb shell when it has `;`).
- `packaging/android/` is a Gradle project (AGP 9.5, Gradle 9.8, JDK 21, all in `mise.toml` except AGP): the manifest (`io.github.lgpy.tabbyfin`, the Flatpak's ID; `INTERNET`, `WAKE_LOCK`, cleartext HTTP for LAN servers; rotation handled by the activity), the launcher icon at each density, made from `packaging/tabbyfin.png`. Gradle rather than `cargo-apk` or `xbuild`, because GameActivity, Media3 (if the spike picks it) and any Kotlin glue need it anyway.
- `packaging/android/build.sh [--release] [--abi …]` builds the library into `src/main/jniLibs/` (git-ignored) and packages it; mise tasks `android:build` (both ABIs) and `android:install` (the connected device's ABI, then starts it). `--release` optimises the library but still makes the debug-signed APK, about 75 MB for both ABIs, so it installs anywhere until there's a release key. A debug library is ~500 MB of debug info. Gradle repackages in place, so an incremental APK can look much bigger than its contents; `rm -rf packaging/android/build` shrinks it.
- CI's `android` job builds the `--release` APK on every push and uploads it.
- Release: a signing key, then F-Droid (fits GPL-3.0) and possibly the Play Store.

## What the desktop code assumes

Android is `target_os = "android"`, not `"linux"`. Code gated on `target_os = "linux"` (`ash`, `zbus`, the EGL video path) drops out, while code gated on `not(any(windows, target_os = "macos"))` applies and has to be checked case by case.

| Area | Desktop | Android |
|---|---|---|
| Player process (`player::is_host_process`, `host.rs`) | libmpv in a player host process, the same executable re-run | In-process, always (`Player::spawn`); there's no libmpv in the APK yet (A3) |
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

- **Layout sizes:** compact (under 600 dp wide), medium and expanded. On compact screens a bottom bar (Home, Libraries, Search, Settings) replaces the top bar's buttons; shelves keep scrolling sideways; the library grid picks its column count from the width.
- **Touch targets:** at least 48 dp for buttons, cards and menu rows. Audit `widgets/` (`round_button`, `list_row`, `segmented`, `select`, `menu`).
- **Hover:** hover states and tooltips have no touch equivalent. The menus that only open by pointer (the item menu, the player's menus, the Settings pickers) need a tap; a long press on a card opens the item menu.
- **Scrolling:** kinetic scrolling by drag. A drag on a shelf is the row's or the page's, by its first 8 points (`widgets/shelf.rs`), so the page scrolls from a shelf too; a vertical drag that starts on a shelf doesn't coast after the release yet. `smooth_scroll.rs` is tuned for wheels.
- **Text input:** done for the platform (above); left: the keyboard's kind per field (URL for the server address, no suggestions for user names) and a form taller than what the keyboard leaves scrolling its focused field into view.
- **Back:** done (above); left: predictive Back's animation.
- **Safe areas:** the insets are applied (above); left: dark or light status bar icons to match the theme (they're white on the light backdrop now), and the player drawing under the bars.
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
| A0 | **Hello Android** | S–M | An eframe window with the cozy backdrop runs on a phone; `android:build` and the CI APK work. Runs on the emulator; a phone and the first CI run are left. |
| A1 | **Platform gaps** | M | The table above is handled; sign-in and Home work by touch, in the desktop layout. Done on the emulator. |
| A2 | **Video spike** | M | The decision is written down. |
| A3 | **Playback** | L | The chosen player plays with reporting, resume and tracks, and the touch player page. |
| A4 | **Touch layout** | L | Layout sizes, the bottom bar, touch targets, long-press menus, insets and the soft keyboard. |
| A5 | **Release** | M | A signed APK on F-Droid (and maybe the Play Store). |

After that: Android TV, picture-in-picture, background audio, downloads.

## Testing

- **Emulator, on Windows:** the SDK's emulator, WHPX-accelerated on this machine, runs an x86_64 image at full speed. The AVD `tabbyfin` is a Pixel 7 on API 36 (`system-images;android-36;google_apis;x86_64`), and starts headless, drawing in software (SwiftShader, which still offers Vulkan and GLES):

  ```sh
  emulator -avd tabbyfin -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect
  ```

  It takes about two minutes to boot: `adb wait-for-device` waits for it, then `adb shell getprop sys.boot_completed` reads 1 once the launcher is up. Drive it with `adb shell input` (taps, swipes, text) and take screenshots with `adb exec-out screencap -p`. It's good for the app starting, sign-in and the touch layout, not for judging video. It takes 2–3 GB, so don't run it during a big cargo build, and stop it with `adb emu kill`. `ANDROID_HOME` and the SDK's `platform-tools`, `emulator` and `cmdline-tools/latest/bin` are on the user `PATH`; recreate the AVD with `avdmanager create avd -n tabbyfin -k "system-images;android-36;google_apis;x86_64" -d pixel_7`. `sdkmanager` and `avdmanager` need Java 17 or newer, and the default `java` here is 8, so set `JAVA_HOME` to a 17+ JDK when running them. (On Linux, KVM plays the same part, and the user needs to be in the `kvm` group.)
- **Real phone, over Wi-Fi:** the phone's *Wireless debugging* (Developer options), paired from the same machine with `adb pair` and `adb connect`. This is where playback, the video spike, battery, rotation and the real soft keyboard get tested.
- The `egui_kittest` UI tests keep running on the desktop; a compact-width variant of the page tests covers the phone layout.
- `TESTING.md` has an Android section for what only a phone can show.

## Risks

- **Text input:** the keyboard bridge (`android.rs`) is tried with Gboard on the emulator only; other keyboards (Samsung's, SwiftKey, CJK input methods that compose whole phrases) may report their text differently.
- **The touch layout touches nearly every page.** It's the largest part of the work.
- **Build memory:** Android builds are extra cargo builds on a machine that's already building the desktop app; run one at a time.
