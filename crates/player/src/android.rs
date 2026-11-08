//! Android glue: the app's JavaVM and activity, and the SurfaceView the player draws into.
//!
//! The video doesn't go through the UI on Android: the player draws into a SurfaceView under the
//! app's own surface (`VideoSurface.java`), and the UI leaves that part of its surface transparent
//! (docs/android.md, *Decision*). The UI says where the video goes ([`place_surface`]); the
//! player is told when the surface comes and goes.
//!
//! JNI goes through raw `jni-sys`. App classes are loaded through the activity's class loader,
//! since a native thread's `FindClass` only sees the system's.

use std::ffi::{CStr, c_void};
use std::ptr;
use std::sync::{Mutex, OnceLock};

use jni_sys::{JNI_OK, JNI_VERSION_1_6, JNIEnv, JavaVM, jclass, jint, jobject, jvalue};

use crate::{Error, Result};

/// The JavaVM and a global reference to the activity, as addresses.
static CONTEXT: OnceLock<(usize, usize)> = OnceLock::new();

/// Sets the app's JavaVM and activity. Called once, by `android_main`, before any player starts.
///
/// # Safety
/// Both are valid for the life of the process; `activity` is a JNI global reference.
pub unsafe fn init(vm: *mut c_void, activity: *mut c_void) {
    let _ = CONTEXT.set((vm as usize, activity as usize));
}

pub(crate) fn vm() -> Option<*mut JavaVM> {
    CONTEXT.get().map(|&(vm, _)| vm as *mut JavaVM)
}

fn context() -> Result<(*mut JavaVM, jobject)> {
    CONTEXT
        .get()
        .map(|&(vm, activity)| (vm as *mut JavaVM, activity as jobject))
        .ok_or_else(|| Error::Host("the Android context isn't set".into()))
}

/// Runs `f` with this thread's JNIEnv, attaching the thread for the call if it isn't (ART aborts
/// when an attached native thread exits, so it's detached again after).
pub(crate) fn with_env<T>(f: impl FnOnce(*mut JNIEnv, jobject) -> Result<T>) -> Result<T> {
    let (vm, activity) = context()?;
    // SAFETY: a valid JavaVM; the env is only used on this thread.
    unsafe {
        let mut env: *mut c_void = ptr::null_mut();
        let attached = ((**vm).v1_2.GetEnv)(vm, &mut env, JNI_VERSION_1_6) == JNI_OK;
        if !attached && ((**vm).v1_1.AttachCurrentThread)(vm, &mut env, ptr::null_mut()) != JNI_OK {
            return Err(Error::Host("couldn't attach to the JavaVM".into()));
        }
        let result = f(env as *mut JNIEnv, activity);
        if !attached {
            ((**vm).v1_1.DetachCurrentThread)(vm);
        }
        result
    }
}

/// Turns a pending Java exception into an error (and describes it in logcat).
pub(crate) unsafe fn check(env: *mut JNIEnv, what: &str) -> Result<()> {
    // SAFETY: a valid env for this thread.
    unsafe {
        if ((**env).v1_2.ExceptionCheck)(env) {
            ((**env).v1_1.ExceptionDescribe)(env);
            ((**env).v1_1.ExceptionClear)(env);
            return Err(Error::Host(format!("{what} threw")));
        }
    }
    Ok(())
}

/// One of the app's classes, through the activity's class loader. A local reference.
pub(crate) unsafe fn load_class(env: *mut JNIEnv, activity: jobject, name: &CStr) -> Result<jclass> {
    // SAFETY: a valid env for this thread and a valid activity reference.
    unsafe {
        let f = &**env;
        let activity_class = (f.v1_1.GetObjectClass)(env, activity);
        let get_loader = (f.v1_1.GetMethodID)(
            env,
            activity_class,
            c"getClassLoader".as_ptr(),
            c"()Ljava/lang/ClassLoader;".as_ptr(),
        );
        let loader = (f.v1_1.CallObjectMethodA)(env, activity, get_loader, ptr::null());
        check(env, "getClassLoader")?;
        let loader_class = (f.v1_1.GetObjectClass)(env, loader);
        let load = (f.v1_1.GetMethodID)(
            env,
            loader_class,
            c"loadClass".as_ptr(),
            c"(Ljava/lang/String;)Ljava/lang/Class;".as_ptr(),
        );
        let jname = (f.v1_1.NewStringUTF)(env, name.as_ptr());
        let class = (f.v1_1.CallObjectMethodA)(env, loader, load, [jvalue { l: jname }].as_ptr()) as jclass;
        check(env, "loadClass")?;
        for local in [activity_class, loader, loader_class, jname] {
            (f.v1_1.DeleteLocalRef)(env, local);
        }
        Ok(class)
    }
}

/// Runs `f` with `VideoSurface`'s class (its natives registered) and the activity.
fn with_video_surface<T>(f: impl FnOnce(*mut JNIEnv, jclass, jobject) -> Result<T>) -> Result<T> {
    with_env(|env, activity| unsafe {
        let class = load_class(env, activity, c"io.github.lgpy.tabbyfin.VideoSurface")?;
        let result = register_natives(env, class).and_then(|()| f(env, class, activity));
        ((**env).v1_1.DeleteLocalRef)(env, class);
        result
    })
}

/// Calls a static `void` method of `VideoSurface`.
fn call_video_surface(method: &CStr, signature: &CStr, args: &[jvalue]) -> Result<()> {
    with_video_surface(|env, class, _| unsafe {
        let f = &**env;
        let id = (f.v1_1.GetStaticMethodID)(env, class, method.as_ptr(), signature.as_ptr());
        (f.v1_1.CallStaticVoidMethodA)(env, class, id, args.as_ptr());
        check(env, "VideoSurface")
    })
}

/// Calls a static `void` method of one of the app's Java classes (`packaging/android/`), through
/// the activity's class loader as the surface's calls are. The method posts to the UI thread
/// itself, since this runs on the Rust app's thread.
fn call_app_static(class: &CStr, method: &CStr, signature: &CStr, args: &[jvalue]) -> Result<()> {
    with_env(|env, activity| unsafe {
        let class = load_class(env, activity, class)?;
        let f = &**env;
        let id = (f.v1_1.GetStaticMethodID)(env, class, method.as_ptr(), signature.as_ptr());
        // Before calling it: a name or signature the class doesn't have leaves it null, and
        // calling through null takes the process with it.
        let mut result = check(env, "the app's Java");
        if result.is_ok() {
            (f.v1_1.CallStaticVoidMethodA)(env, class, id, args.as_ptr());
            result = check(env, "the app's Java");
        }
        (f.v1_1.DeleteLocalRef)(env, class);
        result
    })
}

/// Calls a static `void` method of one of the app's Java classes with the activity (as its
/// `Context`), then `strings`, then `ints`, in that order: for the app's own Java outside the
/// player (the downloads' notification, `DownloadService.java`), so JNI stays in this module.
/// Callable from any thread.
pub fn call_with_context(class: &CStr, method: &CStr, signature: &CStr, strings: &[&str], ints: &[i32]) -> Result<()> {
    with_env(|env, activity| unsafe {
        let f = &**env;
        // UTF-16 rather than `NewStringUTF`, whose modified UTF-8 doesn't take what's outside
        // the BMP (an emoji in a title) the way Rust writes it.
        let mut locals = Vec::with_capacity(strings.len());
        for text in strings {
            let units: Vec<u16> = text.encode_utf16().collect();
            locals.push((f.v1_1.NewString)(env, units.as_ptr(), units.len() as jint));
        }
        let mut args = vec![jvalue { l: activity }];
        args.extend(locals.iter().map(|&l| jvalue { l }));
        args.extend(ints.iter().map(|&i| jvalue { i }));
        let result = call_app_static(class, method, signature, &args);
        for local in locals {
            (f.v1_1.DeleteLocalRef)(env, local);
        }
        result
    })
}

/// The status and navigation bar icons: dark over the app's light theme, light over the dark one.
/// `MainActivity` sets them on the UI thread, and keeps them while the player hides the bars.
pub fn light_system_bars(light: bool) {
    let result = context().and_then(|(_, activity)| {
        let args = [jvalue { l: activity }, jvalue { z: light }];
        call_app_static(
            c"io.github.lgpy.tabbyfin.MainActivity",
            c"setLightSystemBars",
            c"(Landroid/app/Activity;Z)V",
            &args,
        )
    });
    if let Err(err) = result {
        tracing::warn!("couldn't set the system bars' icons: {err}");
    }
}

/// The window while the player page is open (`on`): the screen kept on, landscape and the
/// system bars hidden. Closing it undoes that and gives the brightness back to the system.
pub fn watching(on: bool) {
    let result = context().and_then(|(_, activity)| {
        let args = [jvalue { l: activity }, jvalue { z: on }];
        call_video_surface(c"watching", c"(Landroid/app/Activity;Z)V", &args)
    });
    if let Err(err) = result {
        tracing::warn!("couldn't set up the window for the player: {err}");
    }
}

/// The screen's brightness from 0 to 1: the window's own, or the system's.
pub fn brightness() -> Option<f32> {
    let result = with_video_surface(|env, class, activity| unsafe {
        let f = &**env;
        let id = (f.v1_1.GetStaticMethodID)(
            env,
            class,
            c"brightness".as_ptr(),
            c"(Landroid/app/Activity;)F".as_ptr(),
        );
        let value = (f.v1_1.CallStaticFloatMethodA)(env, class, id, [jvalue { l: activity }].as_ptr());
        check(env, "VideoSurface.brightness")?;
        Ok(value)
    });
    result
        .inspect_err(|err| tracing::warn!("couldn't read the brightness: {err}"))
        .ok()
}

/// Sets the window's brightness, from 0 to 1, until the player closes ([`watching`]).
pub fn set_brightness(value: f32) {
    let result = context().and_then(|(_, activity)| {
        let args = [jvalue { l: activity }, jvalue { f: value.clamp(0., 1.) }];
        call_video_surface(c"setBrightness", c"(Landroid/app/Activity;F)V", &args)
    });
    if let Err(err) = result {
        tracing::warn!("couldn't set the brightness: {err}");
    }
}

/// A JNI global reference to an `android.view.Surface`, as an address: what mpv takes as `wid`.
pub(crate) type SurfaceRef = i64;

/// Told when the video surface comes (`Some`) and goes (`None`). It must stop drawing into a
/// surface before returning from `None`; the reference is deleted after.
type Sink = Box<dyn Fn(Option<SurfaceRef>) + Send + Sync>;

/// The player the surface goes to (one at a time), and the reference it was last given.
static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static CURRENT: Mutex<SurfaceRef> = Mutex::new(0);

/// Shows the video surface (at the last place it was put) and hands it to `sink` whenever it
/// exists. Replaces the previous player's sink.
pub(crate) fn show_surface(sink: impl Fn(Option<SurfaceRef>) + Send + Sync + 'static) -> Result<()> {
    *SINK.lock().unwrap() = Some(Box::new(sink));
    let (_, activity) = context()?;
    call_video_surface(c"show", c"(Landroid/app/Activity;)V", &[jvalue { l: activity }])
}

/// Takes the video surface away; the sink is told first.
pub(crate) fn hide_surface() {
    if let Err(err) = call_video_surface(c"hide", c"()V", &[]) {
        tracing::warn!("couldn't hide the video surface: {err}");
    }
}

/// Puts the video surface at this rectangle of the window, in pixels.
pub fn place_surface(left: i32, top: i32, width: i32, height: i32) {
    let args = [left, top, width, height].map(|i| jvalue { i });
    if let Err(err) = call_video_surface(c"place", c"(IIII)V", &args) {
        tracing::warn!("couldn't place the video surface: {err}");
    }
}

unsafe fn register_natives(env: *mut JNIEnv, class: jclass) -> Result<()> {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    if REGISTERED.get().is_some() {
        return Ok(());
    }
    let methods = [jni_sys::JNINativeMethod {
        name: c"nativeSurface".as_ptr() as *mut _,
        signature: c"(Landroid/view/Surface;)V".as_ptr() as *mut _,
        fnPtr: native_surface as *mut c_void,
    }];
    // SAFETY: the signature matches `native_surface`.
    unsafe {
        ((**env).v1_1.RegisterNatives)(env, class, methods.as_ptr(), methods.len() as jint);
        check(env, "RegisterNatives(VideoSurface)")?;
    }
    let _ = REGISTERED.set(());
    Ok(())
}

/// The surface was created (or is going away: `surface` null). On the UI thread.
extern "system" fn native_surface(env: *mut JNIEnv, _class: jclass, surface: jobject) {
    // SAFETY: `env` is this thread's; `surface` a valid local reference or null.
    let f = unsafe { &**env };
    let new = if surface.is_null() {
        0
    } else {
        unsafe { (f.v1_1.NewGlobalRef)(env, surface) as SurfaceRef }
    };
    if let Some(sink) = SINK.lock().unwrap().as_ref() {
        sink((new != 0).then_some(new));
    }
    let old = std::mem::replace(&mut *CURRENT.lock().unwrap(), new);
    if old != 0 {
        unsafe { (f.v1_1.DeleteGlobalRef)(env, old as jobject) };
    }
}
