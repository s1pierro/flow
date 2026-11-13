package fr.s1pierro.flow;

import android.content.Context;
import android.os.Build;
import android.os.VibrationEffect;
import android.os.Vibrator;
import android.util.DisplayMetrics;

/**
 * Le retour au toucher de la télécommande : la vibration la plus courte que l'appareil
 * sache faire (une « strie » sous le doigt), et la densité réelle de l'écran pour
 * compter en millimètres. Rust appelle ces méthodes par JNI (acces::vibreur).
 */
public class Vibreur {
    private static Vibrator vibreur;
    /** 0 : primitive TICK (Android 11) ; 1 : effet TICK (Android 10) ; 2 : impulsion. */
    private static int maniere = -1;

    /** Une strie, de force arg (0 à 1). Rend "" si tout va bien, sinon l'erreur. */
    public static String tic(Context c, String arg) {
        try {
            if (vibreur == null) {
                vibreur = (Vibrator) c.getApplicationContext().getSystemService(Context.VIBRATOR_SERVICE);
            }
            if (vibreur == null || !vibreur.hasVibrator()) return "pas de vibreur";
            float force = 1f;
            try { force = Math.max(0.05f, Math.min(1f, Float.parseFloat(arg))); } catch (Throwable e) { }
            if (maniere < 0) {
                if (Build.VERSION.SDK_INT >= 30 && vibreur.areAllPrimitivesSupported(VibrationEffect.Composition.PRIMITIVE_TICK)) maniere = 0;
                else if (Build.VERSION.SDK_INT >= 29) maniere = 1;
                else maniere = 2;
            }
            if (maniere == 0) {
                vibreur.vibrate(VibrationEffect.startComposition().addPrimitive(VibrationEffect.Composition.PRIMITIVE_TICK, force).compose());
            } else if (maniere == 1) {
                vibreur.vibrate(VibrationEffect.createPredefined(VibrationEffect.EFFECT_TICK));
            } else {
                vibreur.vibrate(VibrationEffect.createOneShot(4, Math.max(1, (int) (255 * force))));
            }
            return "";
        } catch (Throwable e) {
            String m = e.getMessage();
            return e.getClass().getSimpleName() + (m == null ? "" : " : " + m);
        }
    }

    /** La manière retenue (« primitive », « effet », « impulsion »), après un premier tic. */
    public static String maniere(Context c, String arg) {
        return maniere == 0 ? "primitive" : maniere == 1 ? "effet" : maniere == 2 ? "impulsion" : "";
    }

    /** Les pixels par pouce de l'écran, en x et en y (« 401.0 403.5 »). */
    public static String dpi(Context c, String arg) {
        DisplayMetrics m = c.getResources().getDisplayMetrics();
        return m.xdpi + " " + m.ydpi;
    }
}
