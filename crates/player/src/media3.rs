//! Media3's ExoPlayer behind the [`Player`](crate::Player) API, Android's default player
//! (docs/android.md, *Decision*). `Media3Bridge.java` runs ExoPlayer on its own thread; this side
//! turns the app's mpv commands and properties into the bridge's JSON commands, and the bridge's
//! callbacks into [`PlayerEvent`]s.
//!
//! ExoPlayer draws into the video surface (`crate::android`) itself. Subtitles don't go there:
//! their cues come back as [`PlayerEvent::Cues`] for the UI to draw. Track ids are mpv's: the
//! Nth audio or subtitle track of the file, external subtitles after the embedded ones.

use std::ffi::{CStr, CString, c_void};
use std::ptr;
use std::sync::{Arc, Mutex};

use jni_sys::{JNIEnv, jclass, jdouble, jint, jlong, jobject, jobjectArray, jstring, jvalue};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::android::{self, SurfaceRef, check, load_class, with_env};
use crate::libmpv::VideoOutput;
use crate::{Cue, CueImage, Cues, EndReason, Error, PlayerEvent, Result};

/// What the bridge's callbacks reach, through the handle they're given. Leaked: ExoPlayer's
/// thread may call back after the player is dropped, and it's a few bytes per playback.
struct Shared {
    events: mpsc::UnboundedSender<PlayerEvent>,
}

/// A global reference to the `Media3Bridge`, shared with the video surface's sink.
struct Bridge(jobject);

// SAFETY: a JNI global reference, usable from any attached thread.
unsafe impl Send for Bridge {}
unsafe impl Sync for Bridge {}

impl Bridge {
    /// Calls one of the bridge's `void` methods.
    fn call(&self, method: &CStr, signature: &CStr, args: &[jvalue]) -> Result<()> {
        with_env(|env, _| unsafe {
            let f = &**env;
            let class = (f.v1_1.GetObjectClass)(env, self.0);
            let id = (f.v1_1.GetMethodID)(env, class, method.as_ptr(), signature.as_ptr());
            (f.v1_1.DeleteLocalRef)(env, class);
            check(env, "Media3Bridge method lookup")?;
            (f.v1_1.CallVoidMethodA)(env, self.0, id, args.as_ptr());
            check(env, "Media3Bridge")
        })
    }

    fn send(&self, command: &Value) -> Result<()> {
        let text = CString::new(command.to_string()).expect("JSON has no nul bytes");
        with_env(|env, _| unsafe {
            let string = ((**env).v1_1.NewStringUTF)(env, text.as_ptr());
            let result = self.call(c"command", c"(Ljava/lang/String;)V", &[jvalue { l: string }]);
            ((**env).v1_1.DeleteLocalRef)(env, string);
            result
        })
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = with_env(|env, _| {
            // SAFETY: our global reference, not used again.
            unsafe { ((**env).v1_1.DeleteGlobalRef)(env, self.0) };
            Ok(())
        });
    }
}

pub(crate) struct Media3Player {
    bridge: Arc<Bridge>,
    shared: &'static Shared,
    /// `http-header-fields` and `start`, which mpv takes as properties before `loadfile`.
    pending: Mutex<(Vec<String>, f64)>,
}

type Started = (Media3Player, mpsc::UnboundedReceiver<PlayerEvent>, VideoOutput);

impl Media3Player {
    pub fn spawn() -> Result<Started> {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let shared: &'static Shared = Box::leak(Box::new(Shared { events: events_tx }));
        let bridge = Arc::new(with_env(|env, activity| unsafe {
            create_bridge(env, activity, shared as *const Shared as jlong)
        })?);
        // ExoPlayer draws into the video surface whenever it exists, and lets go of it before
        // the surface goes.
        let sink = bridge.clone();
        android::show_surface(move |surface: Option<SurfaceRef>| {
            let surface = surface.map_or(ptr::null_mut(), |s| s as jobject);
            if let Err(err) = sink.call(c"surface", c"(Landroid/view/Surface;)V", &[jvalue { l: surface }]) {
                tracing::warn!("Media3 and the video surface: {err}");
            }
        })?;
        let player = Self {
            bridge,
            shared,
            pending: Mutex::new((Vec::new(), 0.)),
        };
        Ok((player, events_rx, VideoOutput::external()))
    }

    /// An mpv command, as the app sends it.
    pub fn command(&self, args: &[Value]) -> Result<()> {
        let arg = |i: usize| args.get(i).and_then(Value::as_str);
        match arg(0).unwrap_or_default() {
            "loadfile" => {
                let (headers, start) = self.pending.lock().unwrap().clone();
                self.send(json!({"op": "load", "url": arg(1), "headers": headers, "start": start}))
            }
            "cycle" if arg(1) == Some("pause") => self.send(json!({"op": "toggle_pause"})),
            "cycle" if arg(1) == Some("mute") => self.send(json!({"op": "toggle_mute"})),
            "seek" => {
                let secs = args.get(1).and_then(number).unwrap_or(0.);
                let relative = arg(2) != Some("absolute");
                self.send(json!({"op": "seek", "secs": secs, "relative": relative}))
            }
            // `sub-add <url> <flag> <title> <lang>`
            "sub-add" => self.send(json!({
                "op": "sub_add",
                "url": arg(1),
                "select": arg(2) == Some("select"),
                "title": arg(3),
                "language": arg(4).filter(|l| !l.is_empty()),
            })),
            "quit" | "stop" => {
                self.send(json!({"op": "release"}))?;
                let _ = self.shared.events.send(PlayerEvent::EndFile(EndReason::Quit));
                let _ = self.shared.events.send(PlayerEvent::Exited);
                Ok(())
            }
            _ => {
                tracing::warn!(?args, "Media3 doesn't take this command");
                Ok(())
            }
        }
    }

    /// An mpv property, as the app sets it.
    pub fn set_property(&self, name: &str, value: &Value) -> Result<()> {
        match name {
            "http-header-fields" => {
                self.pending.lock().unwrap().0 = serde_json::from_value(value.clone()).unwrap_or_default();
                Ok(())
            }
            "start" => {
                self.pending.lock().unwrap().1 = number(value).unwrap_or(0.);
                Ok(())
            }
            "pause" | "mute" => self.send(json!({"op": name, "value": flag(value)})),
            "volume" | "speed" => match number(value) {
                Some(n) => self.send(json!({"op": name, "value": n})),
                None => Err(rejected(name, value)),
            },
            "aid" | "sid" => {
                self.send(json!({"op": name, "id": track_id(value).ok_or_else(|| rejected(name, value))?}))
            }
            "sub-delay" | "audio-delay" => match number(value) {
                Some(secs) => self.send(json!({"op": name.replace('-', "_"), "secs": secs})),
                None => Err(rejected(name, value)),
            },
            // The UI draws the subtitles, in its own style and size, and shows the title.
            "force-media-title" | "sub-scale" | "sub-color" | "sub-border-style" | "sub-back-color" => Ok(()),
            _ => {
                tracing::warn!(name, ?value, "Media3 doesn't take this property");
                Ok(())
            }
        }
    }

    fn send(&self, command: Value) -> Result<()> {
        self.bridge.send(&command)
    }
}

impl Drop for Media3Player {
    fn drop(&mut self) {
        let _ = self.send(json!({"op": "release"}));
        android::hide_surface();
    }
}

/// A number, or a string holding one (mpv takes both).
fn number(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}

/// A flag, as a bool or mpv's `yes`/`no`.
fn flag(value: &Value) -> bool {
    value.as_bool().unwrap_or_else(|| value.as_str() == Some("yes"))
}

/// mpv's track id as the bridge takes it: N from 1, 0 for `auto`, -1 for `no`.
fn track_id(value: &Value) -> Option<i64> {
    match value.as_str() {
        Some("auto") => Some(0),
        Some("no") => Some(-1),
        _ => number(value).map(|n| n as i64).filter(|&n| n >= 1),
    }
}

fn rejected(name: &str, value: &Value) -> Error {
    Error::Command {
        command: format!("set {name} {value}"),
        message: "invalid value".into(),
    }
}

/// `Media3Bridge.create(activity, handle)`, as a global reference.
unsafe fn create_bridge(env: *mut JNIEnv, activity: jobject, handle: jlong) -> Result<Bridge> {
    // SAFETY: a valid env for this thread and a valid activity reference.
    unsafe {
        let f = &**env;
        let class = load_class(env, activity, c"io.github.lgpy.tabbyfin.Media3Bridge")?;
        register_natives(env, class)?;
        let create = (f.v1_1.GetStaticMethodID)(
            env,
            class,
            c"create".as_ptr(),
            c"(Landroid/content/Context;J)Lio/github/lgpy/tabbyfin/Media3Bridge;".as_ptr(),
        );
        let args = [jvalue { l: activity }, jvalue { j: handle }];
        let bridge = (f.v1_1.CallStaticObjectMethodA)(env, class, create, args.as_ptr());
        check(env, "Media3Bridge.create")?;
        let global = (f.v1_1.NewGlobalRef)(env, bridge);
        for local in [class, bridge] {
            (f.v1_1.DeleteLocalRef)(env, local);
        }
        Ok(Bridge(global))
    }
}

unsafe fn register_natives(env: *mut JNIEnv, class: jclass) -> Result<()> {
    let methods = [
        jni_sys::JNINativeMethod {
            name: c"nativeEvent".as_ptr() as *mut _,
            signature: c"(JID)V".as_ptr() as *mut _,
            fnPtr: native_event as *mut c_void,
        },
        jni_sys::JNINativeMethod {
            name: c"nativeError".as_ptr() as *mut _,
            signature: c"(JLjava/lang/String;)V".as_ptr() as *mut _,
            fnPtr: native_error as *mut c_void,
        },
        jni_sys::JNINativeMethod {
            name: c"nativeCues".as_ptr() as *mut _,
            signature: c"(JLjava/lang/String;[Ljava/lang/Object;)V".as_ptr() as *mut _,
            fnPtr: native_cues as *mut c_void,
        },
    ];
    // SAFETY: the signatures match the functions below.
    unsafe {
        ((**env).v1_1.RegisterNatives)(env, class, methods.as_ptr(), methods.len() as jint);
        check(env, "RegisterNatives(Media3Bridge)")
    }
}

/// Event kinds, as `Media3Bridge.java` numbers them.
extern "system" fn native_event(_env: *mut JNIEnv, _class: jclass, handle: jlong, kind: jint, value: jdouble) {
    // SAFETY: the handle is a leaked `Shared`.
    let shared = unsafe { &*(handle as *const Shared) };
    let secs = || std::time::Duration::from_secs_f64(value.max(0.));
    let event = match kind {
        1 => PlayerEvent::FileLoaded,
        2 => PlayerEvent::Position(secs()),
        3 => PlayerEvent::Duration(secs()),
        4 => PlayerEvent::Paused(value != 0.),
        5 => PlayerEvent::Volume(value),
        6 => PlayerEvent::Muted(value != 0.),
        7 => PlayerEvent::Seeked,
        8 => PlayerEvent::Restarted,
        9 => PlayerEvent::EndFile(EndReason::Eof),
        _ => return,
    };
    let _ = shared.events.send(event);
}

/// Playback failed.
extern "system" fn native_error(env: *mut JNIEnv, _class: jclass, handle: jlong, message: jstring) {
    // SAFETY: the handle is a leaked `Shared`; `message` a valid string.
    let shared = unsafe { &*(handle as *const Shared) };
    let message = unsafe { string(env, message) };
    let _ = shared.events.send(PlayerEvent::EndFile(EndReason::Error(message)));
}

/// One cue, as the bridge describes it in JSON; an image's pixels are in the array beside it.
#[derive(Deserialize)]
struct CueJson {
    text: Option<String>,
    /// The index of its ARGB pixels in the images array.
    image: Option<usize>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    rect: [f32; 4],
}

#[derive(Deserialize)]
struct CuesJson {
    cues: Vec<CueJson>,
    aspect: f32,
}

/// The cues to show now. `images` holds an `int[]` of ARGB pixels per image cue.
extern "system" fn native_cues(env: *mut JNIEnv, _class: jclass, handle: jlong, json: jstring, images: jobjectArray) {
    // SAFETY: the handle is a leaked `Shared`; the arguments are valid local references.
    let shared = unsafe { &*(handle as *const Shared) };
    let parsed: CuesJson = match serde_json::from_str(&unsafe { string(env, json) }) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::warn!("Media3's cues: {err}");
            return;
        }
    };
    let cues = parsed
        .cues
        .into_iter()
        .filter_map(|cue| match (cue.text, cue.image) {
            (_, Some(index)) => {
                let rgba = unsafe { argb_to_rgba(env, images, index) }?;
                (rgba.len() == (cue.width * cue.height * 4) as usize).then_some(Cue::Image(CueImage {
                    width: cue.width,
                    height: cue.height,
                    rgba,
                    rect: cue.rect,
                }))
            }
            (Some(text), None) => Some(Cue::Text(text)),
            (None, None) => None,
        })
        .collect();
    let _ = shared.events.send(PlayerEvent::Cues(Cues {
        cues,
        aspect: parsed.aspect,
    }));
}

/// A Java string's contents.
unsafe fn string(env: *mut JNIEnv, s: jstring) -> String {
    if s.is_null() {
        return String::new();
    }
    // SAFETY: a valid env and string.
    unsafe {
        let f = &**env;
        let chars = (f.v1_1.GetStringUTFChars)(env, s, ptr::null_mut());
        let text = CStr::from_ptr(chars).to_string_lossy().into_owned();
        (f.v1_1.ReleaseStringUTFChars)(env, s, chars);
        text
    }
}

/// Element `index` of `images`, an `int[]` of ARGB pixels, as RGBA bytes.
unsafe fn argb_to_rgba(env: *mut JNIEnv, images: jobjectArray, index: usize) -> Option<Vec<u8>> {
    // SAFETY: a valid env and array; the element is an int[].
    unsafe {
        let f = &**env;
        if images.is_null() || index as jint >= (f.v1_1.GetArrayLength)(env, images) {
            return None;
        }
        let pixels = (f.v1_1.GetObjectArrayElement)(env, images, index as jint);
        let len = (f.v1_1.GetArrayLength)(env, pixels) as usize;
        let mut argb = vec![0i32; len];
        (f.v1_1.GetIntArrayRegion)(env, pixels, 0, len as jint, argb.as_mut_ptr());
        (f.v1_1.DeleteLocalRef)(env, pixels);
        Some(
            argb.into_iter()
                .flat_map(|p| {
                    let [a, r, g, b] = (p as u32).to_be_bytes();
                    [r, g, b, a]
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_ids() {
        assert_eq!(track_id(&json!(2)), Some(2));
        assert_eq!(track_id(&json!("3")), Some(3));
        assert_eq!(track_id(&json!("no")), Some(-1));
        assert_eq!(track_id(&json!("auto")), Some(0));
        assert_eq!(track_id(&json!(0)), None);
    }
}
