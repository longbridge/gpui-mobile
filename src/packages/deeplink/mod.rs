//! Deep linking — handle incoming URLs and universal links.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
pub(crate) mod ios;

use std::sync::Mutex;

/// Callback type for deep link events.
type DeepLinkCallback = Box<dyn Fn(&str) + Send + 'static>;

static CALLBACK: Mutex<Option<DeepLinkCallback>> = Mutex::new(None);

/// Register a callback to be invoked when a deep link is received.
///
/// On Android it runs on the UI thread; forward the link to GPUI from there.
pub fn set_deep_link_handler(handler: impl Fn(&str) + Send + 'static) {
    *CALLBACK.lock().unwrap() = Some(Box::new(handler));
}

/// Deliver a link the running app was opened with.
///
/// On Android, call it from the host Activity's `onNewIntent` with the intent's data
/// (`GpuiActivity` in the example already does). It updates [`get_latest_link`] and
/// calls the handler.
#[cfg(target_os = "android")]
pub fn handle_link(url: &str) {
    android::set_latest_link(url);
    notify_deep_link(url);
}

/// Called internally when a deep link URL is received.
#[allow(dead_code)]
pub(crate) fn notify_deep_link(url: &str) {
    // Not under the lock, so the handler may replace itself.
    let Some(handler) = CALLBACK.lock().unwrap().take() else {
        return;
    };
    handler(url);
    CALLBACK.lock().unwrap().get_or_insert(handler);
}

/// Get the initial deep link URL that launched the app (if any).
pub fn get_initial_link() -> Result<Option<String>, String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_initial_link()
    }
    #[cfg(target_os = "android")]
    {
        android::get_initial_link()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Ok(None)
    }
}

/// Get the latest deep link URL received while the app was running.
pub fn get_latest_link() -> Option<String> {
    #[cfg(target_os = "ios")]
    {
        ios::get_latest_link()
    }
    #[cfg(target_os = "android")]
    {
        android::get_latest_link()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        None
    }
}
