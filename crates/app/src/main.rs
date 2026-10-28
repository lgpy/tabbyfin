// Release builds on Windows run without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Every module of the egui app is declared here, including those later tasks fill, so they
// never need to edit this file. Each one's doc comment says which task brings it.
mod app;
mod browse;
mod config;
mod credentials;
#[cfg(debug_assertions)]
mod debug;
mod images;
mod item_menu;
mod keys;
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
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
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

fn main() {
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
    let default_device = setup.device_descriptor.clone();
    setup.device_descriptor = Arc::new(move |adapter| {
        let mut descriptor = default_device(adapter);
        descriptor.required_features |= adapter.features() & wgpu::Features::VULKAN_EXTERNAL_MEMORY_FD;
        descriptor
    });
    egui_wgpu::WgpuConfiguration {
        wgpu_setup: setup.into(),
        ..Default::default()
    }
}
