//! Host-driven Android entry point: a plain Java `Activity` owns a `SurfaceView`,
//! and GPUI runs on a **process-lived** render thread of our own.
//!
//! # Why this exists next to [`super::jni`]
//!
//! The `android-activity` path in `jni.rs` spawns a fresh native thread per Activity
//! and blocks the Java `onDestroy` until that thread stops ([`notify_destroyed`]).
//! A recreated Activity therefore always runs on a *new* thread, while
//! `AndroidDispatcher::new()` captured `ALooper_forThread()` of the *old* one — so
//! GPUI's `App::new_app` trips `assert!(is_main_thread(), "must construct App on main
//! thread")` and the screen goes black with nothing but a log line.
//!
//! Here the render thread is created once and outlives every Activity. Surfaces are
//! handed in and taken back through [`surface_created`] / [`surface_destroyed`], which
//! is the same mechanism the background/foreground cycle already uses
//! (`AndroidWindow::term_window` keeps the renderer and its atlas alive, `init_window`
//! re-attaches a new surface to it).
//!
//! This module is **additive**: the `android-activity` path is untouched, so both
//! entry points can coexist while hosts migrate.
//!
//! # Threading contract
//!
//! - `render thread` — owns the `ALooper`, the `AndroidPlatform`, GPUI's `App` and the
//!   `ApplicationHandle`. Everything GPUI touches stays here.
//! - `Java UI thread` — calls the `surface_*` / `lifecycle_*` / `dispatch_*` functions
//!   below. They only touch the command queue and the window slot, never GPUI state.
//! - [`surface_destroyed`] **blocks** until the render thread has stopped using the
//!   surface. That wait is mandatory: destroying a `Surface` while a thread sits inside
//!   `ANativeWindow_lock` is a use-after-free.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicPtr, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

use ndk::native_window::NativeWindow;

use super::platform::AndroidPlatform;
use crate::android::{AndroidKeyEvent, TouchPoint};

/// The render thread's `ALooper`, so other threads can wake it.
static LOOPER: Mutex<Option<LooperPtr>> = Mutex::new(None);

/// Commands posted from the Java UI thread, drained on the render thread.
static COMMANDS: Mutex<Vec<Command>> = Mutex::new(Vec::new());

/// Set once the render thread is running; guards against a second spawn.
static STARTED: OnceLock<()> = OnceLock::new();

/// Cleared by the render thread once it has released the outgoing surface.
static SURFACE_RELEASED: AtomicBool = AtomicBool::new(true);

/// The `ANativeWindow` the renderer is currently attached to.
///
/// `SurfaceHolder.Callback` fires `surfaceCreated` **and** `surfaceChanged` for the
/// same `Surface`, and Vulkan allows only one surface per `ANativeWindow`: building a
/// second one fails with `ERROR_NATIVE_WINDOW_IN_USE_KHR`. So a repeat of the window
/// we already hold is treated as a resize, not as a re-attach.
static CURRENT_SURFACE: AtomicPtr<ndk_sys::ANativeWindow> = AtomicPtr::new(std::ptr::null_mut());

struct LooperPtr(*mut ndk_sys::ALooper);
// SAFETY: `ALooper_wake` is documented as safe to call from any thread.
unsafe impl Send for LooperPtr {}

enum Command {
    SurfaceCreated {
        window: NativeWindow,
        scale: f32,
    },
    SurfaceDestroyed,
    Resumed,
    Paused,
    /// Input arrives on the Java UI thread but GPUI may only be touched from the
    /// render thread, so both go through the queue like everything else.
    Touch(TouchPoint),
    Key(AndroidKeyEvent),
}

fn post(command: Command) {
    COMMANDS.lock().expect("poisoned").push(command);
    wake_render_thread();
}

fn wake_render_thread() {
    if let Some(looper) = LOOPER.lock().expect("poisoned").as_ref() {
        // SAFETY: the pointer stays valid for the lifetime of the render thread,
        // which is the lifetime of the process.
        unsafe { ndk_sys::ALooper_wake(looper.0) };
    }
}

/// Start the render thread. Idempotent — later calls are no-ops, which is what makes
/// an Activity recreation cheap: the thread, the platform and the GPUI `App` all survive.
///
/// `launch` runs **on the render thread** once the first surface is available. Use it to
/// build the UI exactly as you would inside `Application::run`'s closure.
pub fn start<F>(launch: F)
where
    F: FnOnce() + Send + 'static,
{
    let mut launch = Some(launch);
    STARTED.get_or_init(move || {
        std::thread::Builder::new()
            .name("gpui-main".into())
            .spawn(move || render_thread(launch.take().expect("launch closure")))
            .expect("spawn gpui-main");
    });
}

fn render_thread<F: FnOnce()>(launch: F) {
    // SAFETY: called once, on this thread, before anything registers with the looper.
    let looper = unsafe { ndk_sys::ALooper_prepare(0) };
    assert!(
        !looper.is_null(),
        "ALooper_prepare failed; AndroidDispatcher requires a looper on this thread"
    );
    *LOOPER.lock().expect("poisoned") = Some(LooperPtr(looper));
    log::info!("gpui-main: render thread started, looper={looper:p}");

    // The platform must be built **here**: `AndroidDispatcher::new()` captures
    // `ALooper_forThread()`, and GPUI compares it against the calling thread forever after.
    let platform = Arc::new(AndroidPlatform::new(false));
    super::jni::set_host_platform(Arc::clone(&platform));

    let mut launched = Some(launch);

    loop {
        for command in COMMANDS
            .lock()
            .expect("poisoned")
            .drain(..)
            .collect::<Vec<_>>()
        {
            match command {
                Command::SurfaceCreated { window, scale } => {
                    on_surface_created(&platform, window, scale, &mut launched);
                }
                Command::SurfaceDestroyed => {
                    if let Some(win) = platform.primary_window() {
                        // Keeps the renderer (and its atlas) alive; only the surface goes.
                        win.term_window();
                    }
                    CURRENT_SURFACE.store(std::ptr::null_mut(), Ordering::SeqCst);
                    SURFACE_RELEASED.store(true, Ordering::SeqCst);
                }
                Command::Resumed => {
                    platform.did_become_active();
                    if let Some(win) = platform.primary_window() {
                        win.set_active(true);
                    }
                }
                Command::Paused => {
                    platform.did_enter_background();
                    if let Some(win) = platform.primary_window() {
                        win.set_active(false);
                    }
                }
                Command::Touch(point) => match platform.primary_window() {
                    Some(win) => win.handle_touch(point),
                    None => log::warn!("gpui-main: touch dropped — no window"),
                },
                Command::Key(event) => {
                    if let Some(win) = platform.primary_window() {
                        win.handle_key_event(event);
                    }
                }
            }
        }

        platform.tick();
        platform.flush_main_thread_tasks();
        if let Some(win) = platform.primary_window() {
            if win.is_active() {
                win.request_frame();
            }
        }

        // Sleep on the looper until woken by a command or a dispatcher task.
        // The short timeout keeps animations ticking; a demand-driven version can
        // drop it once frame scheduling moves to `AChoreographer`.
        // SAFETY: called only from the thread that owns this looper.
        unsafe {
            ndk_sys::ALooper_pollOnce(
                8,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
    }
}

fn on_surface_created<F: FnOnce()>(
    platform: &Arc<AndroidPlatform>,
    window: NativeWindow,
    scale: f32,
    launched: &mut Option<F>,
) {
    SURFACE_RELEASED.store(false, Ordering::SeqCst);

    let incoming = window.ptr().as_ptr();
    if CURRENT_SURFACE.load(Ordering::SeqCst) == incoming {
        // `surfaceChanged` for a surface we are already rendering into: the size may
        // have changed, but the Vulkan surface must not be rebuilt.
        if let Some(existing) = platform.primary_window() {
            existing.handle_resize();
        }
        return;
    }

    if let Some(existing) = platform.primary_window() {
        // Recreated Activity (or a resumed one): re-attach the new surface to the
        // renderer we already have. GPUI's scene cache and the texture atlas survive.
        let gpu = platform.gpu_context();
        match existing.init_window(window, gpu) {
            Ok(()) => {
                CURRENT_SURFACE.store(incoming, Ordering::SeqCst);
                log::info!("gpui-main: surface re-attached to existing window");
            }
            Err(err) => log::error!("gpui-main: init_window failed: {err:#}"),
        }
        existing.set_active(true);
        return;
    }

    // First surface: create the window, then let the host build its UI.
    match platform.open_window(window, scale, false) {
        Ok(win) => {
            CURRENT_SURFACE.store(incoming, Ordering::SeqCst);
            win.set_active(true);
            log::info!("gpui-main: first window opened");
            if let Some(launch) = launched.take() {
                launch();
            }
        }
        Err(err) => log::error!("gpui-main: open_window failed: {err:#}"),
    }
}

/// Hand a new `Surface` to the render thread. Safe to call repeatedly.
///
/// # Safety
/// `window` must be a live `ANativeWindow` obtained from `ANativeWindow_fromSurface`.
pub fn surface_created(window: NativeWindow, scale: f32) {
    post(Command::SurfaceCreated { window, scale });
}

/// Take the surface back and **block until the render thread has let go of it**.
///
/// Call this from `SurfaceHolder.Callback.surfaceDestroyed` before returning to the
/// framework, otherwise the `Surface` is torn down underneath a thread that may be
/// inside `ANativeWindow_lock`.
pub fn surface_destroyed() {
    post(Command::SurfaceDestroyed);

    // Bounded wait: a stuck render thread must not turn into an ANR. 2 s is far below
    // the 5 s input-dispatch timeout while being far above a worst-case frame.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !SURFACE_RELEASED.load(Ordering::SeqCst) {
        if std::time::Instant::now() >= deadline {
            log::error!(
                "gpui-main: timed out waiting for the render thread to release the surface"
            );
            return;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
}

/// Deliver one pointer of a `MotionEvent`.
///
/// `action` is an `AMOTION_EVENT_ACTION_*` value already reduced to a single pointer
/// (`POINTER_DOWN`/`POINTER_UP` collapsed to `DOWN`/`UP` by the caller), matching what
/// [`super::jni::process_input_events`] feeds the window on the `android-activity` path.
/// Coordinates are in physical pixels, relative to the surface.
pub fn touch(id: i32, action: u32, x: f32, y: f32) {
    post(Command::Touch(TouchPoint { id, x, y, action }));
}

/// Deliver a key event. `action` is `0` for down and `1` for up.
///
/// The unicode character is resolved here rather than in Java so that the mapping stays
/// identical to the `android-activity` path.
pub fn key(key_code: i32, action: i32, meta_state: i32) {
    let unicode_char = super::jni::unicode_char_for_key_event(key_code, action, meta_state);
    post(Command::Key(AndroidKeyEvent {
        key_code,
        action,
        meta_state,
        unicode_char,
    }));
}

/// Forward an IME update from a host-owned `InputConnection`.
///
/// Mirrors what [`super::jni`]'s `nativeIme` does for `GpuiInputActivity`, exposed
/// publicly so a host with its own Activity class can route its own JNI entry point
/// here. Safe to call from the Java UI thread — the event is queued, not applied.
///
/// `kind` matches the Java side: `0` composing, `1` commit, `2` delete-surrounding,
/// `3` delete-in-code-points, `4` done/dismiss.
pub fn ime_event(session: u64, kind: i32, text: String, start: usize, end: usize) {
    super::text_input::enqueue(super::text_input::ImeEvent {
        session,
        kind,
        text,
        start,
        end,
    });
}

pub fn resumed() {
    post(Command::Resumed);
}

pub fn paused() {
    post(Command::Paused);
}
