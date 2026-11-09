package dev.gpui.mobile;

import android.Manifest;
import android.app.Activity;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;

import androidx.core.app.NotificationCompat;

/**
 * Java helper for local notifications on Android.
 *
 * <p>All public methods are {@code static} and called from Rust via JNI.</p>
 */
public final class GpuiNotifications {

    private static final String DEFAULT_CHANNEL_ID = "default";
    private static final String DEFAULT_CHANNEL_NAME = "Default";
    private static final String DEFAULT_CHANNEL_DESC = "Default notification channel";

    /** Intent extra carrying a tapped notification's payload to the launched Activity. */
    public static final String EXTRA_PAYLOAD = "dev.gpui.mobile.notification_payload";

    /**
     * Initialize the notification system by creating the default notification channel.
     *
     * @param activity The current Activity.
     */
    public static void initialize(Activity activity) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationManager nm = (NotificationManager) activity.getSystemService(Context.NOTIFICATION_SERVICE);
            if (nm != null) {
                NotificationChannel channel = new NotificationChannel(
                        DEFAULT_CHANNEL_ID,
                        DEFAULT_CHANNEL_NAME,
                        NotificationManager.IMPORTANCE_DEFAULT
                );
                channel.setDescription(DEFAULT_CHANNEL_DESC);
                nm.createNotificationChannel(channel);
            }
        }
    }

    /**
     * Show a notification.
     *
     * @param activity    The current Activity.
     * @param id          Notification ID (for updating/canceling).
     * @param title       Notification title.
     * @param body        Notification body text.
     * @param channelId   Notification channel ID.
     * @param channelName Notification channel name.
     * @param channelDesc Notification channel description.
     * @param importance  Importance level (0=min, 1=low, 2=default, 3=high, 4=max).
     * @param payload     Optional payload string (may be empty), handed to the app
     *                    when the notification is tapped.
     * @throws SecurityException when the app may not post notifications, instead of
     *                    the system dropping it silently.
     */
    public static void show(Activity activity, int id, String title, String body,
                            String channelId, String channelName, String channelDesc,
                            int importance, String payload) {
        NotificationManager nm = (NotificationManager) activity.getSystemService(Context.NOTIFICATION_SERVICE);
        if (nm == null) {
            throw new IllegalStateException("No NotificationManager");
        }
        if (Build.VERSION.SDK_INT >= 33
                && activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            throw new SecurityException("POST_NOTIFICATIONS is not granted; request it first");
        }
        if (!nm.areNotificationsEnabled()) {
            throw new SecurityException("Notifications are turned off for this app");
        }

        // Create or update the notification channel (API 26+)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            int androidImportance = mapImportance(importance);
            NotificationChannel channel = new NotificationChannel(channelId, channelName, androidImportance);
            channel.setDescription(channelDesc);
            nm.createNotificationChannel(channel);
            // The user's importance wins over the one just set.
            NotificationChannel existing = nm.getNotificationChannel(channelId);
            if (existing != null && existing.getImportance() == NotificationManager.IMPORTANCE_NONE) {
                throw new SecurityException("Notification channel \"" + channelId + "\" is turned off");
            }
        }

        // Build the notification
        NotificationCompat.Builder builder = new NotificationCompat.Builder(activity, channelId)
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle(title)
                .setContentText(body)
                .setAutoCancel(true)
                .setPriority(mapPriority(importance));

        // Tapping opens the app, with the payload for takeLaunchPayload().
        Intent launch = activity.getPackageManager().getLaunchIntentForPackage(activity.getPackageName());
        if (launch != null) {
            if (payload != null && !payload.isEmpty()) launch.putExtra(EXTRA_PAYLOAD, payload);
            builder.setContentIntent(PendingIntent.getActivity(activity, id, launch,
                    PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
        }

        nm.notify(id, builder.build());
    }

    /**
     * The payload of the notification that opened {@code activity}, removed from its
     * intent so it is handled once; null if it was not opened from one.
     */
    public static String takeLaunchPayload(Activity activity) {
        Intent intent = activity.getIntent();
        if (intent == null) return null;
        String payload = intent.getStringExtra(EXTRA_PAYLOAD);
        intent.removeExtra(EXTRA_PAYLOAD);
        return payload;
    }

    /**
     * Cancel a specific notification by ID.
     *
     * @param activity The current Activity.
     * @param id       The notification ID to cancel.
     */
    public static void cancel(Activity activity, int id) {
        NotificationManager nm = (NotificationManager) activity.getSystemService(Context.NOTIFICATION_SERVICE);
        if (nm != null) {
            nm.cancel(id);
        }
    }

    /**
     * Cancel all notifications.
     *
     * @param activity The current Activity.
     */
    public static void cancelAll(Activity activity) {
        NotificationManager nm = (NotificationManager) activity.getSystemService(Context.NOTIFICATION_SERVICE);
        if (nm != null) {
            nm.cancelAll();
        }
    }

    /**
     * Map our importance level (0-4) to Android NotificationManager.IMPORTANCE_* constants.
     */
    private static int mapImportance(int importance) {
        switch (importance) {
            case 0: return NotificationManager.IMPORTANCE_MIN;      // 1
            case 1: return NotificationManager.IMPORTANCE_LOW;      // 2
            case 2: return NotificationManager.IMPORTANCE_DEFAULT;  // 3
            case 3: return NotificationManager.IMPORTANCE_HIGH;     // 4
            case 4: return NotificationManager.IMPORTANCE_MAX;      // 5
            default: return NotificationManager.IMPORTANCE_DEFAULT; // 3
        }
    }

    /**
     * Map our importance level (0-4) to NotificationCompat priority constants.
     * Used for pre-API-26 devices.
     */
    private static int mapPriority(int importance) {
        switch (importance) {
            case 0: return NotificationCompat.PRIORITY_MIN;
            case 1: return NotificationCompat.PRIORITY_LOW;
            case 2: return NotificationCompat.PRIORITY_DEFAULT;
            case 3: return NotificationCompat.PRIORITY_HIGH;
            case 4: return NotificationCompat.PRIORITY_MAX;
            default: return NotificationCompat.PRIORITY_DEFAULT;
        }
    }

    // Prevent instantiation.
    private GpuiNotifications() {}
}
