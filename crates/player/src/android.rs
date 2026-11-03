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

/// Calls a static `void` method of `VideoSurface` with int arguments.
fn call_video_surface(method: &CStr, signature: &CStr, args: &[jvalue]) -> Result<()> {
    with_env(|env, activity| unsafe {
        let f = &**env;
        let class = load_class(env, activity, c"io.github.lgpy.tabbyfin.VideoSurface")?;
        register_natives(env, class)?;
        let id = (f.v1_1.GetStaticMethodID)(env, class, method.as_ptr(), signature.as_ptr());
        (f.v1_1.CallStaticVoidMethodA)(env, class, id, args.as_ptr());
        check(env, "VideoSurface")?;
        (f.v1_1.DeleteLocalRef)(env, class);
        Ok(())
    })
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
