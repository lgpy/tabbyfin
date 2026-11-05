package io.github.lgpy.tabbyfin;

import android.app.Activity;
import android.content.pm.ActivityInfo;
import android.provider.Settings;
import android.view.Gravity;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowManager;
import android.widget.FrameLayout;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;

/**
 * The SurfaceView the player draws into: under the app's own surface, which stays transparent
 * over it (MainActivity makes GameActivity's surface a translucent media overlay). There's one,
 * shown while a player plays; Rust places it where the player page shows the video
 * (`crates/player/src/android.rs`) and is told when its surface comes and goes.
 *
 * Everything runs on the UI thread, in the order Rust asked for it: a player's hide and the next
 * one's show can come back to back.
 */
final class VideoSurface implements SurfaceHolder.Callback {
    private static Activity activity;
    private static VideoSurface shown;
    /** Where it goes, in window pixels: left, top, width, height. Kept across show/hide. */
    private static int[] place = {0, 0, 1, 1};
    /**
     * The video's width over its height, to fit the view inside the place at; 0 fills the place.
     * mpv letterboxes the video itself, while Media3 stretches it over the whole surface.
     */
    private static float aspect;

    private final SurfaceView view;

    private VideoSurface() {
        view = new SurfaceView(activity);
        view.getHolder().addCallback(this);
        activity.addContentView(view, layout());
    }

    /** Shows the surface. Called by Rust, on any thread. */
    static void show(Activity activity) {
        VideoSurface.activity = activity;
        activity.runOnUiThread(() -> {
            if (shown == null) shown = new VideoSurface();
        });
    }

    /** Takes it away; the player is told the surface is gone first. Called by Rust. */
    static void hide() {
        if (activity == null) return;
        activity.runOnUiThread(() -> {
            aspect = 0;
            if (shown == null) return;
            if (shown.view.getParent() instanceof ViewGroup) {
                ((ViewGroup) shown.view.getParent()).removeView(shown.view);
            }
            shown = null;
        });
    }

    /**
     * The window while the player page is open (`on`), from its first item to its last: the
     * screen kept on, landscape (either way up, by the sensor) and the system bars hidden until
     * swiped in. Closing it undoes that and gives the brightness back to the system. Called by
     * Rust.
     */
    static void watching(Activity activity, boolean on) {
        VideoSurface.activity = activity;
        activity.runOnUiThread(() -> {
            Window window = activity.getWindow();
            WindowInsetsControllerCompat bars = WindowCompat.getInsetsController(window, window.getDecorView());
            if (on) {
                window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                activity.setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE);
                bars.setSystemBarsBehavior(WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                bars.hide(WindowInsetsCompat.Type.systemBars());
                cutout(window, WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES);
            } else {
                cutout(window, WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_DEFAULT);
                window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                activity.setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED);
                // The icons' own light or dark appearance (MainActivity.setLightSystemBars) is
                // the window's, and stays as it was across hiding and showing the bars.
                bars.show(WindowInsetsCompat.Type.systemBars());
                applyBrightness(window, WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE);
            }
        });
    }

    /**
     * Whether the window draws into the display cutout: the player's video runs under it in
     * landscape (`SHORT_EDGES`), while its controls keep to the insets Rust is told of. Android
     * 15 and newer always draw there at this target SDK; 9 to 14 need asking, and 8 has no
     * cutouts.
     */
    private static void cutout(Window window, int mode) {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.P) return;
        WindowManager.LayoutParams attributes = window.getAttributes();
        attributes.layoutInDisplayCutoutMode = mode;
        window.setAttributes(attributes);
    }

    /**
     * The window's brightness from 0 to 1: its own while the player set one, else the system's
     * (its 0–255 setting, which on most phones maps to the panel along a curve, so a drag
     * starting from it may jump a little). Called by Rust.
     */
    static float brightness(Activity activity) {
        float own = activity.getWindow().getAttributes().screenBrightness;
        if (own >= 0) return own;
        try {
            return Settings.System.getInt(activity.getContentResolver(), Settings.System.SCREEN_BRIGHTNESS) / 255f;
        } catch (Settings.SettingNotFoundException e) {
            return 0.5f;
        }
    }

    /** Sets the window's brightness, from 0 to 1, until the player closes. Called by Rust. */
    static void setBrightness(Activity activity, float value) {
        activity.runOnUiThread(() -> applyBrightness(activity.getWindow(), value));
    }

    private static void applyBrightness(Window window, float value) {
        WindowManager.LayoutParams attributes = window.getAttributes();
        attributes.screenBrightness = value;
        window.setAttributes(attributes);
    }

    /** Puts it at this rectangle of the window, in pixels. Called by Rust. */
    static void place(int left, int top, int width, int height) {
        if (activity == null) return;
        activity.runOnUiThread(() -> {
            place = new int[] {left, top, Math.max(width, 1), Math.max(height, 1)};
            relayout();
        });
    }

    /** The video's width over its height, or 0 to fill the place. Called by Media3Bridge. */
    static void setAspect(float ratio) {
        if (activity == null) return;
        activity.runOnUiThread(() -> {
            aspect = ratio;
            relayout();
        });
    }

    private static void relayout() {
        if (shown != null) shown.view.setLayoutParams(layout());
    }

    /** The place, or the largest rectangle of the video's aspect centred in it. */
    private static FrameLayout.LayoutParams layout() {
        int left = place[0], top = place[1], width = place[2], height = place[3];
        if (aspect > 0) {
            if (width > height * aspect) {
                int fitted = Math.max(Math.round(height * aspect), 1);
                left += (width - fitted) / 2;
                width = fitted;
            } else {
                int fitted = Math.max(Math.round(width / aspect), 1);
                top += (height - fitted) / 2;
                height = fitted;
            }
        }
        FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(width, height, Gravity.TOP | Gravity.START);
        params.leftMargin = left;
        params.topMargin = top;
        return params;
    }

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        nativeSurface(holder.getSurface());
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {}

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        nativeSurface(null);
    }

    /** The surface exists, or (null) is going away: the player stops drawing before this returns. */
    private static native void nativeSurface(Surface surface);
}
