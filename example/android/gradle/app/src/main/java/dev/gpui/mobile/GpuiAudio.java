package dev.gpui.mobile;

import android.app.Activity;
import android.media.MediaPlayer;
import android.media.PlaybackParams;
import android.os.Build;
import android.util.SparseArray;

import java.io.IOException;

/**
 * Audio playback helper for the GPUI audio package.
 *
 * <p>Uses {@link MediaPlayer} for audio playback. All public methods are static
 * and called from Rust via JNI.</p>
 *
 * <p>Streams are prepared asynchronously, so no call blocks on the network: until
 * the stream is ready, play, pause and seek are remembered and applied then.
 * MediaPlayer delivers its callbacks on the main thread, since the threads that
 * create players have no Looper.</p>
 */
public final class GpuiAudio {

    private static final String TAG = "GpuiAudio";
    private static final SparseArray<MediaPlayer> sPlayers = new SparseArray<>();
    private static final SparseArray<Pending> sPending = new SparseArray<>();
    private static int sNextId = 1;

    /**
     * A player whose source is still being prepared, and what to do once it is.
     * Read and written only while holding {@code sPlayers}.
     */
    private static final class Pending {
        boolean playWhenReady;
        long seekMs = -1;
        float speed = -1;
    }

    /** Players whose source failed to load, with the reason. */
    private static final SparseArray<String> sErrors = new SparseArray<>();

    // State codes for getState (must match Rust's audio::android).
    private static final int STATE_LOADING = 1;
    private static final int STATE_READY = 2;
    private static final int STATE_PLAYING = 3;
    private static final int STATE_PAUSED = 4;
    private static final int STATE_COMPLETED = 5;

    /**
     * Create a new audio player.
     *
     * @return Player ID, or -1 on failure.
     */
    public static int create(Activity activity) {
        try {
            int id = sNextId++;
            MediaPlayer mp = new MediaPlayer();
            synchronized (sPlayers) {
                sPlayers.put(id, mp);
            }
            return id;
        } catch (Exception e) {
            android.util.Log.e(TAG, "create failed", e);
            return -1;
        }
    }

    /**
     * Set the audio source from a URL or file path.
     *
     * <p>A local file is prepared right away. A stream is prepared in the background;
     * {@link #getState} reports loading until it is ready, or throws its error.</p>
     *
     * @return Duration in milliseconds, or -1 if not known yet.
     */
    public static long setUrl(Activity activity, int id, String url) throws IOException {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
            sErrors.remove(id);
            sPending.remove(id);
        }
        if (mp == null) throw new IllegalArgumentException("No audio player " + id);

        mp.reset();
        mp.setDataSource(url);
        if (url.startsWith("/") || url.startsWith("file:")) {
            mp.prepare();
            return mp.getDuration();
        }
        prepareAsync(id, mp, new Pending());
        return -1;
    }

    private static void prepareAsync(int id, MediaPlayer mp, Pending pending) {
        synchronized (sPlayers) {
            sPending.put(id, pending);
        }
        mp.setOnPreparedListener(player -> {
            long seekMs;
            float speed;
            boolean play;
            synchronized (sPlayers) {
                Pending ready = sPending.get(id);
                if (ready == null) return;
                sPending.remove(id);
                seekMs = ready.seekMs;
                speed = ready.speed;
                play = ready.playWhenReady;
            }
            if (seekMs >= 0) player.seekTo((int) seekMs);
            if (speed > 0) applySpeed(player, speed);
            if (play) player.start();
        });
        mp.setOnErrorListener((player, what, extra) -> {
            synchronized (sPlayers) {
                sPending.remove(id);
                sErrors.put(id, "MediaPlayer error " + what + " (" + extra + ")");
            }
            return true;
        });
        mp.prepareAsync();
    }

    /** Whether the player is still preparing. */
    private static boolean preparing(int id) {
        synchronized (sPlayers) {
            return sPending.get(id) != null;
        }
    }

    /**
     * If the player is still preparing, record {@code change} for when it is ready
     * and return true; otherwise return false and leave the call to the player.
     */
    private static boolean whenPrepared(int id, java.util.function.Consumer<Pending> change) {
        synchronized (sPlayers) {
            Pending pending = sPending.get(id);
            if (pending == null) return false;
            change.accept(pending);
            return true;
        }
    }

    /**
     * Start or resume playback.
     */
    public static void play(int id) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null) return;

        if (whenPrepared(id, pending -> pending.playWhenReady = true)) return;
        try {
            mp.start();
        } catch (IllegalStateException e) {
            android.util.Log.e(TAG, "play failed", e);
        }
    }

    /**
     * Pause playback.
     */
    public static void pause(int id) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null) return;

        if (whenPrepared(id, pending -> pending.playWhenReady = false)) return;
        try {
            if (mp.isPlaying()) {
                mp.pause();
            }
        } catch (IllegalStateException e) {
            android.util.Log.e(TAG, "pause failed", e);
        }
    }

    /**
     * Stop playback and reset to the beginning.
     */
    public static void stop(int id) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null) return;

        // Still preparing: just don't start when ready.
        if (whenPrepared(id, pending -> pending.playWhenReady = false)) return;
        try {
            mp.stop();
            // Back to prepared, at the start, without blocking on the network.
            Pending pending = new Pending();
            pending.seekMs = 0;
            prepareAsync(id, mp, pending);
        } catch (Exception e) {
            android.util.Log.e(TAG, "stop failed", e);
        }
    }

    /**
     * Seek to position in milliseconds.
     */
    public static void seek(int id, long positionMs) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null) return;

        if (whenPrepared(id, pending -> pending.seekMs = positionMs)) return;
        try {
            mp.seekTo((int) positionMs);
        } catch (IllegalStateException e) {
            android.util.Log.e(TAG, "seek failed", e);
        }
    }

    /**
     * Set volume (0.0 to 1.0).
     */
    public static void setVolume(int id, float volume) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null) return;

        try {
            float v = Math.max(0.0f, Math.min(1.0f, volume));
            mp.setVolume(v, v);
        } catch (IllegalStateException e) {
            android.util.Log.e(TAG, "setVolume failed", e);
        }
    }

    /**
     * Set playback speed (requires API 23+).
     */
    public static void setSpeed(int id, float speed) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null) return;

        if (whenPrepared(id, pending -> pending.speed = speed)) return;
        applySpeed(mp, speed);
    }

    private static void applySpeed(MediaPlayer mp, float speed) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            try {
                PlaybackParams params = mp.getPlaybackParams();
                params.setSpeed(speed);
                mp.setPlaybackParams(params);
            } catch (Exception e) {
                android.util.Log.e(TAG, "setSpeed failed", e);
            }
        } else {
            android.util.Log.w(TAG, "setSpeed requires API 23+, current: " + Build.VERSION.SDK_INT);
        }
    }

    /**
     * Set looping mode.
     */
    public static void setLooping(int id, boolean looping) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null) return;

        try {
            mp.setLooping(looping);
        } catch (IllegalStateException e) {
            android.util.Log.e(TAG, "setLooping failed", e);
        }
    }

    /**
     * Get current playback position in milliseconds.
     */
    public static long getPosition(int id) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null || preparing(id)) return -1;

        try {
            return mp.getCurrentPosition();
        } catch (IllegalStateException e) {
            return -1;
        }
    }

    /**
     * Get total duration in milliseconds.
     */
    public static long getDuration(int id) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null || preparing(id)) return -1;

        try {
            return mp.getDuration();
        } catch (IllegalStateException e) {
            return -1;
        }
    }

    /**
     * Check if currently playing.
     */
    public static boolean isPlaying(int id) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
        }
        if (mp == null) return false;

        try {
            return mp.isPlaying();
        } catch (IllegalStateException e) {
            return false;
        }
    }

    /**
     * The player's state: 1 loading, 2 ready, 3 playing, 4 paused, 5 completed.
     *
     * @throws IllegalStateException with the reason, if the source failed to load.
     */
    public static int getState(int id) {
        MediaPlayer mp;
        String error;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
            error = sErrors.get(id);
        }
        if (mp == null) throw new IllegalArgumentException("No audio player " + id);
        if (error != null) throw new IllegalStateException(error);
        if (preparing(id)) return STATE_LOADING;
        try {
            if (mp.isPlaying()) return STATE_PLAYING;
            int position = mp.getCurrentPosition();
            int duration = mp.getDuration();
            if (duration > 0 && position >= duration) return STATE_COMPLETED;
            return position > 0 ? STATE_PAUSED : STATE_READY;
        } catch (IllegalStateException e) {
            return STATE_READY;
        }
    }

    /**
     * Release the player and free resources.
     */
    public static void dispose(int id) {
        MediaPlayer mp;
        synchronized (sPlayers) {
            mp = sPlayers.get(id);
            sPlayers.remove(id);
            sPending.remove(id);
            sErrors.remove(id);
        }
        if (mp == null) return;

        try {
            mp.release();
        } catch (Exception e) {
            android.util.Log.e(TAG, "dispose failed", e);
        }
    }

    private GpuiAudio() {}
}
