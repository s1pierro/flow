# Passerelle Debian → Termux

Permet à Claude Code, lancé dans Debian, d'utiliser la chaîne de build APK de Termux.

## Installation (à confier à Claude Code dans Debian)

Décompresse ce dossier dans Debian, puis demande à Claude Code :

> Installe la passerelle du dossier termux-bridge et lance le diagnostic.

Ce qu'il doit faire :

1. Copier `termux-run` dans `/usr/local/bin/` et le rendre exécutable.
2. Copier `CLAUDE.md` dans `~/.claude/CLAUDE.md` (consignes valables pour tous
   les projets) ; s'il en existe déjà un, ajouter le contenu à la fin.
3. Créer le raccourci `~/termux` vers `/data/data/com.termux/files/home`
   (tes projets Termux, accessibles depuis Debian).
4. Lancer `termux-run --check` et expliquer le résultat.

## Si le diagnostic signale un dossier invisible

Debian doit voir les dossiers de Termux. Quitte Debian et relance-le depuis
Termux en les partageant :

    proot-distro login debian --bind /data/data/com.termux/files

Cette option rend visibles, à leur emplacement réel, les outils de Termux
(`…/files/usr`) et ton dossier Termux (`…/files/home`).

N'utilise pas `--termux-home` : il remplace le dossier personnel de Debian,
où sont installés Claude Code et ta connexion, qui deviendraient introuvables.

Si tu lances Debian via un raccourci ou un script, ajoute-y cette option.

## Utilisation

Dans un projet, par exemple `~/termux/audio-test` :

    termux-run ./build.sh

L'APK est produit dans `dist/`.
