package io.github.lgpy.tabbyfin;

import android.content.Context;
import android.graphics.Bitmap;
import android.net.Uri;
import android.os.Handler;
import android.os.HandlerThread;
import android.util.Log;
import android.view.Surface;
import androidx.media3.common.AudioAttributes;
import androidx.media3.common.C;
import androidx.media3.common.MediaItem;
import androidx.media3.common.MimeTypes;
import androidx.media3.common.PlaybackException;
import androidx.media3.common.Player;
import androidx.media3.common.TrackSelectionOverride;
import androidx.media3.common.Tracks;
import androidx.media3.common.VideoSize;
import androidx.media3.common.text.Cue;
import androidx.media3.common.text.CueGroup;
import androidx.media3.datasource.DefaultDataSource;
import androidx.media3.datasource.DefaultHttpDataSource;
import androidx.media3.exoplayer.DefaultRenderersFactory;
import androidx.media3.exoplayer.ExoPlaybackException;
import androidx.media3.exoplayer.ExoPlayer;
import androidx.media3.exoplayer.ForwardingRenderer;
import androidx.media3.exoplayer.Renderer;
import androidx.media3.exoplayer.audio.AudioRendererEventListener;
import androidx.media3.exoplayer.metadata.MetadataOutput;
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory;
import androidx.media3.exoplayer.text.TextOutput;
import androidx.media3.exoplayer.trackselection.DefaultTrackSelector;
import androidx.media3.exoplayer.video.VideoRendererEventListener;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.function.LongSupplier;
import org.json.JSONArray;
import org.json.JSONObject;

/**
 * Media3's ExoPlayer behind the Rust `Player` API (`crates/player/src/media3.rs`), Android's
 * default player. Rust sends commands as JSON ({@link #command}) and the video surface
 * ({@link #surface}); events and subtitle cues go back through the native methods. ExoPlayer runs
 * on its own looper thread and draws into the video surface (VideoSurface); the cues are drawn by
 * the app.
 *
 * Track ids are mpv's, as the app sends them: the Nth audio or text track of the file, from 1,
 * with the external subtitles after the embedded ones; 0 is the default choice, -1 none.
 */
public final class Media3Bridge {
    // Event kinds, matched by media3.rs.
    static final int FILE_LOADED = 1, POSITION = 2, DURATION = 3, PAUSED = 4, VOLUME = 5, MUTED = 6,
            SEEKED = 7, RESTARTED = 8, END_EOF = 9;

    private static final String TAG = "tabbyfin-media3";

    private static native void nativeEvent(long handle, int kind, double value);

    private static native void nativeError(long handle, String message);

    /** The cues as JSON (see media3.rs), with an int[] of ARGB pixels per image cue. */
    private static native void nativeCues(long handle, String json, Object[] images);

    private final long handle;
    private final HandlerThread thread = new HandlerThread("media3");
    private final Handler handler;
    private final Context context;
    private ExoPlayer player;
    private DefaultTrackSelector selector;
    private DefaultHttpDataSource.Factory http;
    private boolean released;

    private String url;
    private final List<MediaItem.SubtitleConfiguration> subtitles = new ArrayList<>();
    /** The external subtitles in the item ExoPlayer has now (the rest wait for a reload). */
    private int subtitlesLoaded;
    /** The file is loaded (FILE_LOADED went out). */
    private boolean loaded;
    /** Waiting to be ready again after a seek, or after a reload for new subtitles. */
    private boolean seeking, reloading;
    /** The tracks were picked for what ExoPlayer has now. */
    private boolean tracksApplied;
    private int aid, sid;

    private float volume = 1f;
    private boolean muted;
    /** mpv's delays: positive plays the audio, or shows the subtitles, later. In µs. */
    private volatile long audioDelayUs, subDelayUs;

    /** Called by Rust, on any thread. */
    public static Media3Bridge create(Context context, long handle) {
        return new Media3Bridge(context, handle);
    }

    private Media3Bridge(Context context, long handle) {
        this.context = context.getApplicationContext();
        this.handle = handle;
        thread.start();
        handler = new Handler(thread.getLooper());
        handler.post(this::start);
    }

    /** Called by Rust, on any thread: a command as JSON, run on the player's thread. */
    public void command(String json) {
        handler.post(() -> {
            if (released) return;
            try {
                run(new JSONObject(json));
            } catch (Exception e) {
                Log.w(TAG, "command " + json, e);
            }
        });
    }

    /**
     * The video surface came (or went: null). Called by Rust on the UI thread; when it goes,
     * ExoPlayer lets go of it before this returns.
     */
    public void surface(Surface surface) {
        if (surface != null) {
            handler.post(() -> {
                if (!released) player.setVideoSurface(surface);
            });
            return;
        }
        CountDownLatch done = new CountDownLatch(1);
        boolean posted = handler.post(() -> {
            if (!released) player.clearVideoSurface();
            done.countDown();
        });
        try {
            if (posted) done.await(500, TimeUnit.MILLISECONDS);
        } catch (InterruptedException ignored) {
            Thread.currentThread().interrupt();
        }
    }

    private void start() {
        http = new DefaultHttpDataSource.Factory().setAllowCrossProtocolRedirects(true);
        selector = new DefaultTrackSelector(context);
        selector.setParameters(selector.buildUponParameters().setTunnelingEnabled(true));
        player = new ExoPlayer.Builder(context, new Renderers(context))
                .setTrackSelector(selector)
                .setLooper(thread.getLooper())
                // The server's files come over HTTP, with the headers the "load" op sets; a
                // downloaded one plays from file://, its external subtitles too, and HTTP
                // sources can't open those, so DefaultDataSource picks by the URL's scheme.
                .setMediaSourceFactory(new DefaultMediaSourceFactory(new DefaultDataSource.Factory(context, http)))
                // Pauses when another app takes the audio, and when headphones are unplugged.
                .setAudioAttributes(new AudioAttributes.Builder()
                        .setUsage(C.USAGE_MEDIA)
                        .setContentType(C.AUDIO_CONTENT_TYPE_MOVIE)
                        .build(), true)
                .setHandleAudioBecomingNoisy(true)
                .build();
        player.addListener(listener);
        player.setPlayWhenReady(true);
        handler.postDelayed(this::poll, 1000);
    }

    private void run(JSONObject cmd) throws Exception {
        switch (cmd.getString("op")) {
            case "load": {
                Map<String, String> headers = new HashMap<>();
                JSONArray lines = cmd.optJSONArray("headers");
                for (int i = 0; lines != null && i < lines.length(); i++) {
                    String[] kv = lines.getString(i).split(":\\s*", 2);
                    if (kv.length == 2) headers.put(kv[0], kv[1]);
                }
                http.setDefaultRequestProperties(headers);
                url = cmd.getString("url");
                subtitles.clear();
                loaded = false;
                prepare((long) (cmd.optDouble("start", 0) * 1000));
                break;
            }
            case "pause":
                player.setPlayWhenReady(!cmd.getBoolean("value"));
                break;
            case "toggle_pause":
                player.setPlayWhenReady(!player.getPlayWhenReady());
                break;
            case "seek": {
                long target = (long) (cmd.getDouble("secs") * 1000);
                if (cmd.optBoolean("relative")) target += player.getCurrentPosition();
                seeking = true;
                player.seekTo(Math.max(0, target));
                nativeEvent(handle, SEEKED, 0);
                break;
            }
            case "volume":
                volume = (float) (cmd.getDouble("value") / 100);
                applyVolume();
                break;
            case "mute":
                muted = cmd.getBoolean("value");
                applyVolume();
                break;
            case "toggle_mute":
                muted = !muted;
                applyVolume();
                break;
            case "speed":
                player.setPlaybackSpeed((float) cmd.getDouble("value"));
                break;
            case "aid":
                aid = cmd.getInt("id");
                applyTracks();
                break;
            case "sid":
                sid = cmd.getInt("id");
                applyTracks();
                break;
            case "sub_add":
                addSubtitle(cmd);
                break;
            case "sub_delay":
                subDelayUs = (long) (cmd.getDouble("secs") * 1_000_000);
                break;
            case "audio_delay":
                audioDelayUs = (long) (cmd.getDouble("secs") * 1_000_000);
                applyTracks();
                break;
            case "release":
                released = true;
                handler.removeCallbacksAndMessages(null);
                player.release();
                thread.quitSafely();
                break;
            default:
                Log.w(TAG, "unknown op " + cmd);
        }
    }

    /** Gives ExoPlayer the file and the external subtitles so far, from `positionMs`. */
    private void prepare(long positionMs) {
        MediaItem item = new MediaItem.Builder()
                .setUri(url)
                .setSubtitleConfigurations(new ArrayList<>(subtitles))
                .build();
        subtitlesLoaded = subtitles.size();
        tracksApplied = false;
        player.setMediaItem(item, positionMs);
        player.prepare();
    }

    /**
     * An external subtitle file. ExoPlayer only takes them with the file, so the file is given
     * again from where it is, once the app has added all it adds after loading.
     */
    private void addSubtitle(JSONObject cmd) throws Exception {
        String uri = cmd.getString("url");
        MediaItem.SubtitleConfiguration.Builder config = new MediaItem.SubtitleConfiguration.Builder(Uri.parse(uri))
                .setMimeType(subtitleMime(uri));
        if (!cmd.isNull("title")) config.setLabel(cmd.getString("title"));
        if (!cmd.isNull("language")) config.setLanguage(cmd.getString("language"));
        int embedded = count(player.getCurrentTracks(), C.TRACK_TYPE_TEXT) - subtitlesLoaded;
        subtitles.add(config.build());
        if (cmd.optBoolean("select")) sid = Math.max(embedded, 0) + subtitles.size();
        handler.removeCallbacks(reload);
        handler.postDelayed(reload, 200);
    }

    private final Runnable reload = () -> {
        if (released || url == null) return;
        reloading = true;
        prepare(player.getCurrentPosition());
    };

    private static String subtitleMime(String uri) {
        String path = Uri.parse(uri).getLastPathSegment();
        String ext = path == null ? "" : path.substring(path.lastIndexOf('.') + 1).toLowerCase(Locale.ROOT);
        switch (ext) {
            case "vtt":
            case "webvtt":
                return MimeTypes.TEXT_VTT;
            case "ass":
            case "ssa":
                return MimeTypes.TEXT_SSA;
            case "ttml":
            case "xml":
                return MimeTypes.APPLICATION_TTML;
            default:
                return MimeTypes.APPLICATION_SUBRIP;
        }
    }

    /**
     * Picks the audio and text tracks asked for, once ExoPlayer knows the file's tracks. Tunneled
     * video is timed by the decoder against the audio, out of the shift's reach (Renderers), so
     * it's off while there's an audio delay.
     */
    private void applyTracks() {
        Tracks tracks = player.getCurrentTracks();
        DefaultTrackSelector.Parameters.Builder params = selector.buildUponParameters()
                .clearOverridesOfType(C.TRACK_TYPE_AUDIO)
                .clearOverridesOfType(C.TRACK_TYPE_TEXT)
                .setTrackTypeDisabled(C.TRACK_TYPE_TEXT, sid < 0)
                .setTunnelingEnabled(audioDelayUs == 0);
        Tracks.Group audio = nth(tracks, C.TRACK_TYPE_AUDIO, aid);
        if (audio != null) params.setOverrideForType(new TrackSelectionOverride(audio.getMediaTrackGroup(), 0));
        Tracks.Group text = nth(tracks, C.TRACK_TYPE_TEXT, sid);
        if (text != null) params.setOverrideForType(new TrackSelectionOverride(text.getMediaTrackGroup(), 0));
        selector.setParameters(params);
        tracksApplied = !tracks.isEmpty();
    }

    /** The `n`th group of `type` (from 1), or null. */
    private static Tracks.Group nth(Tracks tracks, int type, int n) {
        if (n < 1) return null;
        int seen = 0;
        for (Tracks.Group group : tracks.getGroups()) {
            if (group.getType() == type && ++seen == n) return group;
        }
        return null;
    }

    private static int count(Tracks tracks, int type) {
        int n = 0;
        for (Tracks.Group group : tracks.getGroups()) if (group.getType() == type) n++;
        return n;
    }

    /** mpv's volume is cubic: 50% is an eighth of the gain. ExoPlayer's is linear. */
    private void applyVolume() {
        player.setVolume(muted ? 0f : volume * volume * volume);
        nativeEvent(handle, VOLUME, volume * 100);
        nativeEvent(handle, MUTED, muted ? 1 : 0);
    }

    private void poll() {
        if (released) return;
        if (loaded && !reloading) nativeEvent(handle, POSITION, player.getCurrentPosition() / 1000.0);
        handler.postDelayed(this::poll, 1000);
    }

    /** Ready after a seek or a reload: the frames are at the new position. */
    private void restarted() {
        if (!seeking && !reloading) return;
        seeking = false;
        reloading = false;
        nativeEvent(handle, POSITION, player.getCurrentPosition() / 1000.0);
        nativeEvent(handle, RESTARTED, 0);
    }

    private final Player.Listener listener = new Player.Listener() {
        @Override
        public void onPlaybackStateChanged(int state) {
            if (state == Player.STATE_READY) {
                if (!loaded) {
                    loaded = true;
                    nativeEvent(handle, FILE_LOADED, 0);
                    nativeEvent(handle, DURATION, player.getDuration() / 1000.0);
                    nativeEvent(handle, PAUSED, player.getPlayWhenReady() ? 0 : 1);
                    applyVolume();
                    nativeEvent(handle, POSITION, player.getCurrentPosition() / 1000.0);
                    nativeEvent(handle, RESTARTED, 0);
                } else {
                    restarted();
                }
            } else if (state == Player.STATE_ENDED) {
                nativeEvent(handle, END_EOF, 0);
            }
        }

        @Override
        public void onRenderedFirstFrame() {
            // A seek within what's buffered can stay ready throughout.
            restarted();
        }

        @Override
        public void onPlayWhenReadyChanged(boolean playWhenReady, int reason) {
            nativeEvent(handle, PAUSED, playWhenReady ? 0 : 1);
        }

        @Override
        public void onPlayerError(PlaybackException error) {
            Log.w(TAG, "playback error", error);
            String message = error.getErrorCodeName();
            if (error.getCause() != null && error.getCause().getMessage() != null) {
                message += ": " + error.getCause().getMessage();
            }
            nativeError(handle, message);
        }

        @Override
        public void onTracksChanged(Tracks tracks) {
            if (!tracksApplied && !tracks.isEmpty()) applyTracks();
        }

        @Override
        public void onVideoSizeChanged(VideoSize size) {
            if (size.width > 0 && size.height > 0) {
                VideoSurface.setAspect(size.width * size.pixelWidthHeightRatio / size.height);
            }
        }

        @Override
        public void onCues(CueGroup group) {
            sendCues(group.cues);
        }
    };

    private void sendCues(List<Cue> cues) {
        try {
            JSONArray list = new JSONArray();
            List<int[]> images = new ArrayList<>();
            for (Cue cue : cues) {
                if (cue.bitmap != null) {
                    list.put(imageCue(cue, images));
                } else if (cue.text != null && cue.text.length() > 0) {
                    list.put(new JSONObject().put("text", cue.text.toString()));
                }
            }
            VideoSize size = player.getVideoSize();
            float aspect = size.height > 0 ? size.width * size.pixelWidthHeightRatio / size.height : 0;
            JSONObject json = new JSONObject().put("cues", list).put("aspect", aspect);
            nativeCues(handle, json.toString(), images.toArray());
        } catch (Exception e) {
            Log.w(TAG, "cues", e);
        }
    }

    /** A picture cue (PGS and the like), placed in fractions of the video. */
    private JSONObject imageCue(Cue cue, List<int[]> images) throws Exception {
        Bitmap bitmap = cue.bitmap;
        if (bitmap.getConfig() != Bitmap.Config.ARGB_8888) bitmap = bitmap.copy(Bitmap.Config.ARGB_8888, false);
        int w = bitmap.getWidth(), h = bitmap.getHeight();
        int[] pixels = new int[w * h];
        bitmap.getPixels(pixels, 0, w, 0, 0, w, h);
        images.add(pixels);

        VideoSize video = player.getVideoSize();
        float width = cue.size != Cue.DIMEN_UNSET ? cue.size
                : video.width > 0 ? Math.min((float) w / video.width, 1f) : 0.5f;
        // 0: the UI keeps the picture's aspect ratio.
        float height = cue.bitmapHeight != Cue.DIMEN_UNSET ? cue.bitmapHeight : 0;
        float x = cue.position != Cue.DIMEN_UNSET ? cue.position : 0.5f;
        int xAnchor = cue.position != Cue.DIMEN_UNSET ? cue.positionAnchor : Cue.ANCHOR_TYPE_MIDDLE;
        boolean fraction = cue.line != Cue.DIMEN_UNSET && cue.lineType == Cue.LINE_TYPE_FRACTION;
        float y = fraction ? cue.line : 0.95f;
        int yAnchor = fraction ? cue.lineAnchor : Cue.ANCHOR_TYPE_END;
        float left = x - anchorShift(xAnchor) * width;
        // An unknown height is placed by its start; the UI can't shift what it hasn't sized.
        float top = y - anchorShift(yAnchor) * height;
        return new JSONObject()
                .put("image", images.size() - 1)
                .put("width", w)
                .put("height", h)
                .put("rect", new JSONArray().put(left).put(top).put(width).put(height));
    }

    private static float anchorShift(int anchor) {
        if (anchor == Cue.ANCHOR_TYPE_MIDDLE) return 0.5f;
        if (anchor == Cue.ANCHOR_TYPE_END) return 1f;
        return 0f;
    }

    /**
     * Android's decoders, then Jellyfin's ffmpeg audio decoders for what it lacks (DTS, TrueHD…),
     * as Findroid has them. The video and text renderers are shifted for the delays: positive
     * audio delay shows the video earlier against the audio (which is the clock), and positive
     * subtitle delay shows the cues later.
     */
    private final class Renderers extends DefaultRenderersFactory {
        Renderers(Context context) {
            super(context);
            setExtensionRendererMode(EXTENSION_RENDERER_MODE_ON);
        }

        @Override
        public Renderer[] createRenderers(Handler eventHandler, VideoRendererEventListener video,
                AudioRendererEventListener audio, TextOutput text, MetadataOutput metadata) {
            Renderer[] renderers = super.createRenderers(eventHandler, video, audio, text, metadata);
            for (int i = 0; i < renderers.length; i++) {
                int type = renderers[i].getTrackType();
                if (type == C.TRACK_TYPE_VIDEO) {
                    renderers[i] = new Shifted(renderers[i], () -> audioDelayUs);
                } else if (type == C.TRACK_TYPE_TEXT) {
                    renderers[i] = new Shifted(renderers[i], () -> audioDelayUs - subDelayUs);
                }
            }
            return renderers;
        }
    }

    /** A renderer that renders as if the playback position were `offset` µs later. */
    private static final class Shifted extends ForwardingRenderer {
        private final LongSupplier offsetUs;

        Shifted(Renderer renderer, LongSupplier offsetUs) {
            super(renderer);
            this.offsetUs = offsetUs;
        }

        @Override
        public void render(long positionUs, long elapsedRealtimeUs) throws ExoPlaybackException {
            super.render(positionUs + offsetUs.getAsLong(), elapsedRealtimeUs);
        }
    }
}
