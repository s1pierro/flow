# Flow

Une *application d'exploitation* pour Android, en Rust : l'écran se partage en
fenêtres, et chaque fenêtre montre un **contexte**, une sous-application, pour
composer son espace de travail à son goût. Née d'une copie de
[Rustine](https://github.com/s1pierro/Rustine) 0.8.

Contextes : Accueil (les contextes en cartes, celui des fenêtres nées du menu), Fichiers (les
entrées en cartes : empreinte des dossiers, badge du type MIME des fichiers), Texte (éditeur à numéros de ligne et blocs entre accolades repliables, qui prend aussi la place de l'explorateur
quand on y touche un fichier texte), Disque, Radio, Mixeur (la console audio : une tranche par piste), Encre (dessin à
main levée, enregistré en SVG : épaisseur, couleur, gomme, navigation à deux doigts),
Orchestre (l'orchestrateur : empreintes du stockage tenues à jour, batterie surveillée,
recensement des contextes, journal ; ses réglages dans `Flow/orchestre.txt`), Aide. Les paramètres de l'application (apparence, clavier,
gestes) sont dans le panneau latéral. Chaque fenêtre a sa propre instance, avec son propre état : deux fenêtres
Fichiers parcourent chacune leur dossier, deux fenêtres Texte éditent chacune leur
document. Viendra l'édition de SVG.

Les contextes peuvent recevoir des gestes élaborés, inspirés de
[tnt.js](https://github.com/s1pierro/tnt.js) : tap, appui et appui long selon la
durée, saisie avec un curseur déporté au bout d'une barre (le doigt ne cache plus ce
qu'il désigne), pincement à deux doigts.

Package `fr.s1pierro.flow`. Dans `Flow/` à la racine du stockage :

- `interface.txt` : l'apparence, le clavier et les gestes ;
- `espace.txt` : les fenêtres, leur contexte et leur état, retrouvés au lancement
  suivant ;
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
