//! The WebView controls act on the `android.webkit.WebView` that
//! `GpuiPlatformView` created for the handle's platform view, found by its ID.

use super::{WebViewHandle, WebViewState};
use crate::android::jni::{self as jni_helpers, JniExt};
use futures::channel::oneshot;
use jni::objects::{JClass, JObjectArray, JString, JValue};
use jni::{EnvUnowned, NativeMethod};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const HELPER_CLASS: &str = "dev.gpui.mobile.GpuiPlatformView";

fn view_id(handle: &WebViewHandle) -> Result<i64, String> {
    handle
        .platform_handle
        .as_ref()
        .map(|view| view.id().0 as i64)
        .ok_or_else(|| "No active WebView".to_string())
}

pub fn load_url(handle: &WebViewHandle, url: &str) -> Result<(), String> {
    let id = view_id(handle)?;
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let jurl = env.new_string(url).e()?;
        env.call_static_method(
            &cls,
            jni::jni_str!("webViewLoadUrl"),
            jni::jni_sig!("(JLjava/lang/String;)V"),
            &[JValue::Long(id), JValue::Object(&jurl)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

pub fn load_html(handle: &WebViewHandle, html: &str) -> Result<(), String> {
    let id = view_id(handle)?;
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let jhtml = env.new_string(html).e()?;
        env.call_static_method(
            &cls,
            jni::jni_str!("webViewLoadHtml"),
            jni::jni_sig!("(JLjava/lang/String;)V"),
            &[JValue::Long(id), JValue::Object(&jhtml)],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

/// Calls a `static void name(long viewId)` on the helper.
macro_rules! by_id {
    ($fn_name:ident, $java:literal) => {
        pub fn $fn_name(handle: &WebViewHandle) -> Result<(), String> {
            let id = view_id(handle)?;
            jni_helpers::with_env(|env| {
                let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
                env.call_static_method(
                    &cls,
                    jni::jni_str!($java),
                    jni::jni_sig!("(J)V"),
                    &[JValue::Long(id)],
                )
                .map_err(|e| {
                    env.exception_clear();
                    e.to_string()
                })?;
                Ok(())
            })
        }
    };
}

by_id!(go_back, "webViewGoBack");
by_id!(go_forward, "webViewGoForward");
by_id!(reload, "webViewReload");
by_id!(stop_loading, "webViewStopLoading");

pub fn state(handle: &WebViewHandle) -> Result<WebViewState, String> {
    let id = view_id(handle)?;
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let result = env
            .call_static_method(
                &cls,
                jni::jni_str!("webViewState"),
                jni::jni_sig!("(J)[Ljava/lang/String;"),
                &[JValue::Long(id)],
            )
            .and_then(|v| v.l())
            .map_err(|e| {
                env.exception_clear();
                e.to_string()
            })?;
        if result.is_null() {
            return Err("The WebView is not created yet".into());
        }
        // SAFETY: `webViewState` returns a `String[]`.
        let array = unsafe { JObjectArray::<JString>::from_raw(env, result.into_raw()) };
        let mut fields = Vec::with_capacity(7);
        for i in 0..array.len(env).e()? {
            let item: JString = array.get_element(env, i).e()?;
            fields.push(jni_helpers::get_string(env, &item));
        }
        let field = |i: usize| fields.get(i).cloned().unwrap_or_default();
        Ok(WebViewState {
            url: field(0),
            title: field(1),
            loading: field(2) == "1",
            progress: field(3).parse().unwrap_or(0),
            can_go_back: field(4) == "1",
            can_go_forward: field(5) == "1",
            error: Some(field(6)).filter(|error| !error.is_empty()),
        })
    })
}

pub fn evaluate_javascript(handle: &WebViewHandle, script: &str) -> Result<(), String> {
    evaluate(handle, script, 0)
}

/// Evaluate `script` and resolve with its result, as JSON (`"\"text\""`, `"42"`,
/// `"null"` for `undefined`), which is what `WebView.evaluateJavascript` reports.
pub fn evaluate_javascript_for_result(
    handle: &WebViewHandle,
    script: &str,
) -> oneshot::Receiver<Result<String, String>> {
    let (sender, receiver) = oneshot::channel();
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    pending().lock().expect("poisoned").insert(request, sender);
    let result = jni_helpers::with_env(register_result_callback)
        .and_then(|()| evaluate(handle, script, request as i64));
    if let Err(err) = result {
        if let Some(sender) = pending().lock().expect("poisoned").remove(&request) {
            let _ = sender.send(Err(err));
        }
    }
    receiver
}

fn evaluate(handle: &WebViewHandle, script: &str, request: i64) -> Result<(), String> {
    let id = view_id(handle)?;
    jni_helpers::with_env(|env| {
        let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
        let jscript = env.new_string(script).e()?;
        env.call_static_method(
            &cls,
            jni::jni_str!("webViewEvaluateJavascript"),
            jni::jni_sig!("(JLjava/lang/String;J)V"),
            &[
                JValue::Long(id),
                JValue::Object(&jscript),
                JValue::Long(request),
            ],
        )
        .map_err(|e| {
            env.exception_clear();
            e.to_string()
        })?;
        Ok(())
    })
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<String, String>>>>;

/// Starts at 1: request 0 asks for no result.
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

fn pending() -> &'static Pending {
    static PENDING: OnceLock<Pending> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}

/// Register `GpuiPlatformView.nativeJavascriptResult`, once per loaded library: a
/// library reloaded by a dev tool registers its own.
fn register_result_callback(env: &mut jni::Env<'_>) -> Result<(), String> {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    if REGISTERED.get().is_some() {
        return Ok(());
    }
    let cls = jni_helpers::find_app_class(env, HELPER_CLASS)?;
    // SAFETY: `javascript_result` matches the Java declaration
    // `static native void nativeJavascriptResult(long, String, String)`.
    let methods = [unsafe {
        NativeMethod::from_raw_parts(
            jni::jni_str!("nativeJavascriptResult"),
            jni::jni_str!("(JLjava/lang/String;Ljava/lang/String;)V"),
            javascript_result as *mut std::ffi::c_void,
        )
    }];
    // SAFETY: as above.
    unsafe { env.register_native_methods(&cls, &methods) }.map_err(|e| {
        env.exception_clear();
        e.to_string()
    })?;
    let _ = REGISTERED.set(());
    Ok(())
}

/// Called by `GpuiPlatformView` on the UI thread with a script's result or error.
extern "system" fn javascript_result<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    request: i64,
    value: JString<'local>,
    error: JString<'local>,
) {
    env.with_env(|env| -> jni::errors::Result<()> {
        let result = if error.is_null() {
            Ok(if value.is_null() {
                "null".to_string()
            } else {
                jni_helpers::get_string(env, &value)
            })
        } else {
            Err(jni_helpers::get_string(env, &error))
        };
        if let Some(sender) = pending()
            .lock()
            .expect("poisoned")
            .remove(&(request as u64))
        {
            let _ = sender.send(result);
        }
        Ok(())
    })
    .resolve::<jni::errors::LogErrorAndDefault>();
}
