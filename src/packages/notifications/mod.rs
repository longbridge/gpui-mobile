//! Local notifications for Android and iOS.
//!
//! Provides a cross-platform API for showing, scheduling, and canceling
//! local notifications.
//!
//! Feature-gated behind `notifications`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;

/// Importance/priority level for notifications.
#[derive(Debug, Clone, Copy, Default)]
pub enum Importance {
    Min,
    Low,
    #[default]
    Default,
    High,
    Max,
}

impl Importance {
    /// Convert to an integer for platform APIs.
    /// 0=min, 1=low, 2=default, 3=high, 4=max
    pub fn as_i32(self) -> i32 {
        match self {
            Importance::Min => 0,
            Importance::Low => 1,
            Importance::Default => 2,
            Importance::High => 3,
            Importance::Max => 4,
        }
    }
}

/// Android notification channel configuration.
#[derive(Debug, Clone)]
pub struct NotificationChannel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub importance: Importance,
}

impl Default for NotificationChannel {
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "Default".into(),
            description: "Default notification channel".into(),
            importance: Importance::Default,
        }
    }
}

/// Notification content to display.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: i32,
    pub title: String,
    pub body: String,
    pub channel: NotificationChannel,
    /// Data for the app when the user taps the notification; on Android, read it with
    /// [`take_launch_payload`].
    pub payload: Option<String>,
}

/// Initialize the notification system.
/// On Android, creates the default notification channel.
/// On iOS, requests notification authorization.
pub fn initialize() -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::initialize()
    }
    #[cfg(target_os = "android")]
    {
        android::initialize()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("notifications are only available on iOS and Android".into())
    }
}

/// Show an immediate notification.
///
/// On Android 13+ this fails unless `POST_NOTIFICATIONS` is granted (request it with
/// `permission_handler`'s `Permission::Notification`), and whenever the user turned
/// the app's notifications (or the channel) off, rather than the system dropping it
/// silently.
/// Tapping it opens the app.
pub fn show(notification: &Notification) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::show(notification)
    }
    #[cfg(target_os = "android")]
    {
        android::show(notification)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = notification;
        Err("notifications are only available on iOS and Android".into())
    }
}

/// The payload of the notification whose tap opened the app, once; `None` otherwise.
///
/// Reads the host Activity's intent. A tap on a running app reaches the launcher
/// Activity through `onNewIntent` only if it is `singleTask` or `singleTop`, and the
/// Activity must pass that intent to `setIntent` first.
#[cfg(target_os = "android")]
pub fn take_launch_payload() -> Result<Option<String>, String> {
    android::take_launch_payload()
}

/// Cancel a specific notification by ID.
pub fn cancel(id: i32) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::cancel(id)
    }
    #[cfg(target_os = "android")]
    {
        android::cancel(id)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let _ = id;
        Err("notifications are only available on iOS and Android".into())
    }
}

/// Cancel all notifications.
pub fn cancel_all() -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        ios::cancel_all()
    }
    #[cfg(target_os = "android")]
    {
        android::cancel_all()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        Err("notifications are only available on iOS and Android".into())
    }
}
