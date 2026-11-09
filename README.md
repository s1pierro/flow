# Flow

Application Android en Rust, née d'une copie de
[Rustine](https://github.com/s1pierro/Rustine) 0.8 : interface tactile et 3D en
OpenGL ES, fenêtres, clavier, audio, explorateur de fichiers.

Package `fr.s1pierro.flow`, réglages dans `Flow/` à la racine du stockage.

## Construire

Dans Termux, depuis ce dossier :

    bash build.sh            # produit dist/flow.apk
    bash build.sh install    # puis ouvre l'installeur Android

La compilation a lieu dans `~/.cache/rust-apk/flow`. Le guide complet est dans
[`docs/tutoriel.md`](docs/tutoriel.md) (écrit pour Rustine).

## Licence

PolyForm Noncommercial 1.0.0 : voir [LICENSE.md](LICENSE.md).
