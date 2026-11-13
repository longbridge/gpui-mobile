package dev.gpui.mobile;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Rect;
import android.graphics.RectF;
import android.net.http.HttpResponseCache;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;
import android.util.LruCache;
import android.view.GestureDetector;
import android.view.MotionEvent;
import android.view.ScaleGestureDetector;
import android.view.View;
import java.io.File;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * A slippy map drawn from raster tiles (OpenStreetMap by default), the Android
 * view behind gpui-mobile's {@code maps} package.
 *
 * <p>No SDK and no API key: tiles come over HTTPS from a URL template
 * ({@code {z}/{x}/{y}}), cached in memory and in the HTTP response cache. Pan,
 * pinch and double-tap zoom, markers with a title bubble on tap, and the
 * tile source's attribution in the corner, which OpenStreetMap's tile usage
 * policy requires (https://operations.osmfoundation.org/policies/tiles/). Apps
 * with real traffic should point {@code tile_url} at their own or a commercial
 * tile server.</p>
 *
 * <p>Controlled from Rust through the static methods at the bottom, by platform
 * view ID; they may be called from any thread.</p>
 */
public class GpuiMapView extends View {
    private static final String TAG = "GpuiMapView";
    static final String DEFAULT_TILE_URL = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
    static final String DEFAULT_ATTRIBUTION = "© OpenStreetMap contributors";
    private static final int MAX_ZOOM = 19;
    private static final int TILE_PIXELS = 256;

    private static final Handler mainHandler = new Handler(Looper.getMainLooper());
    private static final ExecutorService loader = Executors.newFixedThreadPool(4);
    private static final LruCache<String, Bitmap> tiles = new LruCache<String, Bitmap>(
            (int) Math.min(Runtime.getRuntime().maxMemory() / 16, 48L * 1024 * 1024)) {
        @Override
        protected int sizeOf(String key, Bitmap bitmap) {
            return bitmap.getByteCount();
        }
    };
    private static final Set<String> loading = new HashSet<>();
    private static final Map<String, Long> failedAt = new HashMap<>();

    static final class Marker {
        final String id;
        final double latitude;
        final double longitude;
        final String title;
        final String snippet;

        Marker(String id, double latitude, double longitude, String title, String snippet) {
            this.id = id;
            this.latitude = latitude;
            this.longitude = longitude;
            this.title = title;
            this.snippet = snippet;
        }
    }

    private final String tileUrl;
    private final String attribution;
    private final String userAgent;
    private final boolean zoomGestures;
    private final boolean scrollGestures;
    private final float tileSize;
    private final float density;

    // Written on the main thread, read from any (getCamera).
    private volatile double latitude;
    private volatile double longitude;
    private volatile double zoom;
    private volatile String selectedMarker;

    private final Map<String, Marker> markers = new LinkedHashMap<>();
    /** Tiles the last frame wanted; a queued load for anything else is skipped. */
    private volatile Set<String> wanted = new HashSet<>();

    private final Paint tilePaint = new Paint(Paint.FILTER_BITMAP_FLAG);
    private final Paint pinPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint pinStroke = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint textPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint labelPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint bubblePaint = new Paint(Paint.ANTI_ALIAS_FLAG);

    private final GestureDetector gestures;
    private final ScaleGestureDetector scaleGestures;

    GpuiMapView(Context context, Map<String, String> params) {
        super(context);
        density = context.getResources().getDisplayMetrics().density;
        tileSize = TILE_PIXELS * density;
        latitude = parseDouble(params.get("latitude"), 0);
        longitude = parseDouble(params.get("longitude"), 0);
        zoom = clampZoom(parseDouble(params.get("zoom"), 2));
        zoomGestures = !"false".equals(params.get("zoom_gestures"));
        scrollGestures = !"false".equals(params.get("scroll_gestures"));
        String url = params.get("tile_url");
        tileUrl = url == null || url.isEmpty() ? DEFAULT_TILE_URL : url;
        String credit = params.get("attribution");
        attribution = credit == null ? DEFAULT_ATTRIBUTION : credit;
        // The tile usage policy asks for a User-Agent naming the app.
        userAgent = context.getPackageName() + " (gpui-mobile GpuiMapView)";

        setBackgroundColor(0xFFE8E4DC);
        setContentDescription("Map");
        pinPaint.setColor(0xFFD93025);
        pinStroke.setColor(Color.WHITE);
        pinStroke.setStyle(Paint.Style.STROKE);
        pinStroke.setStrokeWidth(2 * density);
        textPaint.setColor(0xFF202124);
        textPaint.setTextSize(11 * context.getResources().getDisplayMetrics().scaledDensity);
        labelPaint.setColor(0xFF202124);
        labelPaint.setTextSize(13 * context.getResources().getDisplayMetrics().scaledDensity);
        bubblePaint.setColor(0xF2FFFFFF);

        installHttpCache(context);

        gestures = new GestureDetector(context, new GestureDetector.SimpleOnGestureListener() {
            @Override
            public boolean onDown(MotionEvent e) {
                return true;
            }

            @Override
            public boolean onScroll(MotionEvent e1, MotionEvent e2, float dx, float dy) {
                if (!scrollGestures || scaleGestures.isInProgress()) return false;
                panBy(dx, dy);
                return true;
            }

            @Override
            public boolean onDoubleTap(MotionEvent e) {
                if (!zoomGestures) return false;
                zoomAround(zoom + 1, e.getX(), e.getY());
                return true;
            }

            @Override
            public boolean onSingleTapConfirmed(MotionEvent e) {
                String hit = markerAt(e.getX(), e.getY());
                selectedMarker = hit;
                invalidate();
                return true;
            }
        });
        scaleGestures = new ScaleGestureDetector(context, new ScaleGestureDetector.SimpleOnScaleGestureListener() {
            @Override
            public boolean onScale(ScaleGestureDetector detector) {
                if (!zoomGestures) return false;
                double delta = Math.log(detector.getScaleFactor()) / Math.log(2);
                zoomAround(zoom + delta, detector.getFocusX(), detector.getFocusY());
                return true;
            }
        });
    }

    private static double parseDouble(String value, double fallback) {
        try {
            return value == null ? fallback : Double.parseDouble(value);
        } catch (NumberFormatException e) {
            return fallback;
        }
    }

    private static double clampZoom(double z) {
        return Math.max(1, Math.min(MAX_ZOOM, z));
    }

    private static void installHttpCache(Context context) {
        if (HttpResponseCache.getInstalled() != null) return;
        try {
            HttpResponseCache.install(new File(context.getCacheDir(), "gpui-map-tiles"), 32L * 1024 * 1024);
        } catch (Exception e) {
            Log.w(TAG, "No HTTP cache for tiles: " + e);
        }
    }

    // ── Projection (Web Mercator, in pixels of a world at zoom z) ────────

    private double worldSize(double z) {
        return tileSize * Math.pow(2, z);
    }

    private static double mercatorX(double lng) {
        return (lng + 180) / 360;
    }

    private static double mercatorY(double lat) {
        double clamped = Math.max(-85.05112878, Math.min(85.05112878, lat));
        double rad = Math.toRadians(clamped);
        return (1 - Math.log(Math.tan(rad) + 1 / Math.cos(rad)) / Math.PI) / 2;
    }

    private static double longitudeOf(double x) {
        double lng = x * 360 - 180;
        return ((lng + 540) % 360) - 180;
    }

    private static double latitudeOf(double y) {
        double n = Math.PI - 2 * Math.PI * y;
        return Math.toDegrees(Math.atan(Math.sinh(n)));
    }

    private void panBy(float dx, float dy) {
        double world = worldSize(zoom);
        double x = mercatorX(longitude) + dx / world;
        double y = Math.max(0, Math.min(1, mercatorY(latitude) + dy / world));
        longitude = longitudeOf(x);
        latitude = latitudeOf(y);
        invalidate();
    }

    /** Zoom to {@code z}, keeping the map point under (fx, fy) where it is. */
    private void zoomAround(double z, float fx, float fy) {
        z = clampZoom(z);
        double before = worldSize(zoom);
        double after = worldSize(z);
        double ox = fx - getWidth() / 2.0;
        double oy = fy - getHeight() / 2.0;
        double px = mercatorX(longitude) + ox / before;
        double py = mercatorY(latitude) + oy / before;
        longitude = longitudeOf(px - ox / after);
        latitude = latitudeOf(Math.max(0, Math.min(1, py - oy / after)));
        zoom = z;
        invalidate();
    }

    // ── Drawing ──────────────────────────────────────────────────────────

    @Override
    protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        int width = getWidth();
        int height = getHeight();
        if (width == 0 || height == 0) return;

        int level = (int) Math.max(1, Math.min(MAX_ZOOM, Math.round(zoom)));
        int count = 1 << level;
        double scale = Math.pow(2, zoom - level);
        double size = tileSize * scale;
        double centerX = mercatorX(longitude) * count * size;
        double centerY = mercatorY(latitude) * count * size;
        double left = centerX - width / 2.0;
        double top = centerY - height / 2.0;

        int firstX = (int) Math.floor(left / size);
        int lastX = (int) Math.floor((left + width) / size);
        int firstY = Math.max(0, (int) Math.floor(top / size));
        int lastY = Math.min(count - 1, (int) Math.floor((top + height) / size));

        // Published before any request below, so the loaders see this frame's tiles.
        Set<String> frameTiles = java.util.concurrent.ConcurrentHashMap.newKeySet();
        wanted = frameTiles;
        RectF dst = new RectF();
        for (int ty = firstY; ty <= lastY; ty++) {
            for (int tx = firstX; tx <= lastX; tx++) {
                int wrapped = ((tx % count) + count) % count;
                float x = (float) (tx * size - left);
                float y = (float) (ty * size - top);
                dst.set(x, y, (float) (x + size), (float) (y + size));
                drawTile(canvas, level, wrapped, ty, dst, frameTiles);
            }
        }

        drawMarkers(canvas, left, top, count * size);
        drawAttribution(canvas, width, height);
    }

    private void drawTile(Canvas canvas, int z, int x, int y, RectF dst, Set<String> frameTiles) {
        String key = z + "/" + x + "/" + y;
        frameTiles.add(key);
        Bitmap bitmap = tiles.get(key);
        if (bitmap != null) {
            canvas.drawBitmap(bitmap, null, dst, tilePaint);
            return;
        }
        request(z, x, y, key);
        // Meanwhile, stretch the part of a cached ancestor that covers this tile.
        for (int up = 1; up <= 4 && z - up >= 0; up++) {
            Bitmap parent = tiles.get((z - up) + "/" + (x >> up) + "/" + (y >> up));
            if (parent == null) continue;
            int parts = 1 << up;
            int edge = parent.getWidth() / parts;
            int sx = (x % parts) * edge;
            int sy = (y % parts) * edge;
            canvas.drawBitmap(parent, new Rect(sx, sy, sx + edge, sy + edge), dst, tilePaint);
            return;
        }
    }

    private void request(int z, int x, int y, String key) {
        synchronized (loading) {
            if (loading.contains(key)) return;
            Long failed = failedAt.get(key);
            if (failed != null && SystemClock.uptimeMillis() - failed < 10_000) return;
            loading.add(key);
        }
        String url = tileUrl.replace("{z}", Integer.toString(z))
                .replace("{x}", Integer.toString(x))
                .replace("{y}", Integer.toString(y));
        loader.execute(() -> {
            Bitmap bitmap = null;
            if (!wanted.contains(key)) {
                // Scrolled or zoomed away before its turn; requested again if it comes back.
                synchronized (loading) {
                    loading.remove(key);
                }
                return;
            }
            try {
                HttpURLConnection connection = (HttpURLConnection) new URL(url).openConnection();
                connection.setRequestProperty("User-Agent", userAgent);
                connection.setConnectTimeout(10_000);
                connection.setReadTimeout(15_000);
                connection.setUseCaches(true);
                try (InputStream in = connection.getInputStream()) {
                    bitmap = BitmapFactory.decodeStream(in);
                } finally {
                    connection.disconnect();
                }
            } catch (Exception e) {
                Log.w(TAG, "tile " + key + ": " + e);
            } finally {
                synchronized (loading) {
                    loading.remove(key);
                    if (bitmap == null) failedAt.put(key, SystemClock.uptimeMillis());
                    else failedAt.remove(key);
                }
            }
            if (bitmap != null) {
                tiles.put(key, bitmap);
                postInvalidate();
            }
        });
    }

    private float[] markerPoint(Marker marker, double left, double top, double world) {
        double x = mercatorX(marker.longitude) * world - left;
        double y = mercatorY(marker.latitude) * world - top;
        // The copy of the world nearest the center.
        double width = getWidth();
        while (x < -world / 2 + width / 2) x += world;
        while (x > world / 2 + width / 2) x -= world;
        return new float[] {(float) x, (float) y};
    }

    private void drawMarkers(Canvas canvas, double left, double top, double world) {
        float radius = 7 * density;
        Marker selected = null;
        float[] selectedAt = null;
        for (Marker marker : markers.values()) {
            float[] p = markerPoint(marker, left, top, world);
            float headY = p[1] - 2.5f * radius;
            canvas.drawLine(p[0], p[1], p[0], headY, pinStroke);
            canvas.drawCircle(p[0], headY, radius, pinPaint);
            canvas.drawCircle(p[0], headY, radius, pinStroke);
            if (marker.id.equals(selectedMarker)) {
                selected = marker;
                selectedAt = p;
            }
        }
        if (selected != null && selected.title != null && !selected.title.isEmpty()) {
            String line2 = selected.snippet == null ? "" : selected.snippet;
            float pad = 8 * density;
            float textWidth = Math.max(labelPaint.measureText(selected.title), textPaint.measureText(line2));
            float lineHeight = labelPaint.getTextSize() * 1.3f;
            float boxHeight = pad * 2 + lineHeight + (line2.isEmpty() ? 0 : lineHeight);
            float boxBottom = selectedAt[1] - 4.5f * radius;
            RectF box = new RectF(selectedAt[0] - textWidth / 2 - pad, boxBottom - boxHeight,
                    selectedAt[0] + textWidth / 2 + pad, boxBottom);
            canvas.drawRoundRect(box, 6 * density, 6 * density, bubblePaint);
            canvas.drawText(selected.title, box.left + pad, box.top + pad + labelPaint.getTextSize(), labelPaint);
            if (!line2.isEmpty()) {
                canvas.drawText(line2, box.left + pad, box.top + pad + lineHeight + textPaint.getTextSize(), textPaint);
            }
        }
    }

    private String markerAt(float x, float y) {
        int level = (int) Math.max(1, Math.min(MAX_ZOOM, Math.round(zoom)));
        double size = tileSize * Math.pow(2, zoom - level);
        double world = (1 << level) * size;
        double left = mercatorX(longitude) * world - getWidth() / 2.0;
        double top = mercatorY(latitude) * world - getHeight() / 2.0;
        float radius = 7 * density;
        float slop = 16 * density;
        String hit = null;
        for (Marker marker : markers.values()) {
            float[] p = markerPoint(marker, left, top, world);
            float headY = p[1] - 2.5f * radius;
            if (Math.abs(x - p[0]) < slop && y > headY - slop && y < p[1] + slop / 2) hit = marker.id;
        }
        return hit;
    }

    private void drawAttribution(Canvas canvas, int width, int height) {
        if (attribution.isEmpty()) return;
        float pad = 4 * density;
        float textWidth = textPaint.measureText(attribution);
        float boxHeight = textPaint.getTextSize() + pad * 2;
        RectF box = new RectF(width - textWidth - pad * 2, height - boxHeight, width, height);
        canvas.drawRect(box, bubblePaint);
        canvas.drawText(attribution, box.left + pad, height - pad - textPaint.descent(), textPaint);
    }

    // ── Touch ────────────────────────────────────────────────────────────

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        boolean handled = scaleGestures.onTouchEvent(event);
        handled |= gestures.onTouchEvent(event);
        return handled || super.onTouchEvent(event);
    }

    // ── Control from Rust (any thread) ───────────────────────────────────

    private static void withMap(long viewId, java.util.function.Consumer<GpuiMapView> action) {
        mainHandler.post(() -> {
            View view = GpuiPlatformView.getView(viewId);
            if (view instanceof GpuiMapView) {
                action.accept((GpuiMapView) view);
            } else {
                Log.w(TAG, "No map with id=" + viewId);
            }
        });
    }

    public static void setCenter(long viewId, double latitude, double longitude) {
        withMap(viewId, map -> {
            map.latitude = latitude;
            map.longitude = longitude;
            map.invalidate();
        });
    }

    public static void setZoom(long viewId, double zoom) {
        withMap(viewId, map -> {
            map.zoom = clampZoom(zoom);
            map.invalidate();
        });
    }

    public static void addMarker(long viewId, String markerId, double latitude, double longitude,
            String title, String snippet) {
        withMap(viewId, map -> {
            map.markers.put(markerId, new Marker(markerId, latitude, longitude, title, snippet));
            map.invalidate();
        });
    }

    public static void removeMarker(long viewId, String markerId) {
        withMap(viewId, map -> {
            map.markers.remove(markerId);
            if (markerId.equals(map.selectedMarker)) map.selectedMarker = null;
            map.invalidate();
        });
    }

    public static void clearMarkers(long viewId) {
        withMap(viewId, map -> {
            map.markers.clear();
            map.selectedMarker = null;
            map.invalidate();
        });
    }

    /** {latitude, longitude, zoom}, or null if there is no map with that ID. */
    public static double[] getCamera(long viewId) {
        View view = GpuiPlatformView.getView(viewId);
        if (!(view instanceof GpuiMapView)) return null;
        GpuiMapView map = (GpuiMapView) view;
        return new double[] {map.latitude, map.longitude, map.zoom};
    }

    /** The ID of the marker whose bubble is open, or null. */
    public static String getSelectedMarker(long viewId) {
        View view = GpuiPlatformView.getView(viewId);
        return view instanceof GpuiMapView ? ((GpuiMapView) view).selectedMarker : null;
    }
}
