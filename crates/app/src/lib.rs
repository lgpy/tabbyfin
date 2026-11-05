//! The app. `main.rs` runs it on the desktop ([`run`]); Android loads it as a shared library and
//! starts it through [`android_main`] (docs/android.md).

// Every module of the app is declared here.
#[cfg(target_os = "android")]
mod android;
mod app;
mod browse;
mod cast;
mod config;
mod credentials;
#[cfg(debug_assertions)]
mod debug;
mod images;
mod item_menu;
mod keyboard;
mod keys;
mod layout;
mod live;
mod markers;
mod motion;
mod nav;
mod playback;
mod remote;
mod runtime;
mod services;
mod session;
mod smooth_scroll;
mod syncplay;
mod task;
mod theme;
mod toasts;
mod tracks;
mod trickplay;
// The UI tests: `egui_kittest` driving the pages and screens (debug builds' tests only).
#[cfg(test)]
mod ui_tests;
mod update;
mod views;
mod widgets;
mod window;

use std::sync::Arc;

use egui_wgpu::wgpu;
use tracing_subscriber::EnvFilter;

/// Daily log files kept in [`config::log_dir`].
const LOG_FILES_KEPT: usize = 7;

/// Logs to stderr and to a daily file (release builds on Windows have no console, so the file
/// is what bug reports can attach). `RUST_LOG` sets the filter; the default is `info`. Keep the
/// returned guard alive: dropping it flushes and stops the file writer.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::prelude::*;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // The appender prunes old files on start and complains if the folder doesn't exist yet.
    let _ = std::fs::create_dir_all(config::log_dir());
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("tabbyfin")
        .filename_suffix("log")
        .max_log_files(LOG_FILES_KEPT)
        .build(config::log_dir());
    let (file_layer, guard, file_error) = match file {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (
                Some(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .fmt_fields(PlainFields)
                        .with_writer(writer),
                ),
                Some(guard),
                None,
            )
        }
        Err(err) => (None, None, Some(err)),
    };
    // Android has no terminal; its log is logcat (`adb logcat -s tabbyfin`).
    #[cfg(not(target_os = "android"))]
    let terminal = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);
    #[cfg(target_os = "android")]
    let terminal = paranoid_android::layer("tabbyfin").with_ansi(false);
    tracing_subscriber::registry()
        .with(filter)
        .with(terminal)
        .with(file_layer)
        .init();
    if let Some(err) = file_error {
        tracing::warn!("not logging to a file in {}: {err}", config::log_dir().display());
    }
    guard
}

/// The default field format, as a type of its own. Layers cache formatted span fields by
/// formatter type, so with a shared type the file would get the terminal layer's colour codes.
struct PlainFields;

impl<'writer> tracing_subscriber::fmt::FormatFields<'writer> for PlainFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: tracing_subscriber::fmt::format::Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        tracing_subscriber::fmt::format::DefaultFields::new().format_fields(writer, fields)
    }
}

/// Logging in the player host process: plain lines on stderr, which the app reads and logs
/// (with its own timestamps) under the `player` target.
fn init_host_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();
}

/// The desktop app: `main`.
pub fn run() {
    // This executable is also the player host process (`::player::host`).
    if ::player::is_host_process() {
        init_host_logging();
        std::process::exit(::player::host::run());
    }
    let _log_guard = init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting tabbyfin");

    #[cfg(windows)]
    window::become_dpi_aware();
    let config = config::Config::load();
    let launch = window::launch(&config);
    let options = eframe::NativeOptions {
        viewport: window::viewport(launch),
        // Centred on the first launch, and when the saved spot is on no monitor any more
        // (Windows, checked before the window exists); a saved spot is used otherwise.
        centered: launch.is_none(),
        wgpu_options: wgpu_options(),
        // The mascot's meshes are tessellated by lyon, not epaint, so nothing feathers their
        // edges; multisampling smooths them.
        multisampling: 4,
        ..Default::default()
    };
    let placement = window::Placement::new(launch);
    let result = eframe::run_native(
        "Tabbyfin",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, config, placement)))),
    );
    if let Err(err) = result {
        tracing::error!("couldn't open the window: {err}");
        std::process::exit(1);
    }
}

/// Android's entry point, called by android-activity on its own thread when the activity starts.
/// The window is the activity's, so there's no placement to restore.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: winit::platform::android::activity::AndroidApp) {
    // The config, logs and image cache go in the app's private storage: `directories` knows no
    // Android folders.
    if let Some(dir) = android_app.internal_data_path() {
        // libass (mpv's subtitles) finds the system's fonts through fontconfig, which has no
        // configuration on Android: point it at one listing them, as Findroid does.
        let fonts = dir.join("fonts.conf");
        if let Err(err) = std::fs::write(&fonts, fonts_conf(&dir.join("fontconfig-cache"))) {
            eprintln!("couldn't write {}: {err}", fonts.display());
        }
        // SAFETY: set before any of the app's threads start; nothing else reads them.
        unsafe {
            std::env::set_var("TABBYFIN_CONFIG_DIR", &dir);
            std::env::set_var("FONTCONFIG_FILE", &fonts);
        }
    }
    android::env_from_property();
    let _log_guard = init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting tabbyfin");
    android::init(android_app.clone());
    // SAFETY: the activity's JavaVM and a global reference to it, valid for the life of the
    // process.
    unsafe { player::android::init(android_app.vm_as_ptr(), android_app.activity_as_ptr()) };

    let config = config::Config::load();
    let options = eframe::NativeOptions {
        android_app: Some(android_app),
        // A see-through surface, over the video's (`MainActivity`, `player::android`).
        viewport: egui::ViewportBuilder::default().with_transparent(true),
        wgpu_options: wgpu_options(),
        multisampling: 4,
        ..Default::default()
    };
    let result = eframe::run_native(
        "Tabbyfin",
        options,
        Box::new(|cc| {
            android::set_context(cc.egui_ctx.clone());
            Ok(Box::new(app::App::new(cc, config, window::Placement::new(None))))
        }),
    );
    if let Err(err) = result {
        tracing::error!("couldn't open the window: {err}");
    }
}

/// A fontconfig configuration for the system's fonts on Android, with its cache in `cache`.
#[cfg(target_os = "android")]
fn fonts_conf(cache: &std::path::Path) -> String {
    format!(
        r#"<fontconfig>
    <dir>/system/fonts/</dir>
    <dir>/product/fonts/</dir>
    <cachedir>{}</cachedir>
    <alias><family>serif</family><prefer><family>Noto Serif</family></prefer></alias>
    <alias><family>sans-serif</family><prefer><family>Roboto</family><family>Noto Sans</family></prefer></alias>
    <alias><family>monospace</family><prefer><family>Droid Sans Mono</family></prefer></alias>
</fontconfig>
"#,
        cache.display()
    )
}

/// The graphics API per OS. mpv's frames are shown without a copy by importing the player's
/// textures into wgpu's device (docs/in-app-video.md), so the API matters: the player shares
/// D3D11 textures, which D3D12 opens, and Vulkan memory, which Vulkan imports.
#[cfg(windows)]
const BACKENDS: wgpu::Backends = wgpu::Backends::DX12;
#[cfg(target_os = "macos")]
const BACKENDS: wgpu::Backends = wgpu::Backends::METAL;
/// GL is the fallback where Vulkan is missing (some VMs, old drivers); only software video
/// frames work there. wgpu prefers Vulkan when both run on the same kind of device.
#[cfg(not(any(windows, target_os = "macos")))]
const BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN.union(wgpu::Backends::GL);

/// egui-wgpu's setup with our backends (`WGPU_BACKEND` still overrides them, for testing), and a
/// device that can import Vulkan memory where the adapter supports it: the device has to be
/// created with that feature for the player's frames to be imported later.
fn wgpu_options() -> egui_wgpu::WgpuConfiguration {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::from_env().unwrap_or(BACKENDS);
    // No debug labels or validation on Android: the emulator's Vulkan driver crashes naming
    // objects, and phones have no validation layers. `WGPU_DEBUG` and `WGPU_VALIDATION` still
    // turn them on.
    #[cfg(target_os = "android")]
    {
        setup.instance_descriptor.flags = wgpu::InstanceFlags::empty().with_env();
    }
    let default_device = setup.device_descriptor.clone();
    setup.device_descriptor = Arc::new(move |adapter| {
        let mut descriptor = default_device(adapter);
        descriptor.required_features |= adapter.features() & wgpu::Features::VULKAN_EXTERNAL_MEMORY_FD;
        // egui-wgpu asks for wgpu's desktop limits, which phone GPUs (and the emulator's) fall
        // short of; any of the adapter's own limits will do for egui.
        #[cfg(target_os = "android")]
        {
            descriptor.required_limits = adapter.limits();
        }
        descriptor
    });
    egui_wgpu::WgpuConfiguration {
        wgpu_setup: setup.into(),
        ..Default::default()
    }
}
