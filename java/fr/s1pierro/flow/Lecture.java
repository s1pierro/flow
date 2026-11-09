package fr.s1pierro.flow;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.graphics.drawable.Icon;
import android.media.MediaMetadata;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.net.wifi.WifiManager;
import android.os.IBinder;
import android.os.PowerManager;

/**
 * Service de lecture au premier plan : il garde l'application en vie quand on la
 * quitte (sans lui, Android la gèle) et montre la lecture dans les notifications,
 * avec ses commandes, par une session média.
 *
 * Rust ne fait que lui envoyer des intentions (état et titre) ; les commandes de la
 * notification reviennent à Rust par {@link #commande}.
 */
public class Lecture extends Service {
    // L'état envoyé par Rust
    static final int ARRET = 0, CONNEXION = 1, LECTURE = 2, PAUSE = 3, ERREUR = 4;
    // Les commandes renvoyées à Rust
    static final int JOUER = 1, SUSPENDRE = 2, FERMER = 3;

    private static final String CANAL = "lecture";
    private static final int NOTIFICATION = 1;

    static {
        // Déjà chargée par NativeActivity : on retrouve la même bibliothèque
        System.loadLibrary("flow");
    }

    /** Implémentée en Rust (radio::commande). */
    static native void commande(int c);

    private MediaSession session;
    private PowerManager.WakeLock eveil;
    private WifiManager.WifiLock wifi;

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationManager nm = getSystemService(NotificationManager.class);
        NotificationChannel c = new NotificationChannel(CANAL, "Lecture", NotificationManager.IMPORTANCE_LOW);
        c.setShowBadge(false);
        nm.createNotificationChannel(c);

        session = new MediaSession(this, "Flow");
        session.setCallback(new MediaSession.Callback() {
            @Override public void onPlay() { commande(JOUER); }
            @Override public void onPause() { commande(SUSPENDRE); }
            @Override public void onStop() { commande(FERMER); }
        });
        session.setActive(true);

        eveil = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "flow:lecture");
        eveil.setReferenceCounted(false);
        wifi = ((WifiManager) getApplicationContext().getSystemService(Context.WIFI_SERVICE)).createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "flow:lecture");
        wifi.setReferenceCounted(false);
    }

    @Override
    public int onStartCommand(Intent i, int flags, int id) {
        String action = i != null ? i.getAction() : null;
        // Les boutons de la notification
        if ("jouer".equals(action)) { commande(JOUER); return START_NOT_STICKY; }
        if ("suspendre".equals(action)) { commande(SUSPENDRE); return START_NOT_STICKY; }
        if ("fermer".equals(action)) { commande(FERMER); return START_NOT_STICKY; }

        int etat = i != null ? i.getIntExtra("etat", ARRET) : ARRET;
        String titre = i != null ? i.getStringExtra("titre") : null;
        String texte = i != null ? i.getStringExtra("texte") : null;
        if (titre == null) titre = "Flow";
        if (texte == null) texte = "";

        // Lancé par startForegroundService : il faut passer au premier plan, même pour s'arrêter
        startForeground(NOTIFICATION, notification(etat, titre, texte));
        if (etat == ARRET) {
            eveil.release();
            wifi.release();
            session.setPlaybackState(new PlaybackState.Builder().setState(PlaybackState.STATE_STOPPED, 0, 0).build());
            stopForeground(true);
            stopSelf();
            return START_NOT_STICKY;
        }
        if (etat == CONNEXION || etat == LECTURE) {
            eveil.acquire();
            wifi.acquire();
        } else {
            eveil.release();
            wifi.release();
        }
        session.setMetadata(new MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, titre)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, texte)
                .build());
        int s = etat == LECTURE ? PlaybackState.STATE_PLAYING
              : etat == CONNEXION ? PlaybackState.STATE_BUFFERING
              : etat == ERREUR ? PlaybackState.STATE_ERROR
              : PlaybackState.STATE_PAUSED;
        PlaybackState.Builder p = new PlaybackState.Builder()
                .setActions(PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE | PlaybackState.ACTION_PLAY_PAUSE | PlaybackState.ACTION_STOP)
                .setState(s, PlaybackState.PLAYBACK_POSITION_UNKNOWN, 1f);
        if (etat == ERREUR) p.setErrorMessage(texte);
        session.setPlaybackState(p.build());
        // En pause, la notification peut être balayée
        if (etat == PAUSE || etat == ERREUR) stopForeground(false);
        getSystemService(NotificationManager.class).notify(NOTIFICATION, notification(etat, titre, texte));
        return START_NOT_STICKY;
    }

    private PendingIntent action(String a) {
        Intent i = new Intent(this, Lecture.class).setAction(a);
        return PendingIntent.getForegroundService(this, a.hashCode(), i, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    private Notification notification(int etat, String titre, String texte) {
        boolean joue = etat == LECTURE || etat == CONNEXION;
        Notification.Action bascule = joue
                ? new Notification.Action.Builder(Icon.createWithResource(this, R.drawable.ic_pause), "Pause", action("suspendre")).build()
                : new Notification.Action.Builder(Icon.createWithResource(this, R.drawable.ic_lecture), "Lecture", action("jouer")).build();
        Notification.Action fermer = new Notification.Action.Builder(Icon.createWithResource(this, R.drawable.ic_stop), "Fermer", action("fermer")).build();
        Intent ouvrir = getPackageManager().getLaunchIntentForPackage(getPackageName());
        return new Notification.Builder(this, CANAL)
                .setSmallIcon(R.drawable.ic_radio)
                .setContentTitle(titre)
                .setContentText(texte)
                .setOngoing(joue)
                .setShowWhen(false)
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .setCategory(Notification.CATEGORY_TRANSPORT)
                .setContentIntent(ouvrir != null ? PendingIntent.getActivity(this, 0, ouvrir, PendingIntent.FLAG_IMMUTABLE) : null)
                .setDeleteIntent(action("fermer"))
                .addAction(bascule)
                .addAction(fermer)
                .setStyle(new Notification.MediaStyle().setMediaSession(session.getSessionToken()).setShowActionsInCompactView(0, 1))
                .build();
    }

    @Override
    public void onDestroy() {
        eveil.release();
        wifi.release();
        session.release();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent i) {
        return null;
    }
}
