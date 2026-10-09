package dev.gpui.mobile;

import android.Manifest;
import android.app.Activity;
import android.content.Context;
import android.content.pm.PackageManager;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.os.Bundle;
import android.os.Looper;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Java helper for location services, called from Rust via JNI.
 *
 * <p>Uses {@link LocationManager} (no Google Play Services dependency).
 * All public methods are {@code static}.</p>
 *
 * <p>Position data is returned as a pipe-delimited String:
 * "lat|lon|alt|accuracy|speed|speedAccuracy|heading|headingAccuracy|timestamp"</p>
 */
public final class GpuiLocation {

    private static final String TAG = "GpuiLocation";
    private static final long TIMEOUT_SECONDS = 30;

    /**
     * Check whether location services (GPS or network) are enabled on the device.
     *
     * @param activity The current Activity.
     * @return true if at least one location provider is enabled.
     */
    public static boolean isLocationEnabled(Activity activity) {
        LocationManager lm = (LocationManager) activity.getSystemService(Context.LOCATION_SERVICE);
        if (lm == null) return false;
        boolean gps = false;
        boolean network = false;
        try {
            gps = lm.isProviderEnabled(LocationManager.GPS_PROVIDER);
        } catch (Exception ignored) {}
        try {
            network = lm.isProviderEnabled(LocationManager.NETWORK_PROVIDER);
        } catch (Exception ignored) {}
        return gps || network;
    }

    /**
     * Request a single location update and block until it arrives (or timeout).
     *
     * @param activity The current Activity.
     * @param accuracy Accuracy level: 0=lowest, 1=low, 2=medium, 3=high, 4=best, 5=bestForNavigation.
     * @return A pipe-delimited string of 9 values.
     * @throws SecurityException without a location permission.
     * @throws IllegalStateException with no enabled provider, or no fix in time.
     */
    public static String getCurrentPosition(Activity activity, int accuracy) throws InterruptedException {
        requirePermission(activity);
        LocationManager lm = (LocationManager) activity.getSystemService(Context.LOCATION_SERVICE);
        if (lm == null) throw new IllegalStateException("No LocationManager");

        String provider = pickProvider(lm, accuracy);
        if (provider == null) throw new IllegalStateException("No location provider is enabled");

        final CountDownLatch latch = new CountDownLatch(1);
        final AtomicReference<Location> locationRef = new AtomicReference<>(null);

        LocationListener listener = new LocationListener() {
            @Override
            public void onLocationChanged(Location location) {
                locationRef.set(location);
                latch.countDown();
            }

            @Override
            public void onStatusChanged(String provider, int status, Bundle extras) {}

            @Override
            public void onProviderEnabled(String provider) {}

            @Override
            public void onProviderDisabled(String provider) {}
        };

        lm.requestSingleUpdate(provider, listener, Looper.getMainLooper());
        try {
            if (!latch.await(TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
                throw new IllegalStateException("No location fix within " + TIMEOUT_SECONDS + " s");
            }
        } finally {
            lm.removeUpdates(listener);
        }

        Location loc = locationRef.get();
        if (loc == null) return null;
        return locationToString(loc);
    }

    /**
     * Get the last known location from available providers.
     *
     * @param activity The current Activity.
     * @return A pipe-delimited string of 9 values, or null if no cached location exists.
     * @throws SecurityException without a location permission.
     */
    public static String getLastKnownPosition(Activity activity) {
        requirePermission(activity);
        LocationManager lm = (LocationManager) activity.getSystemService(Context.LOCATION_SERVICE);
        if (lm == null) return null;

        Location best = null;

        // Try GPS first, then network, then passive — pick the most recent
        try {
            Location gps = lm.getLastKnownLocation(LocationManager.GPS_PROVIDER);
            if (gps != null) best = gps;
        } catch (SecurityException ignored) {}

        try {
            Location net = lm.getLastKnownLocation(LocationManager.NETWORK_PROVIDER);
            if (net != null) {
                if (best == null || net.getTime() > best.getTime()) {
                    best = net;
                }
            }
        } catch (SecurityException ignored) {}

        try {
            Location passive = lm.getLastKnownLocation(LocationManager.PASSIVE_PROVIDER);
            if (passive != null) {
                if (best == null || passive.getTime() > best.getTime()) {
                    best = passive;
                }
            }
        } catch (SecurityException ignored) {}

        if (best == null) return null;
        return locationToString(best);
    }

    /**
     * Convert a Location to a pipe-delimited string:
     * "lat|lon|alt|accuracy|speed|speedAccuracy|heading|headingAccuracy|timestamp"
     */
    private static String locationToString(Location loc) {
        double lat = loc.getLatitude();
        double lon = loc.getLongitude();
        double alt = loc.hasAltitude() ? loc.getAltitude() : 0.0;
        double acc = loc.hasAccuracy() ? loc.getAccuracy() : 0.0;
        double spd = loc.hasSpeed() ? loc.getSpeed() : 0.0;
        double spdAcc = loc.hasSpeedAccuracy() ? loc.getSpeedAccuracyMetersPerSecond() : 0.0;
        double hdg = loc.hasBearing() ? loc.getBearing() : 0.0;
        double hdgAcc = loc.hasBearingAccuracy() ? loc.getBearingAccuracyDegrees() : 0.0;
        double ts = (double) loc.getTime();

        return lat + "|" + lon + "|" + alt + "|" + acc + "|"
                + spd + "|" + spdAcc + "|" + hdg + "|" + hdgAcc + "|" + ts;
    }

    /** Throw, rather than read "no location", when the app may not read it. */
    private static void requirePermission(Activity activity) {
        if (activity.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) != PackageManager.PERMISSION_GRANTED
                && activity.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) != PackageManager.PERMISSION_GRANTED) {
            throw new SecurityException("Location needs ACCESS_FINE_LOCATION or ACCESS_COARSE_LOCATION");
        }
    }

    /**
     * Pick the best provider based on the requested accuracy level.
     */
    private static String pickProvider(LocationManager lm, int accuracy) {
        // For high/best/bestForNavigation, prefer GPS
        if (accuracy >= 3) {
            if (lm.isProviderEnabled(LocationManager.GPS_PROVIDER)) {
                return LocationManager.GPS_PROVIDER;
            }
        }
        // For low/medium or when GPS is unavailable, try network
        if (lm.isProviderEnabled(LocationManager.NETWORK_PROVIDER)) {
            return LocationManager.NETWORK_PROVIDER;
        }
        // Fallback to GPS if network not available
        if (lm.isProviderEnabled(LocationManager.GPS_PROVIDER)) {
            return LocationManager.GPS_PROVIDER;
        }
        // Last resort: passive
        if (lm.isProviderEnabled(LocationManager.PASSIVE_PROVIDER)) {
            return LocationManager.PASSIVE_PROVIDER;
        }
        return null;
    }

    // Prevent instantiation.
    private GpuiLocation() {}
}
