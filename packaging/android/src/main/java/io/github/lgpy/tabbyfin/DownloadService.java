package io.github.lgpy.tabbyfin;

import android.Manifest;
import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.os.PowerManager;
import android.util.Log;
import androidx.core.app.NotificationCompat;
import androidx.core.app.ServiceCompat;
import androidx.core.content.ContextCompat;

/**
 * Keeps the process alive while the Rust side downloads files on its own threads, with the
 * downloads' progress in a notification. Nothing is downloaded here: Rust does that, and calls in
 * from a worker thread with {@link #show} as it goes.
 *
 * The service is started once and updated after that, since starting a foreground service is
 * refused from the background on Android 12+; while it runs, its notification is only a
 * NotificationManager.notify away.
 */
public final class DownloadService extends Service {
    private static final String TAG = "tabbyfin";
    /** One notification at a time: the downloads that run are all this service's. */
    private static final int ID = 1;
    private static final String CHANNEL = "downloads";
    private static final String EXTRA_TITLE = "title", EXTRA_TEXT = "text", EXTRA_PROGRESS = "progress";

    /** The service while it runs: {@link #show} updates it rather than starting it again. */
    private static volatile DownloadService running;
    /**
     * Android 13+ shows no notification without POST_NOTIFICATIONS; the permission is asked for
     * once per process, so a download started from a fresh launch asks and later ones don't.
     */
    private static volatile boolean askedNotifications;
    /**
     * Started and not yet in the foreground: a {@link #hide} now waits for it to get there,
     * since a service started for the foreground that stops before calling startForeground
     * takes the app down on older Android versions (a download over in milliseconds).
     */
    private static volatile boolean starting;
    /** {@link #hide} came while it was {@link #starting}. */
    private static volatile boolean hideWhenStarted;

    private NotificationManager notifications;
    private PowerManager.WakeLock wakeLock;
    /** The service is in the foreground; a later start only updates the notification. */
    private boolean started;

    /**
     * Shows the downloads' progress: the title and text of the notification, and `progress` from
     * 0 to 1000, or -1 for a bar of its own while the size isn't known. Called by Rust, on any
     * thread, up to about once a second.
     */
    static void show(Context context, String title, String text, int progress) {
        askForNotifications(context);
        hideWhenStarted = false;
        DownloadService service = running;
        if (service != null) {
            service.update(title, text, progress);
            return;
        }
        starting = true;
        Context app = context.getApplicationContext();
        Intent intent = new Intent(app, DownloadService.class)
                .putExtra(EXTRA_TITLE, title)
                .putExtra(EXTRA_TEXT, text)
                .putExtra(EXTRA_PROGRESS, progress);
        try {
            ContextCompat.startForegroundService(app, intent);
        } catch (Exception e) {
            // ForegroundServiceStartNotAllowedException, say: the app is in the background. The
            // downloads carry on, just without the notification.
            starting = false;
            Log.w(TAG, "downloads: no foreground service", e);
        }
    }

    /**
     * The downloads are over: stops the service, and its notification goes with it. Safe when it
     * isn't running. Called by Rust, on any thread.
     */
    static void hide(Context context) {
        if (starting && running == null) {
            hideWhenStarted = true;
            return;
        }
        try {
            Context app = context.getApplicationContext();
            app.stopService(new Intent(app, DownloadService.class));
        } catch (Exception e) {
            Log.w(TAG, "downloads: can't stop the service", e);
        }
    }

    /**
     * Asks for POST_NOTIFICATIONS the first time a download needs the notification, rather than at
     * launch: nothing before downloads needs it.
     */
    private static void askForNotifications(Context context) {
        if (askedNotifications || Build.VERSION.SDK_INT < 33) {
            askedNotifications = true;
            return;
        }
        // No activity to ask with: a later show() from one is the chance.
        if (!(context instanceof Activity activity)) return;
        askedNotifications = true;
        if (activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                == PackageManager.PERMISSION_GRANTED) {
            return;
        }
        // The downloads come in from a worker thread; the prompt is the UI thread's.
        activity.runOnUiThread(() ->
                activity.requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS}, 0));
    }

    @Override
    public void onCreate() {
        super.onCreate();
        notifications = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        // minSdk is 26, so there's always a channel; creating it again leaves the user's own
        // choices (its importance, whether it's silenced) as they are.
        notifications.createNotificationChannel(
                new NotificationChannel(CHANNEL, "Downloads", NotificationManager.IMPORTANCE_LOW));
        // The transfers go on with the screen off. WAKE_LOCK is in the manifest, and the lock
        // goes in onDestroy.
        PowerManager power = (PowerManager) getSystemService(POWER_SERVICE);
        wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "tabbyfin:downloads");
        wakeLock.acquire();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String title = intent == null ? null : intent.getStringExtra(EXTRA_TITLE);
        String text = intent == null ? null : intent.getStringExtra(EXTRA_TEXT);
        int progress = intent == null ? -1 : intent.getIntExtra(EXTRA_PROGRESS, -1);
        if (started) {
            update(title, text, progress);
        } else {
            try {
                // The type only reaches the system on API 29+; ServiceCompat drops it before that.
                ServiceCompat.startForeground(this, ID, notification(title, text, progress),
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
                started = true;
                running = this;
            } catch (Exception e) {
                // ForegroundServiceStartNotAllowedException: the system refuses it from the
                // background. Stop rather than crash; the next show() tries again.
                Log.w(TAG, "downloads: no foreground service", e);
                stopSelf();
            }
            starting = false;
            if (started && hideWhenStarted) {
                hideWhenStarted = false;
                stopSelf();
            }
        }
        // Not sticky: what's being kept alive is Rust's threads, which the process's death takes
        // with it, so a restart would have nothing to do.
        return START_NOT_STICKY;
    }

    /** The running service's notification, as {@link #show} asks for it. */
    private void update(String title, String text, int progress) {
        try {
            notifications.notify(ID, notification(title, text, progress));
        } catch (Exception e) {
            Log.w(TAG, "downloads: no notification", e);
        }
    }

    private Notification notification(String title, String text, int progress) {
        // The activity is singleTask, so this brings the running app back instead of starting a
        // second one.
        PendingIntent open = PendingIntent.getActivity(this, 0,
                new Intent(this, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new NotificationCompat.Builder(this, CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title)
                .setContentText(text)
                .setProgress(1000, Math.max(progress, 0), progress < 0)
                // The downloads are the app's own work: the notification stays, and shows the
                // moment the service starts.
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setSilent(true)
                .setCategory(NotificationCompat.CATEGORY_PROGRESS)
                .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
                .setContentIntent(open)
                .build();
    }

    /**
     * API 35's limit on how long a dataSync service may run: stop rather than be killed, which
     * stops the downloads with them.
     */
    @Override
    public void onTimeout(int startId, int fgsType) {
        stopSelf();
    }

    @Override
    public void onDestroy() {
        running = null;
        started = false;
        if (wakeLock != null && wakeLock.isHeld()) wakeLock.release();
        wakeLock = null;
        // The notification goes with the service.
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
