package dev.gpui.mobile;

import android.content.Context;
import android.content.pm.ApplicationInfo;
import android.util.Log;

import java.io.File;

/**
 * Loads the app's native library for a host-driven Activity
 * ({@code gpui_mobile::android::host}).
 *
 * <p>In a debuggable build, a library at {@code files/dev/lib<name>.so} in the
 * app's data directory is loaded instead of the packaged one, so a dev tool can
 * push a new build of the Rust code and restart the app without reinstalling
 * the APK. Release builds always load the packaged library. The dev tool should
 * make the file read-only, as Android 14 requires of dynamically loaded code.</p>
 *
 * <p>Not for {@code NativeActivity} hosts: {@code NativeActivity} always loads
 * the packaged library itself.</p>
 */
public final class GpuiLibraryLoader {
    private static boolean loaded;

    private GpuiLibraryLoader() {}

    /**
     * Load {@code lib<name>.so} once per process. Call it at the start of
     * {@code onCreate}, before any native method.
     *
     * @param context Any context of the app.
     * @param name    The library name, as for {@link System#loadLibrary}.
     */
    public static synchronized void load(Context context, String name) {
        if (loaded) return;
        File dev = new File(context.getFilesDir(), "dev/lib" + name + ".so");
        boolean debuggable =
                (context.getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0;
        if (debuggable && dev.isFile()) {
            Log.i("GpuiLibraryLoader", "loading dev library " + dev);
            System.load(dev.getAbsolutePath());
        } else {
            System.loadLibrary(name);
        }
        loaded = true;
    }
}
