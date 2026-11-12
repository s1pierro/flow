# Flow

Une *application d'exploitation* pour Android, en Rust : l'écran se partage en
fenêtres, et chaque fenêtre montre un **contexte**, une sous-application, pour
composer son espace de travail à son goût. Née d'une copie de
[Rustine](https://github.com/s1pierro/Rustine) 0.8.

**Télécharger :** [flow-0.26.apk](https://github.com/s1pierro/flow/releases/download/v0.26/flow-0.26.apk)

Contextes : Accueil (les contextes en cartes, celui des fenêtres nées du menu), Fichiers (les
entrées en cartes : empreinte des dossiers, badge du type MIME des fichiers ; recherche, tri par nom, date,
taille ou type ; créer, renommer ; sélection à l'appui long, copier, couper, coller et supprimer, en
arrière-plan avec avancement), Texte (éditeur aux gestes : glisser sélectionne au curseur déporté, appui choisit un mot, appui long propose
Coller, deux doigts font défiler et pincer change la taille, Copier/Couper/Coller au-dessus de la sélection, une poignée à chaque bout pour l'ajuster ;
Annuler et Rétablir à gauche du titre, par mot ou par ligne ;
numéros de ligne, blocs entre accolades repliables et coloration
syntaxique pilotée par des dictionnaires en texte dans `Flow/syntaxes/` : Rust, C, C++,
JavaScript, CSS, HTML, SVG, Markdown, TOML, JSON, Python ; on en ajoute en déposant un fichier, qui prend aussi la place de l'explorateur
quand on y touche un fichier texte), Disque, Radio, Mixeur (la console audio : une tranche par piste,
un égaliseur paramétrique à cinq bandes par piste et sur la sortie, réglé en tirant les poignées de sa courbe), Encre (dessin à
main levée, enregistré en SVG : épaisseur, couleur, gomme, navigation à deux doigts ; chaque trait
fini est simplifié sans perte), SVG (éditeur
de dessin vectoriel : ouvre tout SVG, trace rectangles, ellipses, lignes, polygones et textes avec
fond, trait et épaisseur ; sélection avec poignées pour déplacer, agrandir et tourner ; édition des sommets des chemins (déplacer
sommets et contrôles, ajouter en touchant le contour, retirer, courber, redresser, ouvrir, fermer) ; simplifier les chemins
(sans perte, ou avec un écart permis : droites, courbes rapides, courbes optimales en nombre minimal,
ou B-spline à nœuds automatiques et courbure continue ; en aperçu) ; dupliquer,
supprimer, monter, descendre ; annuler, rétablir ; la source colorée à côté, synchronisée), Argile
(sculpture 3D comme de la terre : gonfler, creuser, lisser, aplatir, pincer, tirer, avec symétrie ;
la forme tourne au doigt, se subdivise pour les détails et s'enregistre en glTF binaire `.glb`),
Orchestre (l'orchestrateur : empreintes du stockage tenues à jour, batterie surveillée,
recensement des contextes, journal ; ses réglages dans `Flow/orchestre.txt`), Aide. Les paramètres de l'application (apparence, clavier,
gestes) sont dans le panneau latéral. Chaque fenêtre a sa propre instance, avec son propre état : deux fenêtres
Fichiers parcourent chacune leur dossier, deux fenêtres Texte éditent chacune leur
document.

La barre du bas accueille les **applets** des contextes, à la façon du tableau de
bord de GNOME 2 : ils pilotent ce qui tourne en arrière-plan sans ouvrir de fenêtre.
Radio (le flux en cours, lecture ou arrêt), Mixeur (le vumètre de la sortie),
Fichiers (l'avancement d'une copie) et Énergie (la batterie : niveau, temps restant
avant la batterie vide ou la charge complète, ou les deux, au choix) ; toucher un applet ouvre son volet (volumes,
flux récents, Arrêter…). Les applets montrés se choisissent dans les paramètres.

Les contextes peuvent recevoir des gestes élaborés, inspirés de
[tnt.js](https://github.com/s1pierro/tnt.js) : tap, appui et appui long selon la
durée, saisie avec un curseur déporté au bout d'une barre (le doigt ne cache plus ce
qu'il désigne), pincement à deux doigts.

Package `fr.s1pierro.flow`. Dans `Flow/` à la racine du stockage :

- `interface.txt` : l'apparence, le clavier, les gestes et les applets de la barre ;
- `espace.txt` : les fenêtres, leur contexte et leur état, retrouvés au lancement
  suivant ;
- `mixeur.txt` : les réglages du mixeur (volume général, et pour chaque piste volume,
  balance, bascules) et ses égaliseurs, retrouvés au lancement suivant ;
- `brouillons/` : les modifications non enregistrées des documents, mises à l'abri.
- `empreintes.txt` : la synthèse de l'analyse du stockage (les dossiers et leurs
  tailles), qui donne les empreintes des dossiers dans Fichiers sans tout parcourir
  au lancement ; refaite au plus une fois par jour, ou à chaque analyse demandée
  dans Disque.

Les deux se modifient à la main. Pour écrire un nouveau contexte, voir
[`docs/contextes.md`](docs/contextes.md).

## Construire

Dans Termux, depuis ce dossier :

    bash build.sh            # produit dist/flow.apk
    bash build.sh install    # puis ouvre l'installeur Android

La compilation a lieu dans `~/.cache/rust-apk/flow`. Le guide complet est dans
[`docs/tutoriel.md`](docs/tutoriel.md) (écrit pour Rustine).

## Licence

PolyForm Noncommercial 1.0.0 : voir [LICENSE.md](LICENSE.md).
