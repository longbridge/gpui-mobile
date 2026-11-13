//! Android platform view implementation.
//!
//! Embeds native Android `View` instances in the GPUI render tree using
//! hybrid composition. Native views are added on top of the Activity's
//! content and positioned to match GPUI element bounds. The Java side
//! (`GpuiPlatformView`) moves them to a recreated Activity: see
//! [`attach_to_activity`].

use crate::platform_view::{
    PlatformView, PlatformViewBounds, PlatformViewFactory, PlatformViewId, PlatformViewParams,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// Android implementation of a platform view.
///
/// In hybrid composition mode, this creates a native Android `View` and
/// adds it to the Activity's window. The view is positioned absolutely
/// to match the GPUI element's screen coordinates.
pub struct AndroidPlatformView {
    id: PlatformViewId,
    view_type: String,
    /// JNI global reference to the Android View object.
    /// 0 means no view (disposed or not yet created).
    #[allow(dead_code)]
    java_view_ref: std::sync::atomic::AtomicUsize,
    disposed: AtomicBool,
    bounds: std::sync::Mutex<PlatformViewBounds>,
    /// What was last sent to Java, so a GPUI frame that changes nothing makes no
    /// JNI call and posts nothing to the UI thread.
    sent: std::sync::Mutex<Sent>,
}

#[derive(Default)]
struct Sent {
    bounds: Option<PlatformViewBounds>,
    clip: Option<Option<PlatformViewBounds>>,
    visible: Option<bool>,
}

impl AndroidPlatformView {
    /// Create a new Android platform view.
    ///
    /// This allocates the Rust-side state. The actual Android `View` is
    /// created via JNI in a separate step.
    pub fn new(view_type: &str, params: &PlatformViewParams) -> Result<Self, String> {
        let id = PlatformViewId::next();

        let view = Self {
            id,
            view_type: view_type.to_string(),
            java_view_ref: std::sync::atomic::AtomicUsize::new(0),
            disposed: AtomicBool::new(false),
            bounds: std::sync::Mutex::new(params.bounds),
            sent: std::sync::Mutex::new(Sent {
                bounds: Some(params.bounds),
                clip: Some(None),
                visible: Some(true),
            }),
        };

        // Create the native Android view via JNI
        view.create_native_view(params)?;

        Ok(view)
    }

    /// Create the native Android View via JNI.
    fn create_native_view(&self, params: &PlatformViewParams) -> Result<(), String> {
        use super::jni::{activity, find_app_class, with_env, JniExt};
        use jni::objects::JValue;

        with_env(|env| {
            let helper_class = find_app_class(env, "dev.gpui.mobile.GpuiPlatformView")?;
            let view_type_jstr = env.new_string(&self.view_type).map_err(|e| e.to_string())?;
            let view_id = self.id.0 as i64;

            let bounds = self.bounds.lock().unwrap();
            let x = bounds.x;
            let y = bounds.y;
            let width = bounds.width;
            let height = bounds.height;
            drop(bounds);

            // "key1=value1|key2=value2", with `%` and `|` escaped so values (HTML,
            // URLs) may contain them.
            let creation_params_str: String = params
                .creation_params
                .iter()
                .map(|(k, v)| format!("{}={}", escape_param(k), escape_param(v)))
                .collect::<Vec<_>>()
                .join("|");
            let creation_params_jstr = env.new_string(&creation_params_str).e()?;

            let act = activity(env)?;

            let result = env
                .call_static_method(
                    &helper_class,
                    jni::jni_str!("createView"),
                    jni::jni_sig!(
                        "(Landroid/app/Activity;Ljava/lang/String;JFFFFLjava/lang/String;)Z"
                    ),
                    &[
                        JValue::Object(&act),
                        JValue::Object(&view_type_jstr),
                        JValue::Long(view_id),
                        JValue::Float(x),
                        JValue::Float(y),
                        JValue::Float(width),
                        JValue::Float(height),
                        JValue::Object(&creation_params_jstr),
                    ],
                )
                .map_err(|e| {
                    env.exception_clear();
                    format!("createView JNI call failed: {}", e)
                })?;

            let success = result.z().unwrap_or(false);
            if !success {
                return Err("GpuiPlatformView.createView returned false".to_string());
            }

            log::info!(
                "AndroidPlatformView: created native view id={} type='{}'",
                self.id,
                self.view_type
            );
            Ok(())
        })
    }

    /// Update the native view's bounds via JNI.
    fn update_native_bounds(&self, bounds: &PlatformViewBounds) {
        use super::jni::{find_app_class, with_env};
        use jni::objects::JValue;

        let view_id = self.id.0 as i64;
        let _ = with_env(|env| {
            let helper_class = find_app_class(env, "dev.gpui.mobile.GpuiPlatformView")?;
            let _ = env
                .call_static_method(
                    &helper_class,
                    jni::jni_str!("setBounds"),
                    jni::jni_sig!("(JFFFF)V"),
                    &[
                        JValue::Long(view_id),
                        JValue::Float(bounds.x),
                        JValue::Float(bounds.y),
                        JValue::Float(bounds.width),
                        JValue::Float(bounds.height),
                    ],
                )
                .map_err(|e| {
                    env.exception_clear();
                    format!("setBounds failed: {}", e)
                })?;
            Ok(())
        });
    }

    /// Update the native view's visibility via JNI.
    fn update_native_visibility(&self, visible: bool) {
        use super::jni::{find_app_class, with_env};
        use jni::objects::JValue;

        let view_id = self.id.0 as i64;
        let _ = with_env(|env| {
            let helper_class = find_app_class(env, "dev.gpui.mobile.GpuiPlatformView")?;
            let _ = env
                .call_static_method(
                    &helper_class,
                    jni::jni_str!("setVisible"),
                    jni::jni_sig!("(JZ)V"),
                    &[JValue::Long(view_id), JValue::Bool(visible)],
                )
                .map_err(|e| {
                    env.exception_clear();
                    format!("setVisible failed: {}", e)
                })?;
            Ok(())
        });
    }

    /// Remove and dispose the native view via JNI.
    fn dispose_native_view(&self) {
        use super::jni::{find_app_class, with_env};
        use jni::objects::JValue;

        let view_id = self.id.0 as i64;
        let _ = with_env(|env| {
            let helper_class = find_app_class(env, "dev.gpui.mobile.GpuiPlatformView")?;
            let _ = env
                .call_static_method(
                    &helper_class,
                    jni::jni_str!("disposeView"),
                    jni::jni_sig!("(J)V"),
                    &[JValue::Long(view_id)],
                )
                .map_err(|e| {
                    env.exception_clear();
                    format!("disposeView failed: {}", e)
                })?;
            Ok(())
        });
    }
}

impl PlatformView for AndroidPlatformView {
    fn id(&self) -> PlatformViewId {
        self.id
    }

    fn view_type(&self) -> &str {
        &self.view_type
    }

    fn set_bounds(&self, bounds: PlatformViewBounds) {
        if self.disposed.load(Ordering::Relaxed) {
            return;
        }
        *self.bounds.lock().unwrap() = bounds;
        if self.sent.lock().unwrap().bounds.replace(bounds) == Some(bounds) {
            return;
        }
        self.update_native_bounds(&bounds);
    }

    fn set_visible(&self, visible: bool) {
        if self.disposed.load(Ordering::Relaxed) {
            return;
        }
        if self.sent.lock().unwrap().visible.replace(visible) == Some(visible) {
            return;
        }
        self.update_native_visibility(visible);
    }

    fn set_clip(&self, clip: Option<PlatformViewBounds>) {
        if self.disposed.load(Ordering::Relaxed) {
            return;
        }
        if self.sent.lock().unwrap().clip.replace(clip) == Some(clip) {
            return;
        }
        use super::jni::{find_app_class, with_env};
        use jni::objects::JValue;

        let rect = clip.unwrap_or(PlatformViewBounds {
            x: 0.0,
            y: 0.0,
            width: -1.0,
            height: -1.0,
        });
        let view_id = self.id.0 as i64;
        if let Err(err) = with_env(|env| {
            let helper_class = find_app_class(env, "dev.gpui.mobile.GpuiPlatformView")?;
            env.call_static_method(
                &helper_class,
                jni::jni_str!("setClip"),
                jni::jni_sig!("(JFFFF)V"),
                &[
                    JValue::Long(view_id),
                    JValue::Float(rect.x),
                    JValue::Float(rect.y),
                    JValue::Float(rect.width),
                    JValue::Float(rect.height),
                ],
            )
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;
            Ok(())
        }) {
            log::warn!("AndroidPlatformView: setClip: {err}");
        }
    }

    fn set_z_index(&self, z_index: i32) {
        if self.disposed.load(Ordering::Relaxed) {
            return;
        }
        use super::jni::{find_app_class, with_env};
        use jni::objects::JValue;

        let view_id = self.id.0 as i64;
        let _ = with_env(|env| {
            let helper_class = find_app_class(env, "dev.gpui.mobile.GpuiPlatformView")?;
            let _ = env
                .call_static_method(
                    &helper_class,
                    jni::jni_str!("setZIndex"),
                    jni::jni_sig!("(JI)V"),
                    &[JValue::Long(view_id), JValue::Int(z_index)],
                )
                .map_err(|e| {
                    env.exception_clear();
                    format!("setZIndex failed: {}", e)
                })?;
            Ok(())
        });
    }

    fn dispose(&self) {
        if self.disposed.swap(true, Ordering::Relaxed) {
            return; // Already disposed
        }
        self.dispose_native_view();
        log::info!("AndroidPlatformView: disposed view {}", self.id);
    }

    fn is_disposed(&self) -> bool {
        self.disposed.load(Ordering::Relaxed)
    }
}

/// Escape a creation parameter for `GpuiPlatformView.parseCreationParams`.
fn escape_param(value: &str) -> String {
    value.replace('%', "%25").replace('|', "%7C")
}

/// Generic Android platform view factory.
///
/// Delegates view creation to the Java-side `GpuiPlatformView` helper,
/// which handles the actual `View` instantiation based on the type string.
pub struct AndroidPlatformViewFactory {
    view_type: String,
}

impl AndroidPlatformViewFactory {
    pub fn new(view_type: &str) -> Self {
        Self {
            view_type: view_type.to_string(),
        }
    }
}

impl PlatformViewFactory for AndroidPlatformViewFactory {
    fn create(&self, params: &PlatformViewParams) -> Result<Box<dyn PlatformView>, String> {
        let view = AndroidPlatformView::new(&self.view_type, params)?;
        Ok(Box::new(view))
    }

    fn view_type(&self) -> &str {
        &self.view_type
    }
}

/// Make `activity` the host of every platform view: `GpuiPlatformView` moves the
/// existing ones into it. [`crate::android::host::set_activity`] calls this, so a
/// recreated Activity keeps its maps, web views, camera previews and videos.
///
/// Apps that don't ship the `GpuiPlatformView` helper have no platform views;
/// for them this does nothing.
pub(crate) fn attach_to_activity(env: &mut jni::Env<'_>, activity: &jni::objects::JObject<'_>) {
    use super::jni::find_app_class;
    use jni::objects::JValue;

    let Ok(helper_class) = find_app_class(env, "dev.gpui.mobile.GpuiPlatformView") else {
        if env.exception_check() {
            env.exception_clear();
        }
        return;
    };
    if let Err(err) = env
        .call_static_method(
            &helper_class,
            jni::jni_str!("attachToActivity"),
            jni::jni_sig!("(Landroid/app/Activity;)V"),
            &[JValue::Object(activity)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })
    {
        // An app with an older copy of the helper.
        log::debug!("GpuiPlatformView.attachToActivity: {err}");
    }
}
