//! System media session integration for Android and iOS.
//!
//! Provides:
//! - Media notification with playback controls (Android notification shade, iOS lock screen)
//! - System panel playback info (title, artist, duration, progress)
//! - Media button handling (headphone buttons, car controls)
//!
//! Feature-gated behind `media_session`.

#[cfg(target_os = "android")]
mod android;

use std::sync::Mutex;

/// Callback for media actions from system controls.
pub type MediaActionCallback = Box<dyn Fn(MediaAction) + Send + 'static>;

/// Actions that can be triggered from system media controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
}

/// Callback for seek requests from system controls.
pub type MediaSeekCallback = Box<dyn Fn(u64) + Send + 'static>;

static ACTION_CALLBACK: Mutex<Option<MediaActionCallback>> = Mutex::new(None);
static SEEK_CALLBACK: Mutex<Option<MediaSeekCallback>> = Mutex::new(None);

/// Register a callback for media actions from system controls.
///
/// On Android it runs on the UI thread.
pub fn set_action_handler(handler: impl Fn(MediaAction) + Send + 'static) {
    *ACTION_CALLBACK.lock().unwrap() = Some(Box::new(handler));
}

/// Register a callback for seek requests from system controls.
///
/// On Android it runs on the UI thread.
pub fn set_seek_handler(handler: impl Fn(u64) + Send + 'static) {
    *SEEK_CALLBACK.lock().unwrap() = Some(Box::new(handler));
}

/// Called from platform code when a media action is received.
#[allow(dead_code)]
pub(crate) fn notify_action(action: MediaAction) {
    // Not under the lock, so the handler may replace itself.
    let Some(handler) = ACTION_CALLBACK.lock().unwrap().take() else {
        return;
    };
    handler(action);
    ACTION_CALLBACK.lock().unwrap().get_or_insert(handler);
}

/// Called from platform code when a seek request is received.
#[allow(dead_code)]
pub(crate) fn notify_seek(position_ms: u64) {
    let Some(handler) = SEEK_CALLBACK.lock().unwrap().take() else {
        return;
    };
    handler(position_ms);
    SEEK_CALLBACK.lock().unwrap().get_or_insert(handler);
}

/// Initialize the media session. Call once before using other functions.
pub fn init() -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        android::init()
    }
    #[cfg(not(target_os = "android"))]
    {
        Ok(())
    }
}

/// Update the media metadata shown in the system notification / lock screen.
pub fn set_metadata(title: &str, artist: &str, duration_ms: u64) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        android::set_metadata(title, artist, duration_ms)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (title, artist, duration_ms);
        Ok(())
    }
}

/// Update the playback state shown in system controls.
pub fn set_playback_state(is_playing: bool, position_ms: u64, speed: f32) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        android::set_playback_state(is_playing, position_ms, speed)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (is_playing, position_ms, speed);
        Ok(())
    }
}

/// Release the media session and dismiss the notification.
pub fn release() -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        android::release()
    }
    #[cfg(not(target_os = "android"))]
    {
        Ok(())
    }
}
