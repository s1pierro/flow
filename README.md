# Flow

Une *application d'exploitation* pour Android, en Rust : l'écran se partage en
fenêtres, et chaque fenêtre montre un **contexte**, une sous-application, pour
composer son espace de travail à son goût. Née d'une copie de
[Rustine](https://github.com/s1pierro/Rustine) 0.8.

**Télécharger :** [flow-0.51.apk](https://github.com/s1pierro/flow/releases/download/v0.51/flow-0.51.apk)

Contextes : Accueil (les contextes en cartes, celui des fenêtres nées du menu), Fichiers (les
entrées en cartes : empreinte des dossiers, badge du type MIME des fichiers ; recherche, tri par nom, date,
taille ou type ; créer, renommer ; sélection à l'appui long, copier, couper, coller et supprimer, en
arrière-plan avec avancement ; un .apk touché s'installe, se met à jour ou ouvre son application, par l'installateur d'Android ; les images
(JPEG, PNG, WebP, GIF, BMP, HEIC, AVIF), décodées par Android, ont leur vignette, et une image touchée s'ouvre en grand :
pincer zoome, glisser déplace ou passe à la suivante, toucher un bord aussi), Texte (éditeur aux gestes : glisser sélectionne au curseur déporté, appui choisit un mot, appui long propose
Coller, deux doigts font défiler et pincer change la taille, Copier/Couper/Coller au-dessus de la sélection, une poignée à chaque bout pour l'ajuster ;
Annuler et Rétablir à gauche du titre, par mot ou par ligne ;
numéros de ligne, blocs entre accolades repliables et coloration
syntaxique pilotée par des dictionnaires en texte dans `Flow/syntaxes/` : Rust, C, C++,
JavaScript, CSS, HTML, SVG, Markdown, TOML, JSON, Python ; on en ajoute en déposant un fichier, qui prend aussi la place de l'explorateur
quand on y touche un fichier texte), Disque, Boombox (la radio : un en-tête aux couleurs inversées porte les modes Flux, Magnéto et
Bibliothèque, la lecture, l'enregistrement et l'adresse du flux ; les flux récents défilent dessous ; au lancement, le dernier flux
écouté est décodé en silence et ses cinq dernières minutes restent en mémoire, si bien que le bouton rouge peut enregistrer
en .wav à partir du passé, dans Music/Boombox ; le Magnéto liste et joue les enregistrements, et coupe la veille, qui consomme environ 60 Mo de données par heure), Toile (les pages web, HTML, CSS et JavaScript, par le
moteur d'Android : précédent, suivant, recharger, adresse ou chemin ; Fichiers y ouvre les .html, dont le bouton Source ouvre
le fichier dans l'éditeur Texte à côté, et la page se recharge à chaque enregistrement ; la console JavaScript s'affiche dans la
ligne d'état), Mixeur (la console audio : une tranche par piste,
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
Mailles (éditer la géométrie d'un `.glb` : sommets, arêtes, faces ; la fenêtre se partage en vues
comme l'espace de travail — Vue 3D, Arborescence des objets, tableau des Opérations — et une
barre rapide propose ce que la sélection permet : déplacer, tourner, échelle, extruder,
subdiviser, fusionner, créer une face, trianguler, retourner, dupliquer, supprimer ; annuler,
rétablir ; Fichiers ouvre désormais les `.glb` dans Mailles),
Lieux (les lieux réels de S.A.G.E., fichiers `.reel` : les générer depuis le téléphone — jusqu'à
20 × 20 km autour d'un point de France, largeur et longueur réglables, relief et toits LiDAR de l'IGN, eau, routes, ponts, bâtiments et
occupation du sol d'OpenStreetMap —, les explorer sur une carte en tuiles ou en 3D (le relief
drapé de la carte, à niveaux de détail, les bâtiments en volume), et les éditer :
bâtiments, routes et ponts, pinceaux de relief, d'eau et de sol ; annuler, rétablir. Le format
vient de la crate partagée `sage-reel` du dépôt S.A.G.E. Un lieu généré est dérivé d'OSM (ODbL) :
il ne se publie pas),
Orchestre (l'orchestrateur : empreintes du stockage tenues à jour, batterie surveillée,
recensement des contextes, journal ; ses réglages dans `Flow/orchestre.txt`),
Télécommande (piloter un autre Flow par le Wi-Fi local — découverte sur le réseau, liaison par
un code à quatre chiffres, puis pavé tactile avec curseur, écran en miroir à plusieurs doigts ou
clavier, ou manette de jeu — deux sticks, croix, A B X Y, tranches, gâchettes — qui fait
voler la 3D des Lieux et sert de manette à WhiteRoom ; l'aller-retour mesuré s'affiche — ou se
laisser piloter), Aide. Les paramètres de l'application (apparence, clavier,
gestes) sont dans le panneau latéral. Deux façons de disposer les fenêtres (Paramètres, Interface) :
**Full flow**, où elles se partagent tout l'espace, et **Free flow**, où chacune flotte à sa place
comme sur un bureau (tirer la barre la déplace, la poignée la redimensionne, la toucher la passe
devant ; une barre des tâches à côté du logo réduit
ou retrouve chaque fenêtre). Chaque fenêtre a sa propre instance, avec son propre état : deux fenêtres
Fichiers parcourent chacune leur dossier, deux fenêtres Texte éditent chacune leur
document.

La barre du bas accueille les **applets** des contextes, à la façon du tableau de
bord de GNOME 2 : ils pilotent ce qui tourne en arrière-plan sans ouvrir de fenêtre.
Boombox (le flux en cours, lecture ou arrêt), Mixeur (le vumètre de la sortie),
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
