package fr.s1pierro.flow;

import android.content.Context;

import java.io.File;
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
