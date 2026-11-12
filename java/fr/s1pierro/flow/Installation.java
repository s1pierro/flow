package fr.s1pierro.flow;

import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageInstaller;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.provider.Settings;

import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Les applications Android, pour le contexte Fichiers : lire un .apk, l'installer ou
 * le mettre à jour par l'installateur du système (qui demande toujours confirmation),
 * lancer une application installée.
 *
 * Rust appelle les méthodes statiques par JNI (acces::installation) : chacune prend le
 * contexte de l'application (et un texte), et rend un texte. L'installation se suit par
 * {@link #suivi} ; le résultat revient du système à ce récepteur.
 */
public class Installation extends BroadcastReceiver {
    static final int RIEN = 0, COPIE = 1, CONFIRMATION = 2, REUSSIE = 3, ECHEC = 4, ANNULEE = 5;

    static volatile int etat = RIEN;
    static volatile long fait, total;
    static volatile String message = "";

    private static long code(PackageInfo p) {
        return Build.VERSION.SDK_INT >= 28 ? p.getLongVersionCode() : p.versionCode;
    }

    private static String ligne(String s) {
        return s == null ? "" : s.replace('\n', ' ');
    }

    /**
     * Ce que dit le .apk : paquet, version, code de version, nom, puis la version et le
     * code de l'application installée (vides si elle ne l'est pas), une ligne chacun.
     * Rien si le fichier n'est pas une application lisible.
     */
    public static String lire(Context c, String chemin) {
        PackageManager pm = c.getPackageManager();
        PackageInfo a = pm.getPackageArchiveInfo(chemin, 0);
        if (a == null) return null;
        String nom = a.packageName;
        if (a.applicationInfo != null) {
            // Sans ces chemins, le nom (et ses ressources) ne se lit pas
            a.applicationInfo.sourceDir = chemin;
            a.applicationInfo.publicSourceDir = chemin;
            CharSequence l = a.applicationInfo.loadLabel(pm);
            if (l != null) nom = l.toString();
        }
        String version = "", code = "";
        try {
            PackageInfo i = pm.getPackageInfo(a.packageName, 0);
            version = i.versionName == null ? "?" : i.versionName;
            code = Long.toString(code(i));
        } catch (PackageManager.NameNotFoundException e) {
            // Pas installée
        }
        return a.packageName + "\n" + ligne(a.versionName) + "\n" + code(a) + "\n" + ligne(nom) + "\n" + ligne(version) + "\n" + code;
    }

    /** « 1 » si Flow a le droit de demander des installations. */
    public static String autorisee(Context c) {
        return c.getPackageManager().canRequestPackageInstalls() ? "1" : "0";
    }

    /** Ouvre le réglage d'Android qui autorise Flow à installer des applications. */
    public static String autoriser(Context c) {
        Intent i = new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + c.getPackageName()));
        i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        c.startActivity(i);
        return "";
    }

    /**
     * Lance l'installation du .apk : la copie vers une session de l'installateur, sur un
     * thread à part, puis la demande de confirmation. Rend un message d'erreur, ou rien.
     */
    public static String installer(final Context c, final String chemin) {
        if (etat == COPIE) return "Une installation est déjà en cours.";
        final File f = new File(chemin);
        if (!f.canRead()) return "Fichier illisible.";
        fait = 0;
        total = f.length();
        message = "";
        etat = COPIE;
        new Thread(new Runnable() {
            public void run() {
                PackageInstaller.Session s = null;
                try {
                    PackageInstaller pi = c.getPackageManager().getPackageInstaller();
                    PackageInstaller.SessionParams p = new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                    p.setSize(total);
                    int id = pi.createSession(p);
                    s = pi.openSession(id);
                    try (InputStream in = new FileInputStream(f); OutputStream out = s.openWrite("application.apk", 0, total)) {
                        byte[] b = new byte[1 << 16];
                        int n;
                        while ((n = in.read(b)) > 0) {
                            out.write(b, 0, n);
                            fait += n;
                        }
                        s.fsync(out);
                    }
                    Intent retour = new Intent(c, Installation.class);
                    // FLAG_MUTABLE (Android 12) : le système complète l'intention
                    int drapeaux = PendingIntent.FLAG_UPDATE_CURRENT | (Build.VERSION.SDK_INT >= 31 ? 0x02000000 : 0);
                    PendingIntent pe = PendingIntent.getBroadcast(c, id, retour, drapeaux);
                    etat = CONFIRMATION;
                    s.commit(pe.getIntentSender());
                    s.close();
                } catch (Exception e) {
                    if (s != null) s.abandon();
                    message = ligne(String.valueOf(e.getMessage()));
                    etat = ECHEC;
                }
            }
        }).start();
        return "";
    }

    /** Le résultat de l'installation, envoyé par le système. */
    @Override
    public void onReceive(Context c, Intent i) {
        int s = i.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
        if (s == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            // La fenêtre de confirmation du système
            Intent confirmer = (Intent) i.getParcelableExtra(Intent.EXTRA_INTENT);
            if (confirmer != null) {
                confirmer.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                c.startActivity(confirmer);
            }
            etat = CONFIRMATION;
        } else if (s == PackageInstaller.STATUS_SUCCESS) {
            etat = REUSSIE;
        } else if (s == PackageInstaller.STATUS_FAILURE_ABORTED) {
            etat = ANNULEE;
        } else {
            message = ligne(i.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE));
            etat = ECHEC;
        }
    }

    /** L'état de l'installation, les octets copiés, leur total, le message d'erreur. */
    public static String suivi(Context c) {
        return etat + "\n" + fait + "\n" + total + "\n" + message;
    }

    /** Oublie une installation terminée (avant d'en suivre une autre). */
    public static String oublier(Context c) {
        if (etat != COPIE) etat = RIEN;
        return "";
    }

    /** Lance l'application installée ; « 1 » si elle a pu l'être. */
    public static String lancer(Context c, String paquet) {
        Intent i = c.getPackageManager().getLaunchIntentForPackage(paquet);
        if (i == null) return "0";
        i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        c.startActivity(i);
        return "1";
    }
}
