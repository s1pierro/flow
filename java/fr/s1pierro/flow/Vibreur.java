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
    /** Les primitives prises en charge : TICK, CLICK (Android 11), ou null à savoir. */
    private static Boolean tick, click;

    /**
     * Une strie, de niveau arg : 0 légère (le « tick »), 1 moyenne (le « clic »), 2 forte
     * (le « clic appuyé »), 3 maximale (impulsion de 12 ms à pleine amplitude). Chaque
     * niveau prend l'effet le plus court qui l'atteint sur l'appareil. Rend "" si tout va
     * bien, sinon l'erreur.
     */
    public static String tic(Context c, String arg) {
        try {
            if (vibreur == null) {
                vibreur = (Vibrator) c.getApplicationContext().getSystemService(Context.VIBRATOR_SERVICE);
            }
            if (vibreur == null || !vibreur.hasVibrator()) return "pas de vibreur";
            int niveau = 2;
            try { niveau = Math.max(0, Math.min(3, Integer.parseInt(arg.trim()))); } catch (Throwable e) { }
            int sdk = Build.VERSION.SDK_INT;
            if (sdk >= 30 && tick == null) {
                tick = vibreur.areAllPrimitivesSupported(VibrationEffect.Composition.PRIMITIVE_TICK);
                click = vibreur.areAllPrimitivesSupported(VibrationEffect.Composition.PRIMITIVE_CLICK);
            }
            VibrationEffect e;
            if (niveau == 0 && sdk >= 30 && tick) {
                e = VibrationEffect.startComposition().addPrimitive(VibrationEffect.Composition.PRIMITIVE_TICK, 1f).compose();
            } else if (niveau == 0 && sdk >= 29) {
                e = VibrationEffect.createPredefined(VibrationEffect.EFFECT_TICK);
            } else if (niveau == 1 && sdk >= 30 && click) {
                e = VibrationEffect.startComposition().addPrimitive(VibrationEffect.Composition.PRIMITIVE_CLICK, 1f).compose();
            } else if (niveau == 1 && sdk >= 29) {
                e = VibrationEffect.createPredefined(VibrationEffect.EFFECT_CLICK);
            } else if (niveau == 2 && sdk >= 29) {
                e = VibrationEffect.createPredefined(VibrationEffect.EFFECT_HEAVY_CLICK);
            } else {
                // Sans effets prédéfinis, ou au maximum : une impulsion, plus longue
                // pour plus forte (un moteur met quelques ms à prendre son élan)
                int ms = niveau == 0 ? 4 : niveau == 1 ? 7 : niveau == 2 ? 10 : 12;
                int amplitude = vibreur.hasAmplitudeControl() ? 255 : VibrationEffect.DEFAULT_AMPLITUDE;
                e = VibrationEffect.createOneShot(ms, amplitude);
            }
            vibreur.vibrate(e);
            return "";
        } catch (Throwable e) {
            String m = e.getMessage();
            return e.getClass().getSimpleName() + (m == null ? "" : " : " + m);
        }
    }

    /** Les pixels par pouce de l'écran, en x et en y (« 401.0 403.5 »). */
    public static String dpi(Context c, String arg) {
        DisplayMetrics m = c.getResources().getDisplayMetrics();
        return m.xdpi + " " + m.ydpi;
    }
}
