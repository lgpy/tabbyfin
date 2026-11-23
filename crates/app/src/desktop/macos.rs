//! macOS: an IOKit power assertion keeps the display on, and `souvlaki` drives Now Playing
//! (`MPNowPlayingInfoCenter`, which wants the main thread: `Desktop::update` runs on it).

use std::ffi::{CString, c_char, c_void};

use super::media_controls;
use super::{Controls, Inhibit, Link};

type CFStringRef = *const c_void;

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const ASSERTION_LEVEL_ON: u32 = 255;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(alloc: *const c_void, c_str: *const c_char, encoding: u32) -> CFStringRef;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(kind: CFStringRef, level: u32, name: CFStringRef, id: *mut u32) -> i32;
    fn IOPMAssertionRelease(id: u32) -> i32;
}

/// A `PreventUserIdleDisplaySleep` assertion, held while it's wanted.
pub struct Inhibitor {
    assertion: Option<u32>,
}

impl Inhibitor {
    pub fn new() -> Self {
        Self { assertion: None }
    }
}

impl Inhibit for Inhibitor {
    fn acquire(&mut self) {
        if self.assertion.is_some() {
            return;
        }
        let kind = CString::new("PreventUserIdleDisplaySleep").unwrap();
        let reason = CString::new("Playing video").unwrap();
        let mut id = 0u32;
        unsafe {
            let kind = CFStringCreateWithCString(std::ptr::null(), kind.as_ptr(), CF_STRING_ENCODING_UTF8);
            let reason = CFStringCreateWithCString(std::ptr::null(), reason.as_ptr(), CF_STRING_ENCODING_UTF8);
            let status = IOPMAssertionCreateWithName(kind, ASSERTION_LEVEL_ON, reason, &mut id);
            CFRelease(kind);
            CFRelease(reason);
            if status == 0 {
                self.assertion = Some(id);
            } else {
                tracing::info!(status, "couldn't keep the display awake");
            }
        }
    }

    fn release(&mut self) {
        if let Some(id) = self.assertion.take() {
            unsafe { IOPMAssertionRelease(id) };
        }
    }
}

pub fn controls(_frame: &eframe::Frame, link: &Link) -> Option<Box<dyn Controls>> {
    let link = Link {
        commands: link.commands.clone(),
        ctx: link.ctx.clone(),
    };
    Some(Box::new(media_controls::start(None, link)?))
}
