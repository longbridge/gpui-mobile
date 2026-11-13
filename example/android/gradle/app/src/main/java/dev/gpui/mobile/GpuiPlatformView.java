package dev.gpui.mobile;

import android.app.Activity;
import android.content.Context;
import android.content.MutableContextWrapper;
import android.graphics.Bitmap;
import android.graphics.Rect;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.View;
import android.view.ViewGroup;
import android.webkit.WebChromeClient;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;
import java.lang.ref.WeakReference;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Native Android views embedded in the GPUI render tree (hybrid composition).
 *
 * <p>Each platform view lives in its own FrameLayout container, positioned in
 * logical pixels over the GPUI surface inside one root FrameLayout added on top
 * of the Activity's content. Views draw above GPUI content; touches inside a
 * view go to it, and touches elsewhere fall through the root to the GPUI
 * surface underneath.</p>
 *
 * <p>Views outlive their Activity: GPUI's render thread and app state live for
 * the whole process, so when the Activity is recreated (a configuration change
 * the host doesn't handle, {@code Activity.recreate()}, "Don't keep
 * activities") {@link #attachToActivity} moves every container into the new
 * Activity. Views are built on a {@link MutableContextWrapper} whose base
 * context is switched to the new Activity, so they don't keep the old one.
 * gpui-mobile calls {@code attachToActivity} from {@code host::set_activity}.</p>
 *
 * <p>Every method may be called from any thread; work on views is posted to the
 * main thread in call order.</p>
 *
 * View types: "container", "video_player", "webview", "camera_preview", "map".
 */
public class GpuiPlatformView {
    private static final String TAG = "GpuiPlatformView";
    private static final Handler mainHandler = new Handler(Looper.getMainLooper());

    // Main thread only, except getView/getContainer/web state reads.
    private static final Map<Long, View> views = new ConcurrentHashMap<>();
    private static final Map<Long, FrameLayout> containers = new LinkedHashMap<>();
    private static final Map<Long, MutableContextWrapper> contexts = new HashMap<>();
    private static final Map<Long, Boolean> hidden = new HashMap<>();
    /** Clip rectangles in logical window coordinates {x, y, width, height}. */
    private static final Map<Long, float[]> clips = new HashMap<>();
    private static final Map<Long, WebState> webStates = new ConcurrentHashMap<>();

    /** The root FrameLayout that hosts all platform views, in {@link #currentActivity}. */
    private static FrameLayout rootContainer;
    private static WeakReference<Activity> currentActivity = new WeakReference<>(null);

    // ── Activity lifecycle ───────────────────────────────────────────────

    /**
     * Use {@code activity} as the host of all platform views from now on: move
     * existing ones into it. Call from every {@code onCreate}; a no-op for the
     * Activity already hosting them.
     */
    public static void attachToActivity(Activity activity) {
        mainHandler.post(() -> attachOnMain(activity));
    }

    private static void attachOnMain(Activity activity) {
        if (activity == null || activity.isFinishing() || activity.isDestroyed()) return;
        if (currentActivity.get() == activity) return;
        currentActivity = new WeakReference<>(activity);
        FrameLayout oldRoot = rootContainer;
        rootContainer = null;
        if (oldRoot != null) {
            detach(oldRoot);
        }
        if (containers.isEmpty()) {
            // Created on the first createView.
            return;
        }
        FrameLayout root = ensureRoot();
        for (Map.Entry<Long, FrameLayout> entry : containers.entrySet()) {
            FrameLayout container = entry.getValue();
            detach(container);
            MutableContextWrapper context = contexts.get(entry.getKey());
            if (context != null) context.setBaseContext(activity);
            root.addView(container);
        }
        Log.i(TAG, "Moved " + containers.size() + " platform view(s) to " + activity);
    }

    private static void detach(View view) {
        ViewGroup parent = (ViewGroup) view.getParent();
        if (parent != null) parent.removeView(view);
    }

    /** The root container in the current Activity, created on first use. Main thread. */
    private static FrameLayout ensureRoot() {
        if (rootContainer != null) return rootContainer;
        Activity activity = currentActivity.get();
        if (activity == null) return null;
        FrameLayout root = new FrameLayout(activity);
        // addContentView puts it after the host's own views (the GPUI SurfaceView), so on top.
        activity.addContentView(root, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        rootContainer = root;
        Log.i(TAG, "Root container added to " + activity);
        return root;
    }

    // ── Creation and layout ──────────────────────────────────────────────

    /**
     * Create a platform view and add it to the view hierarchy.
     *
     * @param creationParams Pipe-delimited key=value pairs (e.g. "player_id=1|url=https://...")
     * @return true: the view is created asynchronously on the main thread
     */
    public static boolean createView(
            Activity activity,
            String viewType,
            long viewId,
            float x, float y,
            float width, float height,
            String creationParams) {

        Log.i(TAG, "createView: type=" + viewType + " id=" + viewId
                + " bounds=(" + x + ", " + y + ", " + width + ", " + height + ")");

        Map<String, String> params = parseCreationParams(creationParams);

        mainHandler.post(() -> {
            try {
                attachOnMain(activity);
                Activity host = currentActivity.get();
                if (host == null) host = activity;
                MutableContextWrapper context = new MutableContextWrapper(host);
                float density = context.getResources().getDisplayMetrics().density;

                FrameLayout container = new FrameLayout(context);
                FrameLayout.LayoutParams layoutParams = new FrameLayout.LayoutParams(
                        Math.round(width * density), Math.round(height * density));
                layoutParams.leftMargin = Math.round(x * density);
                layoutParams.topMargin = Math.round(y * density);
                container.setLayoutParams(layoutParams);

                View view = createViewForType(context, viewType, viewId, params);
                if (view != null) {
                    container.addView(view, new FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            ViewGroup.LayoutParams.MATCH_PARENT));
                }

                views.put(viewId, view != null ? view : container);
                containers.put(viewId, container);
                contexts.put(viewId, context);

                FrameLayout root = ensureRoot();
                if (root != null) root.addView(container);
                Log.i(TAG, "View created: id=" + viewId + " type=" + viewType);
            } catch (Exception e) {
                Log.e(TAG, "Failed to create view: " + e.getMessage(), e);
            }
        });

        return true;
    }

    /**
     * Parse a pipe-delimited creation params string into a map.
     * Format: "key1=value1|key2=value2", with "%" and "|" in keys and values
     * escaped as "%25" and "%7C".
     */
    private static Map<String, String> parseCreationParams(String params) {
        Map<String, String> map = new HashMap<>();
        if (params == null || params.isEmpty()) return map;
        for (String pair : params.split("\\|")) {
            int eq = pair.indexOf('=');
            if (eq > 0) {
                map.put(unescape(pair.substring(0, eq)), unescape(pair.substring(eq + 1)));
            }
        }
        return map;
    }

    private static String unescape(String value) {
        return value.replace("%7C", "|").replace("%25", "%");
    }

    private static View createViewForType(
            Context context, String viewType, long viewId, Map<String, String> params) {
        switch (viewType) {
            case "container":
                FrameLayout frame = new FrameLayout(context);
                frame.setBackgroundColor(0x00000000);
                return frame;
            case "video_player":
                return GpuiVideoPlayer.createVideoSurface(context, parseInt(params.get("player_id")));
            case "webview":
                return createWebView(context, viewId, params);
            case "camera_preview":
                return GpuiCamera.createPreviewSurface(context, parseInt(params.get("session_id")));
            case "map":
                // Placeholder — MapView requires Google Play Services SDK
                FrameLayout mapContainer = new FrameLayout(context);
                mapContainer.setBackgroundColor(0xFFE0E0E0);
                return mapContainer;
            default:
                Log.w(TAG, "Unknown view type: " + viewType + ", creating empty container");
                return new FrameLayout(context);
        }
    }

    private static int parseInt(String value) {
        try {
            return value == null ? 0 : Integer.parseInt(value);
        } catch (NumberFormatException e) {
            Log.w(TAG, "Invalid integer creation param: " + value);
            return 0;
        }
    }

    /** Update a view's position and size, in logical pixels relative to the window. */
    public static void setBounds(long viewId, float x, float y, float width, float height) {
        mainHandler.post(() -> {
            FrameLayout container = containers.get(viewId);
            if (container == null) {
                Log.w(TAG, "setBounds: no container for id=" + viewId);
                return;
            }
            float density = container.getResources().getDisplayMetrics().density;
            FrameLayout.LayoutParams params = (FrameLayout.LayoutParams) container.getLayoutParams();
            params.leftMargin = Math.round(x * density);
            params.topMargin = Math.round(y * density);
            params.width = Math.round(width * density);
            params.height = Math.round(height * density);
            container.setLayoutParams(params);
            applyClip(container, clips.get(viewId));
        });
    }

    /**
     * Clip a view to a rectangle in logical pixels relative to the window (the GPUI
     * content mask, so a view inside a scroll container doesn't draw over what
     * surrounds it). A negative width removes the clip.
     */
    public static void setClip(long viewId, float x, float y, float width, float height) {
        mainHandler.post(() -> {
            FrameLayout container = containers.get(viewId);
            if (container == null) return;
            float[] clip = width < 0 ? null : new float[] {x, y, width, height};
            if (clip == null) clips.remove(viewId); else clips.put(viewId, clip);
            applyClip(container, clip);
        });
    }

    private static void applyClip(FrameLayout container, float[] clip) {
        if (clip == null) {
            container.setClipBounds(null);
            return;
        }
        float density = container.getResources().getDisplayMetrics().density;
        FrameLayout.LayoutParams params = (FrameLayout.LayoutParams) container.getLayoutParams();
        int left = Math.round(clip[0] * density) - params.leftMargin;
        int top = Math.round(clip[1] * density) - params.topMargin;
        container.setClipBounds(new Rect(
                left, top, left + Math.round(clip[2] * density), top + Math.round(clip[3] * density)));
    }

    /** Show or hide a platform view. */
    public static void setVisible(long viewId, boolean visible) {
        mainHandler.post(() -> {
            FrameLayout container = containers.get(viewId);
            if (container == null) return;
            hidden.put(viewId, !visible);
            container.setVisibility(visible ? View.VISIBLE : View.GONE);
        });
    }

    /** Set the z-order between platform views (View.setZ). */
    public static void setZIndex(long viewId, int zIndex) {
        mainHandler.post(() -> {
            FrameLayout container = containers.get(viewId);
            if (container == null) return;
            container.setZ(zIndex);
        });
    }

    /** Remove and dispose a platform view. */
    public static void disposeView(long viewId) {
        mainHandler.post(() -> {
            View view = views.remove(viewId);
            FrameLayout container = containers.remove(viewId);
            if (view instanceof WebView) {
                ((WebView) view).destroy();
            }
            if (container != null) {
                detach(container);
                container.removeAllViews();
            }
            contexts.remove(viewId);
            hidden.remove(viewId);
            clips.remove(viewId);
            webStates.remove(viewId);
            Log.i(TAG, "View disposed: id=" + viewId);
        });
    }

    /** The native View for a platform view ID, for packages that drive it directly. */
    public static View getView(long viewId) {
        return views.get(viewId);
    }

    /** The container FrameLayout for a platform view ID. Main thread. */
    public static FrameLayout getContainer(long viewId) {
        return containers.get(viewId);
    }

    /** Hide all platform views (app in background); views hidden by setVisible stay hidden. */
    public static void pauseAll() {
        mainHandler.post(() -> {
            for (FrameLayout container : containers.values()) {
                container.setVisibility(View.INVISIBLE);
            }
        });
    }

    /** Show the platform views hidden by {@link #pauseAll}. */
    public static void resumeAll() {
        mainHandler.post(() -> {
            for (Map.Entry<Long, FrameLayout> entry : containers.entrySet()) {
                boolean isHidden = Boolean.TRUE.equals(hidden.get(entry.getKey()));
                entry.getValue().setVisibility(isHidden ? View.GONE : View.VISIBLE);
            }
        });
    }

    /** Dispose all platform views. */
    public static void disposeAll() {
        mainHandler.post(() -> {
            for (Long id : containers.keySet().toArray(new Long[0])) {
                disposeView(id);
            }
        });
    }

    /**
     * Whether a point in physical pixels relative to the window falls inside a
     * visible platform view. Main thread.
     */
    public static boolean hitTest(float x, float y) {
        for (FrameLayout container : containers.values()) {
            if (container.getVisibility() != View.VISIBLE) continue;
            int[] location = new int[2];
            container.getLocationInWindow(location);
            if (x >= location[0] && x <= location[0] + container.getWidth()
                    && y >= location[1] && y <= location[1] + container.getHeight()) {
                return true;
            }
        }
        return false;
    }

    /**
     * Dispatch a touch to the platform views, for hosts whose input doesn't go
     * through the view hierarchy (NativeActivity).
     *
     * @param action MotionEvent action constant (0=DOWN, 1=UP, 2=MOVE)
     */
    public static void dispatchTouch(float x, float y, int action) {
        mainHandler.post(() -> {
            if (rootContainer == null) return;
            long now = android.os.SystemClock.uptimeMillis();
            android.view.MotionEvent event = android.view.MotionEvent.obtain(now, now, action, x, y, 0);
            rootContainer.dispatchTouchEvent(event);
            event.recycle();
        });
    }

    // ── WebView ──────────────────────────────────────────────────────────

    /** What a WebView reports, written on the main thread and read from any. */
    static final class WebState {
        volatile String url = "";
        volatile String title = "";
        volatile boolean loading;
        volatile int progress;
        volatile boolean canGoBack;
        volatile boolean canGoForward;
        volatile String error = "";
    }

    private static View createWebView(Context context, long viewId, Map<String, String> params) {
        boolean jsEnabled = Boolean.parseBoolean(params.getOrDefault("javascript_enabled", "true"));
        boolean domStorage = Boolean.parseBoolean(params.getOrDefault("dom_storage_enabled", "true"));
        boolean zoom = Boolean.parseBoolean(params.getOrDefault("zoom_enabled", "true"));
        String userAgent = params.getOrDefault("user_agent", "");
        String url = params.getOrDefault("url", "");
        String html = params.getOrDefault("html", "");

        WebView wv = new WebView(context);
        WebSettings settings = wv.getSettings();
        settings.setJavaScriptEnabled(jsEnabled);
        settings.setDomStorageEnabled(domStorage);
        settings.setBuiltInZoomControls(zoom);
        settings.setSupportZoom(zoom);
        if (zoom) settings.setDisplayZoomControls(false);
        settings.setLoadWithOverviewMode(true);
        settings.setUseWideViewPort(true);
        if (!userAgent.isEmpty()) settings.setUserAgentString(userAgent);

        WebState state = new WebState();
        webStates.put(viewId, state);
        wv.setWebViewClient(new WebViewClient() {
            @Override
            public void onPageStarted(WebView view, String pageUrl, Bitmap favicon) {
                state.loading = true;
                state.error = "";
                update(view, state);
            }

            @Override
            public void onPageFinished(WebView view, String pageUrl) {
                state.loading = false;
                update(view, state);
            }

            @Override
            public void doUpdateVisitedHistory(WebView view, String pageUrl, boolean isReload) {
                update(view, state);
            }

            @Override
            public void onReceivedError(WebView view, android.webkit.WebResourceRequest request,
                    android.webkit.WebResourceError error) {
                if (request.isForMainFrame()) {
                    state.error = error.getErrorCode() + " " + error.getDescription();
                }
            }
        });
        wv.setWebChromeClient(new WebChromeClient() {
            @Override
            public void onProgressChanged(WebView view, int progress) {
                state.progress = progress;
            }

            @Override
            public void onReceivedTitle(WebView view, String title) {
                state.title = title == null ? "" : title;
            }
        });

        if (!html.isEmpty()) {
            wv.loadDataWithBaseURL(null, html, "text/html", "UTF-8", null);
        } else if (!url.isEmpty()) {
            wv.loadUrl(url);
        }
        return wv;
    }

    private static void update(WebView view, WebState state) {
        String url = view.getUrl();
        state.url = url == null ? "" : url;
        String title = view.getTitle();
        if (title != null) state.title = title;
        state.canGoBack = view.canGoBack();
        state.canGoForward = view.canGoForward();
    }

    /** Run {@code action} on the main thread with the WebView of {@code viewId}, if there is one. */
    private static void withWebView(long viewId, java.util.function.Consumer<WebView> action) {
        mainHandler.post(() -> {
            View view = views.get(viewId);
            if (view instanceof WebView) {
                action.accept((WebView) view);
            } else {
                Log.w(TAG, "No WebView with id=" + viewId);
            }
        });
    }

    public static void webViewLoadUrl(long viewId, String url) {
        withWebView(viewId, wv -> wv.loadUrl(url));
    }

    public static void webViewLoadHtml(long viewId, String html) {
        withWebView(viewId, wv -> wv.loadDataWithBaseURL(null, html, "text/html", "UTF-8", null));
    }

    public static void webViewGoBack(long viewId) {
        withWebView(viewId, wv -> { if (wv.canGoBack()) wv.goBack(); });
    }

    public static void webViewGoForward(long viewId) {
        withWebView(viewId, wv -> { if (wv.canGoForward()) wv.goForward(); });
    }

    public static void webViewReload(long viewId) {
        withWebView(viewId, WebView::reload);
    }

    public static void webViewStopLoading(long viewId) {
        withWebView(viewId, WebView::stopLoading);
    }

    /**
     * Evaluate {@code script} in the page. With a nonzero {@code requestId}, the result
     * (JSON, as WebView reports it) or an error comes back through
     * {@link #nativeJavascriptResult}, which gpui-mobile registers.
     */
    public static void webViewEvaluateJavascript(long viewId, String script, long requestId) {
        mainHandler.post(() -> {
            View view = views.get(viewId);
            if (!(view instanceof WebView)) {
                deliverJavascriptResult(requestId, null, "No WebView with id=" + viewId);
                return;
            }
            ((WebView) view).evaluateJavascript(script, value -> deliverJavascriptResult(requestId, value, null));
        });
    }

    private static void deliverJavascriptResult(long requestId, String value, String error) {
        if (requestId == 0) return;
        try {
            nativeJavascriptResult(requestId, value, error);
        } catch (UnsatisfiedLinkError e) {
            Log.w(TAG, "JavaScript result dropped: native callback not registered");
        }
    }

    /**
     * The WebView's state: {url, title, loading, progress, canGoBack, canGoForward, error},
     * or null if there is no WebView with that ID. Any thread.
     */
    public static String[] webViewState(long viewId) {
        WebState state = webStates.get(viewId);
        if (state == null) return null;
        return new String[] {
            state.url,
            state.title,
            state.loading ? "1" : "0",
            Integer.toString(state.progress),
            state.canGoBack ? "1" : "0",
            state.canGoForward ? "1" : "0",
            state.error,
        };
    }

    private static native void nativeJavascriptResult(long requestId, String value, String error);
}
