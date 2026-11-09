# Flow

Une *application d'exploitation* pour Android, en Rust : l'écran se partage en
fenêtres, et chaque fenêtre montre un **contexte**, une sous-application, pour
composer son espace de travail à son goût. Née d'une copie de
[Rustine](https://github.com/s1pierro/Rustine) 0.8.

Contextes : Fichiers, Texte (éditeur, qui prend aussi la place de l'explorateur
quand on y touche un fichier texte), Disque, Radio (et mixeur), Aide. Les paramètres de
l'application (apparence, clavier) sont dans le panneau latéral. Chaque fenêtre a sa propre instance, avec son propre état : deux fenêtres
Fichiers parcourent chacune leur dossier, deux fenêtres Texte éditent chacune leur
document. Viendra l'édition de SVG.

Package `fr.s1pierro.flow`. Dans `Flow/` à la racine du stockage :

- `interface.txt` : l'apparence et le clavier ;
- `espace.txt` : les fenêtres, leur contexte et leur état, retrouvés au lancement
  suivant ;
- `brouillons/` : les modifications non enregistrées des documents, mises à l'abri.

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
