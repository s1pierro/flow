package fr.s1pierro.flow;

import android.app.Activity;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.view.MotionEvent;
import android.view.ViewGroup;
import android.webkit.ConsoleMessage;
import android.webkit.WebChromeClient;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import java.nio.ByteBuffer;
import java.util.HashMap;

/**
 * Le contexte Toile : une page web (HTML, CSS, JavaScript) par le moteur d'Android.
 *
 * Flow dessine tout lui-même (la NativeActivity a pris la surface de la fenêtre) : la
 * WebView est attachée à l'activité sans y être vue, et dessinée régulièrement dans
 * une image que Rust lit (pixels, un ByteBuffer direct en RGBA prémultiplié) et
 * affiche dans sa fenêtre. Les touchers de Flow lui reviennent par dispatchTouchEvent.
 *
 * Rust appelle les méthodes statiques par JNI (acces::toile) : chacune reçoit
 * l'activité et un texte (la page d'abord), rend un texte. Tout ce qui touche la
 * WebView passe au fil d'interface d'Android.
 */
public class Toile {
    static final Handler ui = new Handler(Looper.getMainLooper());
    static final HashMap<String, Page> pages = new HashMap<>();

    static class Page {
        final String id;
        WebView vue;
        int w = 1, h = 1;
        Bitmap image;
        volatile ByteBuffer pixels;
        volatile int version, iw, ih;
        volatile String url = "", titre = "", console = "";
        volatile int progres = 100;
        volatile boolean arriere, avant;
        /** Quand Flow a regardé la page pour la dernière fois, et l'a touchée. */
        volatile long vu, touche;
        boolean dessin;

        Page(String id) { this.id = id; }
    }

    private static Page page(String id) {
        synchronized (pages) { return pages.get(id); }
    }

    private static String premiere(String s) {
        int i = s.indexOf('\n');
        return i < 0 ? s : s.substring(0, i);
    }

    private static String reste(String s) {
        int i = s.indexOf('\n');
        return i < 0 ? "" : s.substring(i + 1);
    }

    /** « id\nadresse » : crée la page si besoin et y charge l'adresse. */
    public static String ouvrir(final Context c, String arg) {
        final String id = premiere(arg), url = reste(arg);
        Page p;
        synchronized (pages) {
            p = pages.get(id);
            if (p == null) { p = new Page(id); pages.put(id, p); }
        }
        final Page q = p;
        q.vu = SystemClock.uptimeMillis();
        ui.post(new Runnable() { public void run() {
            if (q.vue == null) creer(c, q);
            if (!url.isEmpty()) q.vue.loadUrl(url);
        }});
        return "";
    }

    private static void creer(Context c, final Page p) {
        WebView v = new WebView(c);
        WebSettings s = v.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setAllowFileAccess(true);
        s.setAllowFileAccessFromFileURLs(true);
        s.setAllowUniversalAccessFromFileURLs(true);
        s.setLoadWithOverviewMode(true);
        s.setUseWideViewPort(true);
        s.setBuiltInZoomControls(true);
        s.setDisplayZoomControls(false);
        v.setWebViewClient(new WebViewClient() {
            @Override public void onPageFinished(WebView w, String u) { suivre(p); }
            @Override public void doUpdateVisitedHistory(WebView w, String u, boolean r) { suivre(p); }
        });
        v.setWebChromeClient(new WebChromeClient() {
            @Override public void onProgressChanged(WebView w, int n) { p.progres = n; suivre(p); }
            @Override public void onReceivedTitle(WebView w, String t) { p.titre = t == null ? "" : t; }
            @Override public boolean onConsoleMessage(ConsoleMessage m) {
                String niveau = m.messageLevel() == ConsoleMessage.MessageLevel.ERROR ? "erreur : " : "";
                p.console = (niveau + m.message() + " (" + court(m.sourceId()) + ":" + m.lineNumber() + ")").replace('\n', ' ');
                return true;
            }
        });
        p.vue = v;
        // Attachée à l'activité (sans y être vue : la surface est à Flow), la page se
        // croit visible : minuteries et animations tournent
        if (c instanceof Activity) {
            try {
                ((Activity) c).addContentView(v, new ViewGroup.LayoutParams(p.w, p.h));
            } catch (Throwable e) { /* hors de la fenêtre : elle se dessine quand même */ }
        }
        mettre_en_page(p);
        boucler(p);
    }

    private static String court(String s) {
        if (s == null) return "";
        int i = s.lastIndexOf('/');
        return i < 0 ? s : s.substring(i + 1);
    }

    private static void suivre(Page p) {
        if (p.vue == null) return;
        String u = p.vue.getUrl();
        p.url = u == null ? "" : u;
        p.arriere = p.vue.canGoBack();
        p.avant = p.vue.canGoForward();
    }

    private static void mettre_en_page(Page p) {
        WebView v = p.vue;
        ViewGroup.LayoutParams lp = v.getLayoutParams();
        if (lp != null && (lp.width != p.w || lp.height != p.h)) {
            lp.width = p.w;
            lp.height = p.h;
            v.setLayoutParams(lp);
        }
        v.measure(android.view.View.MeasureSpec.makeMeasureSpec(p.w, android.view.View.MeasureSpec.EXACTLY),
                  android.view.View.MeasureSpec.makeMeasureSpec(p.h, android.view.View.MeasureSpec.EXACTLY));
        v.layout(0, 0, p.w, p.h);
    }

    /** Le dessin régulier : vite quand on vient de toucher ou que la page charge,
     *  lentement sinon, plus du tout quand Flow ne la regarde plus. */
    private static void boucler(final Page p) {
        ui.postDelayed(new Runnable() { public void run() {
            if (p.vue == null) return;
            long t = SystemClock.uptimeMillis();
            boolean regardee = t - p.vu < 1500;
            if (regardee) dessiner(p);
            boolean vif = t - p.touche < 3000 || p.progres < 100;
            ui.postDelayed(this, !regardee ? 250 : vif ? 33 : 100);
        }}, 50);
    }

    private static void dessiner(Page p) {
        int w = p.w, h = p.h;
        if (w < 1 || h < 1) return;
        if (p.vue.getWidth() != w || p.vue.getHeight() != h) mettre_en_page(p);
        if (p.image == null || p.image.getWidth() != w || p.image.getHeight() != h) {
            if (p.image != null) p.image.recycle();
            p.image = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888);
        }
        p.image.eraseColor(0xFFFFFFFF);
        Canvas c = new Canvas(p.image);
        c.translate(-p.vue.getScrollX(), -p.vue.getScrollY());
        p.vue.draw(c);
        ByteBuffer b = p.pixels;
        if (b == null || b.capacity() != w * h * 4) b = ByteBuffer.allocateDirect(w * h * 4);
        b.rewind();
        p.image.copyPixelsToBuffer(b);
        p.iw = w;
        p.ih = h;
        p.pixels = b;
        p.version++;
    }

    /** « id w h » : la taille de la page, en pixels. */
    public static String taille(Context c, String arg) {
        String[] a = arg.split(" ");
        final Page p = page(a[0]);
        if (p == null || a.length < 3) return "";
        final int w = Math.max(1, Integer.parseInt(a[1])), h = Math.max(1, Integer.parseInt(a[2]));
        if (w == p.w && h == p.h) return "";
        p.w = w;
        p.h = h;
        ui.post(new Runnable() { public void run() { if (p.vue != null) mettre_en_page(p); }});
        return "";
    }

    /** « id action x y » : un doigt (0 posé, 1 levé, 2 bougé, 3 annulé). */
    public static String toucher(Context c, String arg) {
        String[] a = arg.split(" ");
        final Page p = page(a[0]);
        if (p == null || a.length < 4) return "";
        final int action = Integer.parseInt(a[1]);
        final float x = Float.parseFloat(a[2]), y = Float.parseFloat(a[3]);
        p.touche = SystemClock.uptimeMillis();
        ui.post(new Runnable() { public void run() {
            if (p.vue == null) return;
            long t = SystemClock.uptimeMillis();
            MotionEvent e = MotionEvent.obtain(t, t, action, x, y, 0);
            e.setSource(android.view.InputDevice.SOURCE_TOUCHSCREEN);
            if (action == MotionEvent.ACTION_DOWN) p.vue.requestFocus();
            p.vue.dispatchTouchEvent(e);
            e.recycle();
        }});
        return "";
    }

    /** « id commande » : arriere, avant, recharger, arreter. */
    public static String naviguer(Context c, String arg) {
        String[] a = arg.split(" ");
        final Page p = page(a[0]);
        if (p == null || a.length < 2) return "";
        final String m = a[1];
        p.touche = SystemClock.uptimeMillis();
        ui.post(new Runnable() { public void run() {
            if (p.vue == null) return;
            if (m.equals("arriere")) p.vue.goBack();
            else if (m.equals("avant")) p.vue.goForward();
            else if (m.equals("recharger")) p.vue.reload();
            else if (m.equals("arreter")) p.vue.stopLoading();
            suivre(p);
        }});
        return "";
    }

    /** L'état de la page, une ligne chacun : version de l'image, largeur, hauteur,
     *  adresse, titre, avancement, arrière possible, avant possible, console. */
    public static String etat(Context c, String id) {
        Page p = page(id);
        if (p == null) return null;
        p.vu = SystemClock.uptimeMillis();
        return p.version + "\n" + p.iw + "\n" + p.ih + "\n" + p.url.replace('\n', ' ') + "\n" + p.titre.replace('\n', ' ') + "\n"
            + p.progres + "\n" + (p.arriere ? 1 : 0) + "\n" + (p.avant ? 1 : 0) + "\n" + p.console;
    }

    /** L'image de la page : un ByteBuffer direct, RGBA prémultiplié. */
    public static ByteBuffer pixels(String id) {
        Page p = page(id);
        return p == null ? null : p.pixels;
    }

    public static String fermer(Context c, String id) {
        final Page p;
        synchronized (pages) { p = pages.remove(id); }
        if (p == null) return "";
        ui.post(new Runnable() { public void run() {
            if (p.vue == null) return;
            ViewGroup g = (ViewGroup) p.vue.getParent();
            if (g != null) g.removeView(p.vue);
            p.vue.destroy();
            p.vue = null;
        }});
        return "";
    }
}
