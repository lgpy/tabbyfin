package io.github.lgpy.tabbyfin;

import android.content.Context;
import android.graphics.PixelFormat;
import android.os.Bundle;
import android.view.View;
import android.view.inputmethod.InputMethodManager;
import androidx.activity.OnBackPressedCallback;
import androidx.core.graphics.Insets;
import androidx.core.view.WindowInsetsCompat;
import com.google.androidgamesdk.GameActivity;
import com.google.androidgamesdk.gametextinput.State;

/**
 * The app's one activity. All of the app is Rust (`crates/app`), started from `android_main`;
 * this only passes on what winit 0.30 drops (`crates/app/src/android.rs`).
 */
public class MainActivity extends GameActivity {
    static {
        System.loadLibrary("tabbyfin");
    }

    /** The insets in pixels: the system bars and cutouts, and the soft keyboard. */
    private static native void nativeInsets(int left, int top, int right, int bottom, int keyboard);

    /** The soft keyboard's text changed; the app reads it on its next frame. */
    private static native void nativeTextChanged();

    /** The soft keyboard's action key (Done, Go, Next…) was pressed. */
    private static native void nativeEditorAction();

    /** Whether Back has somewhere to go in the app: a dialog or menu, the player, a page. */
    private static native boolean nativeGoesBack();

    /** Back was pressed, and the app takes it. */
    private static native void nativeBack();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // The app's surface is translucent and over the video's (VideoSurface): the video shows
        // wherever the app draws nothing. A media overlay sits above other SurfaceViews and
        // below the window. Set before the view is attached, which is after onCreate.
        mSurfaceView.getHolder().setFormat(PixelFormat.TRANSLUCENT);
        mSurfaceView.setZOrderMediaOverlay(true);
        // Back goes back in the app while there's somewhere to go, then puts the app in the
        // background instead of closing it (see onDestroy).
        getOnBackPressedDispatcher().addCallback(this, new OnBackPressedCallback(true) {
            @Override
            public void handleOnBackPressed() {
                if (nativeGoesBack()) {
                    nativeBack();
                } else {
                    moveTaskToBack(true);
                }
            }
        });
    }

    /**
     * The Rust app runs once per process: eframe can't start its event loop twice, and winit
     * doesn't end it when the activity goes, so a new activity in this process would never draw.
     * The process ends with the activity instead; the config and the session are saved as they
     * change, so the next launch starts afresh where this one was.
     */
    @Override
    protected void onDestroy() {
        System.exit(0);
    }

    @Override
    public WindowInsetsCompat onApplyWindowInsets(View view, WindowInsetsCompat insets) {
        WindowInsetsCompat result = super.onApplyWindowInsets(view, insets);
        Insets bars = insets.getInsets(
                WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout());
        Insets keyboard = insets.getInsets(WindowInsetsCompat.Type.ime());
        nativeInsets(bars.left, bars.top, bars.right, bars.bottom, keyboard.bottom);
        return result;
    }

    /**
     * Called through android-activity's `set_ime_editor_info` when the focused field's kind
     * changes (a password field). The keyboard reads the new kind only when its input restarts.
     */
    @Override
    public void setImeEditorInfoFields(int inputType, int actionId, int imeOptions) {
        super.setImeEditorInfoFields(inputType, actionId, imeOptions);
        runOnUiThread(() -> {
            InputMethodManager keyboard = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
            keyboard.restartInput(mSurfaceView);
        });
    }

    @Override
    public void onEditorAction(int action) {
        super.onEditorAction(action);
        nativeEditorAction();
    }

    @Override
    public void stateChanged(State newState, boolean dismissed) {
        super.stateChanged(newState, dismissed);
        nativeTextChanged();
    }
}
