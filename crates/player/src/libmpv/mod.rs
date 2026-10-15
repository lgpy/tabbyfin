//! mpv embedded in the app through libmpv, with video drawn inside our window.
//! See `docs/in-app-video.md`.
//!
//! libmpv is loaded with `dlopen` at runtime, so builds don't need it; without it the app shows
//! an error when playback starts.

#[cfg(windows)]
mod angle;
#[cfg(target_os = "linux")]
mod egl;
mod embedded;
#[cfg(any(windows, target_os = "linux"))]
mod opengl;
mod video;
#[cfg(windows)]
mod wgl;

use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};

use libloading::Library;

pub use embedded::EmbeddedPlayer;
pub use video::{Frame, SharedFrame, VideoFrame, VideoOutput};

use crate::Error;

/// Render parameter for `mpv_render_context_create` / `_render`.
#[repr(C)]
pub(crate) struct RenderParam {
    pub kind: c_int,
    pub data: *mut c_void,
}

/// Leading fields of `mpv_event`.
#[repr(C)]
pub(crate) struct RawEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

/// `mpv_event_property`.
#[repr(C)]
pub(crate) struct RawProperty {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

/// `mpv_event_log_message`.
#[repr(C)]
pub(crate) struct RawLogMessage {
    pub prefix: *const c_char,
    pub level: *const c_char,
    pub text: *const c_char,
    pub log_level: c_int,
}

/// Leading fields of `mpv_event_end_file`.
#[repr(C)]
pub(crate) struct RawEndFile {
    pub reason: c_int,
    pub error: c_int,
}

pub(crate) const FORMAT_FLAG: c_int = 3;
pub(crate) const FORMAT_INT64: c_int = 4;
pub(crate) const FORMAT_DOUBLE: c_int = 5;

/// The subset of the libmpv C API we use.
pub(crate) struct Api {
    _lib: Library,
    pub create: unsafe extern "C" fn() -> *mut c_void,
    pub set_option_string: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    pub initialize: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub command: unsafe extern "C" fn(*mut c_void, *mut *const c_char) -> c_int,
    pub set_property_string: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    pub get_property: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int,
    pub get_property_string: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char,
    pub free: unsafe extern "C" fn(*mut c_void),
    pub request_log_messages: unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int,
    pub observe_property: unsafe extern "C" fn(*mut c_void, u64, *const c_char, c_int) -> c_int,
    pub wait_event: unsafe extern "C" fn(*mut c_void, c_double) -> *const RawEvent,
    pub wakeup: unsafe extern "C" fn(*mut c_void),
    pub error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    pub terminate_destroy: unsafe extern "C" fn(*mut c_void),
    pub render_create: unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *mut RenderParam) -> c_int,
    pub render_set_update_callback:
        unsafe extern "C" fn(*mut c_void, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    pub render_update: unsafe extern "C" fn(*mut c_void) -> u64,
    pub render_render: unsafe extern "C" fn(*mut c_void, *mut RenderParam) -> c_int,
    pub render_free: unsafe extern "C" fn(*mut c_void),
}

const LIBRARY_NAMES: &[&str] = if cfg!(windows) {
    &["libmpv-2.dll", "mpv-2.dll"]
} else if cfg!(target_os = "macos") {
    &["libmpv.2.dylib", "libmpv.dylib"]
} else {
    &["libmpv.so.2", "libmpv.so.1", "libmpv.so"]
};

/// Extra folders to search on macOS: Homebrew (`brew install mpv`) on Apple silicon and Intel,
/// which aren't on the default library path.
const EXTRA_DIRS: &[&str] = if cfg!(target_os = "macos") {
    &["/opt/homebrew/lib", "/usr/local/lib"]
} else {
    &[]
};

impl Api {
    pub fn load() -> Result<Self, Error> {
        // Bundled first, any version: next to the executable (Windows), or in ../lib (the
        // Linux AppImage's usr/bin and usr/lib, which may carry libmpv.so.1 where the system
        // has libmpv.so.2). Then the system search path, then the extra folders.
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.to_owned()));
        let bundled = exe_dir.iter().flat_map(|dir| {
            LIBRARY_NAMES
                .iter()
                .flat_map(move |name| [dir.join(name), dir.join("../lib").join(name)])
        });
        let system = LIBRARY_NAMES.iter().map(std::path::PathBuf::from);
        let extra = EXTRA_DIRS.iter().flat_map(|dir| {
            LIBRARY_NAMES
                .iter()
                .map(move |name| std::path::Path::new(dir).join(name))
        });
        let mut last_error = String::new();
        for candidate in bundled.chain(system).chain(extra) {
            // SAFETY: loading libmpv runs no unexpected initialization code.
            match unsafe { Library::new(candidate.as_os_str()) } {
                Ok(lib) => return Self::from_library(lib),
                Err(err) => last_error = err.to_string(),
            }
        }
        Err(Error::LibmpvUnavailable(last_error))
    }

    fn from_library(lib: Library) -> Result<Self, Error> {
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: the declared types match the libmpv C API for these symbols.
                *unsafe { lib.get($name) }.map_err(|e| Error::LibmpvUnavailable(e.to_string()))?
            };
        }
        Ok(Self {
            create: sym!(b"mpv_create\0"),
            set_option_string: sym!(b"mpv_set_option_string\0"),
            initialize: sym!(b"mpv_initialize\0"),
            command: sym!(b"mpv_command\0"),
            set_property_string: sym!(b"mpv_set_property_string\0"),
            get_property: sym!(b"mpv_get_property\0"),
            get_property_string: sym!(b"mpv_get_property_string\0"),
            free: sym!(b"mpv_free\0"),
            request_log_messages: sym!(b"mpv_request_log_messages\0"),
            observe_property: sym!(b"mpv_observe_property\0"),
            wait_event: sym!(b"mpv_wait_event\0"),
            wakeup: sym!(b"mpv_wakeup\0"),
            error_string: sym!(b"mpv_error_string\0"),
            terminate_destroy: sym!(b"mpv_terminate_destroy\0"),
            render_create: sym!(b"mpv_render_context_create\0"),
            render_set_update_callback: sym!(b"mpv_render_context_set_update_callback\0"),
            render_update: sym!(b"mpv_render_context_update\0"),
            render_render: sym!(b"mpv_render_context_render\0"),
            render_free: sym!(b"mpv_render_context_free\0"),
            _lib: lib,
        })
    }

    pub fn error_message(&self, code: c_int) -> String {
        // SAFETY: mpv_error_string returns a static string for any code.
        unsafe { CStr::from_ptr((self.error_string)(code)) }
            .to_string_lossy()
            .into_owned()
    }

    pub fn check(&self, command: &str, code: c_int) -> Result<(), Error> {
        if code >= 0 {
            return Ok(());
        }
        Err(Error::Command {
            command: command.to_owned(),
            message: self.error_message(code),
        })
    }
}

pub(crate) fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).expect("nul bytes removed")
}

/// A libmpv core handle. Destroyed (with the core) when the last reference goes away.
pub(crate) struct Core {
    pub api: Api,
    pub mpv: *mut c_void,
}

// SAFETY: the mpv client API is thread-safe for all functions we call on `mpv`.
unsafe impl Send for Core {}
unsafe impl Sync for Core {}

impl Core {
    pub fn command(&self, args: &[&str]) -> Result<(), Error> {
        let owned: Vec<CString> = args.iter().map(|a| cstring(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: null-terminated array of C strings that outlive the call.
        let code = unsafe { (self.api.command)(self.mpv, ptrs.as_mut_ptr()) };
        self.api.check(args.first().copied().unwrap_or("command"), code)
    }

    pub fn set_property(&self, name: &str, value: &str) -> Result<(), Error> {
        let (cname, cvalue) = (cstring(name), cstring(value));
        // SAFETY: valid C strings for the duration of the call.
        let code = unsafe { (self.api.set_property_string)(self.mpv, cname.as_ptr(), cvalue.as_ptr()) };
        self.api.check(&format!("set {name}"), code)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let name = cstring(name);
        let mut value: i64 = 0;
        // SAFETY: MPV_FORMAT_INT64 writes an i64 into `value`.
        let code = unsafe {
            (self.api.get_property)(
                self.mpv,
                name.as_ptr(),
                FORMAT_INT64,
                &mut value as *mut i64 as *mut c_void,
            )
        };
        (code >= 0).then_some(value)
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        let name = cstring(name);
        // SAFETY: valid C string; the result is a C string owned by us, freed with mpv_free.
        unsafe {
            let value = (self.api.get_property_string)(self.mpv, name.as_ptr());
            if value.is_null() {
                return None;
            }
            let string = CStr::from_ptr(value).to_string_lossy().into_owned();
            (self.api.free)(value as *mut c_void);
            Some(string)
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // SAFETY: last reference; every render context on this core has been freed.
        unsafe { (self.api.terminate_destroy)(self.mpv) };
    }
}
