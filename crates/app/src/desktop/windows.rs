//! Windows: `SetThreadExecutionState` keeps the display on, `souvlaki` drives the media overlay
//! (SMTC, attached to the main window), and `ITaskbarList3` shows the position on the
//! taskbar button.

use std::ffi::c_void;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Power::{ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState};
use windows::Win32::UI::Shell::{ITaskbarList3, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED, TaskbarList};

use super::media_controls::{self, SystemControls};
use super::{Controls, Inhibit, Link, MediaInfo, PlaybackState, Transport};

/// The display-required request. It belongs to the thread that made it, so it's made and
/// released on the UI thread, which is where `Desktop::update` runs.
pub struct Inhibitor;

impl Inhibitor {
    pub fn new() -> Self {
        Self
    }
}

impl Inhibit for Inhibitor {
    fn acquire(&mut self) {
        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED) };
    }

    fn release(&mut self) {
        unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
    }
}

/// The media overlay and the taskbar button, both on the main window.
pub fn controls(frame: &eframe::Frame, link: &Link) -> Option<Box<dyn Controls>> {
    let hwnd = match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => HWND(handle.hwnd.get() as *mut c_void),
        _ => return None,
    };
    let media = media_controls::start(
        Some(hwnd.0),
        Link {
            commands: link.commands.clone(),
            ctx: link.ctx.clone(),
        },
    )?;
    Some(Box::new(WindowControls {
        media,
        taskbar: Taskbar::new(hwnd),
    }))
}

struct WindowControls {
    media: SystemControls,
    taskbar: Option<Taskbar>,
}

impl Controls for WindowControls {
    fn set_metadata(&mut self, info: &MediaInfo) {
        self.media.set_metadata(info);
    }

    fn set_playback(&mut self, state: &PlaybackState) {
        self.media.set_playback(state);
        if let Some(taskbar) = &self.taskbar {
            taskbar.show(state);
        }
    }
}

/// The taskbar button's progress: the playback position, in the paused colour while paused,
/// cleared when this is dropped (when playback stops).
struct Taskbar {
    list: ITaskbarList3,
    hwnd: HWND,
}

impl Taskbar {
    fn new(hwnd: HWND) -> Option<Self> {
        unsafe {
            // The window's thread has COM already (winit starts it); this is for a thread that
            // doesn't, and fails harmlessly when it's started in another mode.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let list: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                .map_err(|err| tracing::info!("no taskbar progress: {err}"))
                .ok()?;
            list.HrInit().ok()?;
            Some(Self { list, hwnd })
        }
    }

    fn show(&self, state: &PlaybackState) {
        let Some(duration) = state.duration.filter(|duration| !duration.is_zero()) else {
            self.clear();
            return;
        };
        let flag = match state.transport {
            Transport::Playing => TBPF_NORMAL,
            Transport::Paused => TBPF_PAUSED,
        };
        let millis = |time: std::time::Duration| time.as_millis().min(u64::MAX as u128) as u64;
        unsafe {
            let _ = self.list.SetProgressState(self.hwnd, flag);
            let _ = self
                .list
                .SetProgressValue(self.hwnd, millis(state.position.min(duration)), millis(duration));
        }
    }

    fn clear(&self) {
        unsafe {
            let _ = self.list.SetProgressState(self.hwnd, TBPF_NOPROGRESS);
        }
    }
}

impl Drop for Taskbar {
    fn drop(&mut self) {
        self.clear();
    }
}
