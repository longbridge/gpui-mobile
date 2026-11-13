package dev.gpui.mobile;

import android.Manifest;
import android.app.Activity;
import android.content.Context;
import android.graphics.ImageFormat;
import android.graphics.SurfaceTexture;
import android.hardware.camera2.*;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.Image;
import android.media.ImageReader;
import android.media.MediaRecorder;
import android.os.Environment;
import android.os.Handler;
import android.os.HandlerThread;
import android.util.Size;
import android.util.SparseArray;
import android.view.Surface;
import android.view.TextureView;
import android.view.ViewGroup;
import android.widget.FrameLayout;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.Semaphore;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Camera2 API helper for the GPUI camera package.
 *
 * <p>All public methods are static and called from Rust via JNI.</p>
 */
public final class GpuiCamera {

    private static final String TAG = "GpuiCamera";

    /** Resolution preset constants matching Rust's ResolutionPreset enum. */
    private static final int RES_LOW = 0;
    private static final int RES_MEDIUM = 1;
    private static final int RES_HIGH = 2;
    private static final int RES_VERY_HIGH = 3;
    private static final int RES_ULTRA_HIGH = 4;
    private static final int RES_MAX = 5;

    /** Flash mode constants matching Rust's FlashMode enum. */
    private static final int FLASH_OFF = 0;
    private static final int FLASH_AUTO = 1;
    private static final int FLASH_ALWAYS = 2;
    private static final int FLASH_TORCH = 3;

    // ── Session management ───────────────────────────────────────────────

    private static final AtomicInteger sNextId = new AtomicInteger(1);
    private static final SparseArray<CameraSession> sSessions = new SparseArray<>();

    static class CameraSession {
        String cameraId;
        CameraDevice device;
        CameraCaptureSession captureSession;
        CaptureRequest.Builder previewRequestBuilder;
        ImageReader imageReader;
        ImageReader photoReader;
        MediaRecorder mediaRecorder;
        HandlerThread backgroundThread;
        Handler backgroundHandler;
        TextureView textureView;
        Surface previewSurface;
        Size previewSize;
        boolean enableAudio;
        int flashMode = FLASH_OFF;
        String videoPath;
        boolean isRecording;
        float maxZoom = 1.0f;
        final Semaphore openLock = new Semaphore(1);
    }

    // ── Public API ───────────────────────────────────────────────────────

    /**
     * List available cameras.
     *
     * @return Array of "id|facing|orientation" strings.
     */
    public static String[] availableCameras(Activity activity) {
        try {
            CameraManager manager = (CameraManager) activity.getSystemService(Context.CAMERA_SERVICE);
            String[] ids = manager.getCameraIdList();
            String[] result = new String[ids.length];

            for (int i = 0; i < ids.length; i++) {
                CameraCharacteristics chars = manager.getCameraCharacteristics(ids[i]);
                Integer facing = chars.get(CameraCharacteristics.LENS_FACING);
                Integer orientation = chars.get(CameraCharacteristics.SENSOR_ORIENTATION);
                int facingInt = (facing != null) ? facing : -1;
                int orientInt = (orientation != null) ? orientation : 0;
                result[i] = ids[i] + "|" + facingInt + "|" + orientInt;
            }

            return result;
        } catch (CameraAccessException e) {
            android.util.Log.e(TAG, "Failed to list cameras", e);
            return new String[0];
        }
    }

    /**
     * Create a camera session.
     *
     * @return Session handle ID, or -1 on failure.
     */
    public static int createCamera(Activity activity, String cameraId, int resolution, boolean enableAudio) {
        try {
            CameraSession session = new CameraSession();
            session.cameraId = cameraId;
            session.enableAudio = enableAudio;

            // Start background thread
            session.backgroundThread = new HandlerThread("CameraBackground");
            session.backgroundThread.start();
            session.backgroundHandler = new Handler(session.backgroundThread.getLooper());

            // Get camera characteristics
            CameraManager manager = (CameraManager) activity.getSystemService(Context.CAMERA_SERVICE);
            CameraCharacteristics chars = manager.getCameraCharacteristics(cameraId);

            // Get max zoom
            Float maxZoom = chars.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM);
            if (maxZoom != null) {
                session.maxZoom = maxZoom;
            }

            // Choose preview size based on resolution
            StreamConfigurationMap map = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
            if (map != null) {
                Size[] sizes = map.getOutputSizes(SurfaceTexture.class);
                session.previewSize = chooseSize(sizes, resolution);
            } else {
                session.previewSize = new Size(1280, 720);
            }

            // Create photo ImageReader
            session.photoReader = ImageReader.newInstance(
                    session.previewSize.getWidth(),
                    session.previewSize.getHeight(),
                    ImageFormat.JPEG,
                    2
            );

            // Open camera
            CountDownLatch openLatch = new CountDownLatch(1);
            AtomicReference<CameraDevice> deviceRef = new AtomicReference<>(null);

            manager.openCamera(cameraId, new CameraDevice.StateCallback() {
                @Override
                public void onOpened(CameraDevice camera) {
                    deviceRef.set(camera);
                    openLatch.countDown();
                }

                @Override
                public void onDisconnected(CameraDevice camera) {
                    camera.close();
                    openLatch.countDown();
                }

                @Override
                public void onError(CameraDevice camera, int error) {
                    camera.close();
                    openLatch.countDown();
                }
            }, session.backgroundHandler);

            if (!openLatch.await(5, TimeUnit.SECONDS)) {
                return -1;
            }

            session.device = deviceRef.get();
            if (session.device == null) {
                return -1;
            }

            int id = sNextId.getAndIncrement();
            synchronized (sSessions) {
                sSessions.put(id, session);
            }
            return id;

        } catch (Exception e) {
            android.util.Log.e(TAG, "createCamera failed", e);
            return -1;
        }
    }

    /**
     * Start camera preview by adding a TextureView to the activity.
     */
    public static void startPreview(Activity activity, int handleId) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null || session.device == null) return;

        final CameraSession s = session;
        CountDownLatch latch = new CountDownLatch(1);

        activity.runOnUiThread(() -> {
            try {
                // Create TextureView for preview
                TextureView tv = new TextureView(activity);
                FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT);
                activity.addContentView(tv, params);
                s.textureView = tv;

                tv.setSurfaceTextureListener(new TextureView.SurfaceTextureListener() {
                    @Override
                    public void onSurfaceTextureAvailable(SurfaceTexture surface, int width, int height) {
                        surface.setDefaultBufferSize(s.previewSize.getWidth(), s.previewSize.getHeight());
                        s.previewSurface = new Surface(surface);
                        createCaptureSession(s);
                        latch.countDown();
                    }
                    @Override
                    public void onSurfaceTextureSizeChanged(SurfaceTexture surface, int w, int h) {}
                    @Override
                    public boolean onSurfaceTextureDestroyed(SurfaceTexture surface) { return true; }
                    @Override
                    public void onSurfaceTextureUpdated(SurfaceTexture surface) {}
                });

                // If texture is already available
                if (tv.isAvailable()) {
                    SurfaceTexture st = tv.getSurfaceTexture();
                    st.setDefaultBufferSize(s.previewSize.getWidth(), s.previewSize.getHeight());
                    s.previewSurface = new Surface(st);
                    createCaptureSession(s);
                    latch.countDown();
                }
            } catch (Exception e) {
                android.util.Log.e(TAG, "startPreview failed", e);
                latch.countDown();
            }
        });

        try {
            latch.await(5, TimeUnit.SECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        }
    }

    /**
     * Stop the camera preview.
     */
    public static void stopPreview(int handleId) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null) return;

        try {
            if (session.captureSession != null) {
                session.captureSession.stopRepeating();
                session.captureSession.close();
                session.captureSession = null;
            }
            if (session.textureView != null) {
                ViewGroup parent = (ViewGroup) session.textureView.getParent();
                if (parent != null) {
                    parent.removeView(session.textureView);
                }
                session.textureView = null;
            }
        } catch (Exception e) {
            android.util.Log.e(TAG, "stopPreview failed", e);
        }
    }

    /**
     * Take a still photo.
     *
     * @return "path|width|height" or null on failure.
     */
    public static String takePicture(int handleId) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null || session.device == null) return null;

        try {
            CountDownLatch latch = new CountDownLatch(1);
            AtomicReference<String> resultRef = new AtomicReference<>(null);

            session.photoReader.setOnImageAvailableListener(reader -> {
                try (Image image = reader.acquireLatestImage()) {
                    if (image != null) {
                        ByteBuffer buffer = image.getPlanes()[0].getBuffer();
                        byte[] bytes = new byte[buffer.remaining()];
                        buffer.get(bytes);

                        String fileName = "photo_" + UUID.randomUUID().toString() + ".jpg";
                        File file = new File(Environment.getExternalStoragePublicDirectory(
                                Environment.DIRECTORY_PICTURES), fileName);
                        // Fallback to cache dir
                        if (!file.getParentFile().exists()) {
                            file = new File(System.getProperty("java.io.tmpdir"), fileName);
                        }

                        try (FileOutputStream fos = new FileOutputStream(file)) {
                            fos.write(bytes);
                        }

                        resultRef.set(file.getAbsolutePath() + "|" +
                                image.getWidth() + "|" + image.getHeight());
                    }
                } catch (IOException e) {
                    android.util.Log.e(TAG, "Failed to save photo", e);
                } finally {
                    latch.countDown();
                }
            }, session.backgroundHandler);

            CaptureRequest.Builder captureBuilder = session.device.createCaptureRequest(
                    CameraDevice.TEMPLATE_STILL_CAPTURE);
            captureBuilder.addTarget(session.photoReader.getSurface());
            captureBuilder.set(CaptureRequest.CONTROL_AF_MODE,
                    CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE);
            applyFlashMode(captureBuilder, session.flashMode);

            session.captureSession.capture(captureBuilder.build(),
                    new CameraCaptureSession.CaptureCallback() {}, session.backgroundHandler);

            latch.await(10, TimeUnit.SECONDS);
            return resultRef.get();

        } catch (Exception e) {
            android.util.Log.e(TAG, "takePicture failed", e);
            return null;
        }
    }

    /**
     * Start video recording.
     */
    public static void startVideoRecording(int handleId) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null || session.device == null || session.isRecording) return;

        try {
            String fileName = "video_" + UUID.randomUUID().toString() + ".mp4";
            File file = new File(System.getProperty("java.io.tmpdir"), fileName);
            session.videoPath = file.getAbsolutePath();

            session.mediaRecorder = new MediaRecorder();
            if (session.enableAudio) {
                session.mediaRecorder.setAudioSource(MediaRecorder.AudioSource.MIC);
            }
            session.mediaRecorder.setVideoSource(MediaRecorder.VideoSource.SURFACE);
            session.mediaRecorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);
            session.mediaRecorder.setVideoEncoder(MediaRecorder.VideoEncoder.H264);
            if (session.enableAudio) {
                session.mediaRecorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC);
            }
            session.mediaRecorder.setVideoSize(session.previewSize.getWidth(), session.previewSize.getHeight());
            session.mediaRecorder.setVideoFrameRate(30);
            session.mediaRecorder.setOutputFile(session.videoPath);
            session.mediaRecorder.prepare();

            // Close existing session
            if (session.captureSession != null) {
                session.captureSession.close();
                session.captureSession = null;
            }

            // Create new session with recorder surface
            List<Surface> surfaces = new ArrayList<>();
            if (session.previewSurface != null) surfaces.add(session.previewSurface);
            Surface recorderSurface = session.mediaRecorder.getSurface();
            surfaces.add(recorderSurface);

            CaptureRequest.Builder builder = session.device.createCaptureRequest(CameraDevice.TEMPLATE_RECORD);
            if (session.previewSurface != null) builder.addTarget(session.previewSurface);
            builder.addTarget(recorderSurface);
            builder.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO);

            CountDownLatch latch = new CountDownLatch(1);

            session.device.createCaptureSession(surfaces, new CameraCaptureSession.StateCallback() {
                @Override
                public void onConfigured(CameraCaptureSession s) {
                    session.captureSession = s;
                    try {
                        s.setRepeatingRequest(builder.build(), null, session.backgroundHandler);
                        session.mediaRecorder.start();
                        session.isRecording = true;
                    } catch (Exception e) {
                        android.util.Log.e(TAG, "Failed to start recording", e);
                    }
                    latch.countDown();
                }

                @Override
                public void onConfigureFailed(CameraCaptureSession s) {
                    android.util.Log.e(TAG, "Recording session configure failed");
                    latch.countDown();
                }
            }, session.backgroundHandler);

            latch.await(5, TimeUnit.SECONDS);

        } catch (Exception e) {
            android.util.Log.e(TAG, "startVideoRecording failed", e);
        }
    }

    /**
     * Stop video recording.
     *
     * @return The video file path, or null on failure.
     */
    public static String stopVideoRecording(int handleId) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null || !session.isRecording) return null;

        try {
            session.isRecording = false;
            session.mediaRecorder.stop();
            session.mediaRecorder.release();
            session.mediaRecorder = null;

            // Re-create preview session
            if (session.previewSurface != null) {
                createCaptureSession(session);
            }

            return session.videoPath;
        } catch (Exception e) {
            android.util.Log.e(TAG, "stopVideoRecording failed", e);
            return null;
        }
    }

    public static void setFlashMode(int handleId, int mode) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null) return;
        session.flashMode = mode;

        // Apply to active preview if running
        if (session.captureSession != null && session.previewRequestBuilder != null) {
            try {
                applyFlashMode(session.previewRequestBuilder, mode);
                session.captureSession.setRepeatingRequest(
                        session.previewRequestBuilder.build(), null, session.backgroundHandler);
            } catch (Exception e) {
                android.util.Log.e(TAG, "setFlashMode failed", e);
            }
        }
    }

    public static void setFocusMode(int handleId, int mode) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null || session.previewRequestBuilder == null) return;

        try {
            int afMode = (mode == 0) ?
                    CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE :
                    CaptureRequest.CONTROL_AF_MODE_AUTO;
            session.previewRequestBuilder.set(CaptureRequest.CONTROL_AF_MODE, afMode);
            if (session.captureSession != null) {
                session.captureSession.setRepeatingRequest(
                        session.previewRequestBuilder.build(), null, session.backgroundHandler);
            }
        } catch (Exception e) {
            android.util.Log.e(TAG, "setFocusMode failed", e);
        }
    }

    public static void setExposureMode(int handleId, int mode) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null || session.previewRequestBuilder == null) return;

        try {
            int aeMode = (mode == 0) ?
                    CaptureRequest.CONTROL_AE_MODE_ON :
                    CaptureRequest.CONTROL_AE_MODE_OFF;
            session.previewRequestBuilder.set(CaptureRequest.CONTROL_AE_MODE, aeMode);
            if (session.captureSession != null) {
                session.captureSession.setRepeatingRequest(
                        session.previewRequestBuilder.build(), null, session.backgroundHandler);
            }
        } catch (Exception e) {
            android.util.Log.e(TAG, "setExposureMode failed", e);
        }
    }

    public static float getMinZoom(int handleId) {
        return 1.0f;
    }

    public static float getMaxZoom(int handleId) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        return (session != null) ? session.maxZoom : 1.0f;
    }

    public static void setZoom(int handleId, float zoom) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null || session.previewRequestBuilder == null) return;

        try {
            float clampedZoom = Math.max(1.0f, Math.min(zoom, session.maxZoom));
            // Digital zoom via SCALER_CROP_REGION
            // (simplified — a real implementation would compute the crop rect from sensor active area)
            session.previewRequestBuilder.set(CaptureRequest.CONTROL_ZOOM_RATIO, clampedZoom);
            if (session.captureSession != null) {
                session.captureSession.setRepeatingRequest(
                        session.previewRequestBuilder.build(), null, session.backgroundHandler);
            }
        } catch (Exception e) {
            android.util.Log.e(TAG, "setZoom failed", e);
        }
    }

    public static void setCamera(int handleId, String newCameraId) {
        // Switch cameras requires re-creating the session
        // For now, log a warning
        android.util.Log.w(TAG, "setCamera: switching cameras at runtime is not yet implemented for Android");
    }

    public static void dispose(int handleId) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
            sSessions.remove(handleId);
        }
        if (session == null) return;

        try {
            if (session.isRecording && session.mediaRecorder != null) {
                session.mediaRecorder.stop();
                session.mediaRecorder.release();
            }
            if (session.captureSession != null) {
                session.captureSession.close();
            }
            if (session.device != null) {
                session.device.close();
            }
            if (session.photoReader != null) {
                session.photoReader.close();
            }
            if (session.backgroundThread != null) {
                session.backgroundThread.quitSafely();
            }
        } catch (Exception e) {
            android.util.Log.e(TAG, "dispose failed", e);
        }
    }

    // ── Internal helpers ─────────────────────────────────────────────────

    private static void createCaptureSession(CameraSession session) {
        try {
            List<Surface> surfaces = new ArrayList<>();
            surfaces.add(session.previewSurface);
            surfaces.add(session.photoReader.getSurface());

            session.previewRequestBuilder = session.device.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW);
            session.previewRequestBuilder.addTarget(session.previewSurface);
            session.previewRequestBuilder.set(CaptureRequest.CONTROL_AF_MODE,
                    CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE);
            applyFlashMode(session.previewRequestBuilder, session.flashMode);

            session.device.createCaptureSession(surfaces, new CameraCaptureSession.StateCallback() {
                @Override
                public void onConfigured(CameraCaptureSession s) {
                    session.captureSession = s;
                    try {
                        s.setRepeatingRequest(session.previewRequestBuilder.build(), null, session.backgroundHandler);
                    } catch (CameraAccessException e) {
                        android.util.Log.e(TAG, "Failed to start preview", e);
                    }
                }

                @Override
                public void onConfigureFailed(CameraCaptureSession s) {
                    android.util.Log.e(TAG, "Capture session configure failed");
                }
            }, session.backgroundHandler);

        } catch (CameraAccessException e) {
            android.util.Log.e(TAG, "createCaptureSession failed", e);
        }
    }

    private static Size chooseSize(Size[] sizes, int resolution) {
        int targetWidth, targetHeight;
        switch (resolution) {
            case RES_LOW:       targetWidth = 320;  targetHeight = 240;  break;
            case RES_MEDIUM:    targetWidth = 640;  targetHeight = 480;  break;
            case RES_HIGH:      targetWidth = 1280; targetHeight = 720;  break;
            case RES_VERY_HIGH: targetWidth = 1920; targetHeight = 1080; break;
            case RES_ULTRA_HIGH:targetWidth = 3840; targetHeight = 2160; break;
            case RES_MAX:       targetWidth = Integer.MAX_VALUE; targetHeight = Integer.MAX_VALUE; break;
            default:            targetWidth = 1280; targetHeight = 720;  break;
        }

        Size best = sizes[0];
        int bestDiff = Integer.MAX_VALUE;

        for (Size size : sizes) {
            int diff = Math.abs(size.getWidth() - targetWidth) + Math.abs(size.getHeight() - targetHeight);
            if (diff < bestDiff) {
                bestDiff = diff;
                best = size;
            }
        }

        return best;
    }

    private static void applyFlashMode(CaptureRequest.Builder builder, int flashMode) {
        switch (flashMode) {
            case FLASH_OFF:
                builder.set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_OFF);
                builder.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON);
                break;
            case FLASH_AUTO:
                builder.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON_AUTO_FLASH);
                break;
            case FLASH_ALWAYS:
                builder.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON_ALWAYS_FLASH);
                break;
            case FLASH_TORCH:
                builder.set(CaptureRequest.FLASH_MODE, CaptureRequest.FLASH_MODE_TORCH);
                builder.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON);
                break;
        }
    }

    /**
     * Create a TextureView for camera preview.
     *
     * <p>The TextureView is wired to the CameraSession. When the surface
     * becomes available, a capture session is created automatically.
     * The view is NOT added to the hierarchy — the caller (GpuiPlatformView) handles that.</p>
     *
     * @param context   The view context (the platform view host's, which follows Activity recreation).
     * @param handleId  Camera session handle ID.
     * @return A configured TextureView, or an empty FrameLayout if the session is not found.
     */
    public static android.view.View createPreviewSurface(Context context, int handleId) {
        CameraSession session;
        synchronized (sSessions) {
            session = sSessions.get(handleId);
        }
        if (session == null || session.device == null) {
            android.util.Log.w(TAG, "createPreviewSurface: session " + handleId + " not found");
            return new android.widget.FrameLayout(context);
        }

        final CameraSession s = session;
        android.view.TextureView tv = new android.view.TextureView(context);
        s.textureView = tv;

        tv.setSurfaceTextureListener(new android.view.TextureView.SurfaceTextureListener() {
            @Override
            public void onSurfaceTextureAvailable(android.graphics.SurfaceTexture surface, int width, int height) {
                surface.setDefaultBufferSize(s.previewSize.getWidth(), s.previewSize.getHeight());
                s.previewSurface = new android.view.Surface(surface);
                createCaptureSession(s);
            }

            @Override
            public void onSurfaceTextureSizeChanged(android.graphics.SurfaceTexture surface, int w, int h) {}

            @Override
            public boolean onSurfaceTextureDestroyed(android.graphics.SurfaceTexture surface) {
                return true;
            }

            @Override
            public void onSurfaceTextureUpdated(android.graphics.SurfaceTexture surface) {}
        });

        // If texture is already available
        if (tv.isAvailable()) {
            android.graphics.SurfaceTexture st = tv.getSurfaceTexture();
            st.setDefaultBufferSize(s.previewSize.getWidth(), s.previewSize.getHeight());
            s.previewSurface = new android.view.Surface(st);
            createCaptureSession(s);
        }

        // Start the session running
        // (capture session creation is async via the SurfaceTextureListener)

        return tv;
    }

    private GpuiCamera() {}
}
