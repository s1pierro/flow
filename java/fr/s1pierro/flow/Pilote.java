package fr.s1pierro.flow;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/**
 * Le pilotage de Flow depuis une autre application, par le bus d'Android : chaque
 * commande arrive en diffusion explicite, par exemple depuis Termux :
 *
 *   am broadcast -n fr.s1pierro.flow/.Pilote -a fr.s1pierro.flow.PILOTE \
 *       --es jeton XXXX --es cmd "ping"
 *
 * La réponse revient dans le même échange (les données du résultat, que `am` affiche).
 * Rust (pilote.rs) vérifie que le pilotage est armé dans les Paramètres et que le jeton
 * est le bon, puis exécute la commande sur le thread de rendu. Le récepteur répond sur
 * un thread à part (goAsync) : le thread principal ne bloque jamais.
 */
public class Pilote extends BroadcastReceiver {
    static {
        // La même bibliothèque que l'activité (même processus : déjà en mémoire, ses
        // statiques partagées) ; c'est ce chargement qui lie la méthode native
        System.loadLibrary("flow");
    }

    /** Exécute la commande ; rend la réponse (première ligne « ok » ou « erreur »). */
    static native String commande(String jeton, String cmd);

    @Override
    public void onReceive(Context c, Intent i) {
        final String jeton = i.getStringExtra("jeton");
        final String cmd = i.getStringExtra("cmd");
        final PendingResult r = goAsync();
        new Thread(new Runnable() {
            public void run() {
                String reponse;
                try {
                    reponse = commande(jeton == null ? "" : jeton, cmd == null ? "" : cmd);
                } catch (UnsatisfiedLinkError e) {
                    // La bibliothèque native n'est chargée que par l'activité
                    reponse = "erreur\nFlow n'est pas lancé.";
                } catch (Throwable e) {
                    reponse = "erreur\n" + e;
                }
                r.setResultCode(reponse.startsWith("ok") ? 0 : 1);
                r.setResultData(reponse);
                r.finish();
            }
        }).start();
    }
}
