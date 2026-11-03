package io.github.lgpy.tabbyfin;

import android.app.Activity;
import android.view.Gravity;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.widget.FrameLayout;

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
            // The screen stays on while a player has video to show, paused or not.
            activity.getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        });
    }

    /** Takes it away; the player is told the surface is gone first. Called by Rust. */
    static void hide() {
        if (activity == null) return;
        activity.runOnUiThread(() -> {
            aspect = 0;
            activity.getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            if (shown == null) return;
            if (shown.view.getParent() instanceof ViewGroup) {
                ((ViewGroup) shown.view.getParent()).removeView(shown.view);
            }
            shown = null;
        });
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
