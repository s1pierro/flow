package fr.s1pierro.flow;

import android.content.Context;
import android.net.ConnectivityManager;
import android.net.LinkAddress;
import android.net.LinkProperties;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.wifi.WifiManager;
import android.os.Build;

import java.io.File;
import java.net.Inet4Address;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;

/**
 * Le réseau de Flow, pour ce que Rust télécharge lui-même (la génération des lieux
 * réels : relief de l'IGN, OpenStreetMap). Rust appelle {@link #telecharger} par JNI
 * (acces::installation), depuis ses propres threads : la méthode bloque jusqu'à la fin.
 */
public class Reseau {
    private static WifiManager.WifiLock wifi;
    private static WifiManager.MulticastLock diffusion;

    /**
     * « 1 » : garde le Wi-Fi éveillé en faible latence (sans l'économie d'énergie qui
     * retarde les paquets) et laisse passer les diffusions (la découverte de la
     * télécommande) ; « 0 » : rend les deux. Rend "" si tout va bien, sinon l'erreur.
     */
    public static synchronized String verrous(Context c, String arg) {
        try {
            // Les sockets de la télécommande passent par le Wi-Fi, même quand Android
            // garde les données mobiles par défaut (un Wi-Fi sans internet)
            ConnectivityManager cm = (ConnectivityManager) c.getApplicationContext().getSystemService(Context.CONNECTIVITY_SERVICE);
            if (cm != null) cm.bindProcessToNetwork("1".equals(arg) ? reseauWifi(cm) : null);
            WifiManager w = (WifiManager) c.getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            if (w == null) return "pas de Wi-Fi";
            if ("1".equals(arg)) {
                if (wifi == null) {
                    int mode = Build.VERSION.SDK_INT >= 29 ? WifiManager.WIFI_MODE_FULL_LOW_LATENCY : WifiManager.WIFI_MODE_FULL_HIGH_PERF;
                    wifi = w.createWifiLock(mode, "flow-telecommande");
                    wifi.setReferenceCounted(false);
                    wifi.acquire();
                }
                if (diffusion == null) {
                    diffusion = w.createMulticastLock("flow-telecommande");
                    diffusion.setReferenceCounted(false);
                    diffusion.acquire();
                }
            } else {
                if (wifi != null) { wifi.release(); wifi = null; }
                if (diffusion != null) { diffusion.release(); diffusion = null; }
            }
            return "";
        } catch (Throwable e) {
            String m = e.getMessage();
            return e.getClass().getSimpleName() + (m == null ? "" : " : " + m);
        }
    }

    private static android.os.PowerManager.WakeLock eveil;
    private static WifiManager.WifiLock eveilWifi;

    /**
     * « 1 » : garde le processeur éveillé (écran éteint) et le Wi-Fi à pleine vitesse,
     * pour un long travail (génération d'une carte, renfort) ; « 0 » : les rend. Rend ""
     * si tout va bien, sinon l'erreur.
     */
    public static synchronized String eveil(Context c, String arg) {
        try {
            if ("1".equals(arg)) {
                if (eveil == null) {
                    android.os.PowerManager p = (android.os.PowerManager) c.getApplicationContext().getSystemService(Context.POWER_SERVICE);
                    eveil = p.newWakeLock(android.os.PowerManager.PARTIAL_WAKE_LOCK, "flow:carte");
                    eveil.setReferenceCounted(false);
                    eveil.acquire();
                }
                WifiManager w = (WifiManager) c.getApplicationContext().getSystemService(Context.WIFI_SERVICE);
                if (eveilWifi == null && w != null) {
                    eveilWifi = w.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "flow-carte");
                    eveilWifi.setReferenceCounted(false);
                    eveilWifi.acquire();
                }
            } else {
                if (eveil != null) { eveil.release(); eveil = null; }
                if (eveilWifi != null) { eveilWifi.release(); eveilWifi = null; }
            }
            return "";
        } catch (Throwable e) {
            String m = e.getMessage();
            return e.getClass().getSimpleName() + (m == null ? "" : " : " + m);
        }
    }

    private static Network reseauWifi(ConnectivityManager cm) {
        for (Network n : cm.getAllNetworks()) {
            NetworkCapabilities k = cm.getNetworkCapabilities(n);
            if (k != null && k.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) return n;
        }
        return null;
    }

    /**
     * L'adresse IPv4 de cet appareil sur le Wi-Fi et la longueur de son préfixe
     * (« 192.168.1.20/24 »), ou "" sans Wi-Fi.
     */
    public static String wifi(Context c, String arg) {
        try {
            ConnectivityManager cm = (ConnectivityManager) c.getApplicationContext().getSystemService(Context.CONNECTIVITY_SERVICE);
            if (cm == null) return "";
            Network n = reseauWifi(cm);
            if (n == null) return "";
            LinkProperties lp = cm.getLinkProperties(n);
            if (lp == null) return "";
            for (LinkAddress a : lp.getLinkAddresses()) {
                if (a.getAddress() instanceof Inet4Address) return a.getAddress().getHostAddress() + "/" + a.getPrefixLength();
            }
            return "";
        } catch (Throwable e) {
            return "";
        }
    }

    /**
     * Le serveur de `url` répond-il vite ? Un GET qui n'attend que dix secondes la
     * connexion et l'en-tête : "" s'il a répondu (quoi qu'il dise), l'erreur sinon.
     */
    public static String sonder(Context c, String url) {
        HttpURLConnection h = null;
        try {
            h = (HttpURLConnection) new URL(url).openConnection();
            h.setConnectTimeout(10000);
            h.setReadTimeout(10000);
            h.setRequestProperty("User-Agent", "flow-lieux/1.0");
            h.getResponseCode();
            return "";
        } catch (Throwable e) {
            String m = e.getMessage();
            return e.getClass().getSimpleName() + (m == null ? "" : " : " + m);
        } finally {
            if (h != null) h.disconnect();
        }
    }

    /**
     * « url\nchemin[\nformulaire] » : télécharge l'adresse dans le fichier (d'abord à
     * côté, puis renommé : il est entier ou absent), par GET, ou par POST du formulaire
     * déjà encodé. Rend "" si tout va bien, sinon l'erreur.
     */
    public static String telecharger(Context c, String arg) {
        String[] a = arg.split("\n", 3);
        if (a.length < 2) return "demande incomplète";
        File f = new File(a[1]);
        File tmp = new File(a[1] + ".part");
        HttpURLConnection h = null;
        try {
            h = (HttpURLConnection) new URL(a[0]).openConnection();
            h.setConnectTimeout(30000);
            h.setReadTimeout(300000);
            h.setInstanceFollowRedirects(true);
            h.setRequestProperty("User-Agent", "flow-lieux/1.0");
            if (a.length > 2) {
                byte[] corps = a[2].getBytes("UTF-8");
                h.setRequestMethod("POST");
                h.setDoOutput(true);
                h.setRequestProperty("Content-Type", "application/x-www-form-urlencoded");
                h.setFixedLengthStreamingMode(corps.length);
                OutputStream o = h.getOutputStream();
                o.write(corps);
                o.close();
            }
            int code = h.getResponseCode();
            if (code < 200 || code >= 300) return "HTTP " + code;
            File d = f.getParentFile();
            if (d != null) d.mkdirs();
            InputStream in = h.getInputStream();
            OutputStream out = new FileOutputStream(tmp);
            byte[] b = new byte[1 << 16];
            int n;
            while ((n = in.read(b)) > 0) out.write(b, 0, n);
            out.close();
            in.close();
            if (f.exists()) f.delete();
            if (!tmp.renameTo(f)) return "renommage impossible : " + f;
            return "";
        } catch (Throwable e) {
            tmp.delete();
            String m = e.getMessage();
            return e.getClass().getSimpleName() + (m == null ? "" : " : " + m);
        } finally {
            if (h != null) h.disconnect();
        }
    }
}
