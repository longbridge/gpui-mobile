//! Screen-reader support: hands GPUI's AccessKit tree to TalkBack.
//!
//! GPUI builds an AccessKit tree every frame once a screen reader is active and passes
//! it to `PlatformWindow::a11y_tree_update`. `accesskit_android`'s [`InjectingAdapter`]
//! exposes that tree as virtual `AccessibilityNodeInfo`s by installing an
//! `AccessibilityDelegate` on an existing `View`, so hosts need no Java changes.
//!
//! Node bounds are in surface pixels, so the `View` must line up with GPUI's surface:
//! the decor view on the `android-activity` path (the native surface fills the window)
//! and the content view on the host-driven path (it holds the `SurfaceView`).
//!
//! Only the active window's Activity is on screen, so one adapter serves the active
//! window, and a window that becomes active takes it over. A recreated Activity has
//! new views; [`super::jni::set_host_activity`] moves the adapter onto them.
//!
//! This adapter version has no "screen reader stopped" callback and raises events
//! without checking whether anyone listens, which Android answers by throwing on the
//! UI thread (`accesskit_android` 0.9 checks, but needs `accesskit` 0.25). So each
//! update first asks `AccessibilityManager.isEnabled()`; once it is off, GPUI is told
//! to stop building trees and a fresh adapter waits for the next screen reader.

use std::sync::{Arc, Mutex};

use accesskit_android::{
    jni::{
        objects::{GlobalRef, JObject},
        JNIEnv, JavaVM,
    },
    InjectingAdapter,
};
use gpui::{
    accesskit::{ActionHandler, ActionRequest, ActivationHandler, TreeUpdate},
    A11yCallbacks,
};

static BRIDGE: Mutex<Option<Bridge>> = Mutex::new(None);

/// `android.R.id.content`.
const CONTENT_VIEW_ID: i32 = 0x0102_0002;

struct Bridge {
    callbacks: Handlers,
    adapter: Option<Adapter>,
}

struct Adapter {
    inner: InjectingAdapter,
    vm: JavaVM,
    /// `android.view.accessibility.AccessibilityManager`.
    manager: GlobalRef,
}

/// One window's GPUI callbacks, shared by every adapter its Activity gets.
#[derive(Clone)]
pub(super) struct Handlers(Arc<Mutex<A11yCallbacks>>);

impl Handlers {
    pub(super) fn new(callbacks: A11yCallbacks) -> Self {
        Self(Arc::new(Mutex::new(callbacks)))
    }

    fn deactivate(&self) {
        (self.0.lock().expect("poisoned").deactivation)();
    }
}

impl ActivationHandler for Handlers {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        (self.0.lock().expect("poisoned").activation)()
    }
}

impl ActionHandler for Handlers {
    fn do_action(&mut self, request: ActionRequest) {
        (self.0.lock().expect("poisoned").action)(request)
    }
}

/// Point the screen reader at this window, on the GPUI thread. Called from
/// `PlatformWindow::a11y_init` and when the window becomes active again.
pub(super) fn init(callbacks: &Handlers) {
    let mut bridge = BRIDGE.lock().expect("poisoned");
    if let Some(previous) = bridge.as_ref() {
        if Arc::ptr_eq(&previous.callbacks.0, &callbacks.0) {
            return;
        }
        // The previous window is off screen; stop it building trees.
        previous.callbacks.deactivate();
    }
    bridge
        .insert(Bridge {
            callbacks: callbacks.clone(),
            adapter: None,
        })
        .attach();
}

/// `PlatformWindow::a11y_tree_update`, on the GPUI thread that called [`init`].
pub(super) fn update(callbacks: &Handlers, tree: TreeUpdate) {
    let mut bridge = BRIDGE.lock().expect("poisoned");
    let Some(bridge) = bridge
        .as_mut()
        .filter(|bridge| Arc::ptr_eq(&bridge.callbacks.0, &callbacks.0))
    else {
        return;
    };
    let Some(adapter) = bridge.adapter.as_mut() else {
        return;
    };
    if adapter.is_accessibility_enabled() {
        adapter.inner.update_if_active(|| tree);
        return;
    }
    // The screen reader has gone. Stop GPUI building trees, and swap in an adapter
    // that is not yet active, so the next screen reader activates GPUI again.
    bridge.callbacks.deactivate();
    bridge.attach();
}

/// A new Activity was registered; move the adapter onto its views.
pub(super) fn activity_changed() {
    if let Some(bridge) = BRIDGE.lock().expect("poisoned").as_mut() {
        bridge.attach();
    }
}

impl Bridge {
    fn attach(&mut self) {
        // Dropping the old adapter removes its delegate from the old view.
        self.adapter = None;
        let callbacks = self.callbacks.clone();
        match with_host_view(|env, view| {
            let manager = accessibility_manager(env, view)?;
            Ok(Adapter {
                inner: InjectingAdapter::new(env, view, callbacks.clone(), callbacks),
                vm: env.get_java_vm()?,
                manager,
            })
        }) {
            Ok(adapter) => self.adapter = Some(adapter),
            Err(err) => log::warn!("accessibility: cannot attach to the host view: {err}"),
        }
    }
}

impl Adapter {
    fn is_accessibility_enabled(&self) -> bool {
        let enabled = self
            .vm
            .get_env()
            .and_then(|mut env| env.call_method(&self.manager, "isEnabled", "()Z", &[])?.z());
        enabled.unwrap_or_else(|err| {
            log::warn!("accessibility: AccessibilityManager.isEnabled failed: {err}");
            false
        })
    }
}

fn accessibility_manager(
    env: &mut JNIEnv,
    view: &JObject,
) -> accesskit_android::jni::errors::Result<GlobalRef> {
    let context = env
        .call_method(view, "getContext", "()Landroid/content/Context;", &[])?
        .l()?;
    let name = env.new_string("accessibility")?;
    let manager = env
        .call_method(
            &context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[(&name).into()],
        )?
        .l()?;
    env.new_global_ref(manager)
}

fn with_host_view<T>(
    f: impl FnOnce(&mut JNIEnv, &JObject) -> accesskit_android::jni::errors::Result<T>,
) -> Result<T, String> {
    let vm = super::jni::java_vm();
    let activity = super::jni::activity_as_ptr();
    if vm.is_null() || activity.is_null() {
        return Err("JavaVM or Activity not available".into());
    }
    // SAFETY: both pointers come from the running JVM; the Activity is a global
    // reference that `super::jni` keeps alive.
    let vm = unsafe { JavaVM::from_raw(vm.cast()) }.map_err(|e| e.to_string())?;
    let activity = unsafe { JObject::from_raw(activity.cast()) };
    // `InjectingAdapter::update_if_active` expects the calling thread to stay attached.
    let mut env = vm
        .attach_current_thread_permanently()
        .map_err(|e| e.to_string())?;
    env.with_local_frame(8, |env| {
        let window = env
            .call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let mut view = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        if super::jni::android_app().is_none() {
            view = env
                .call_method(
                    &view,
                    "findViewById",
                    "(I)Landroid/view/View;",
                    &[CONTENT_VIEW_ID.into()],
                )?
                .l()?;
        }
        if view.is_null() {
            return Ok(Err("the Activity has no content view".into()));
        }
        Ok::<_, accesskit_android::jni::errors::Error>(Ok(f(env, &view)?))
    })
    .map_err(|e| e.to_string())?
}
