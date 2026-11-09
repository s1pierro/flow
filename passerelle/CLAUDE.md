# Environnement : Debian (proot) sur tablette Android, chaîne de build dans Termux

Ce Debian tourne en proot dans Termux. Les APK Android en Rust se construisent
avec les outils **de Termux** (Rust natif Android, aapt/aapt2, zipalign,
apksigner), pas avec ceux de Debian.

## Règle

- Toute commande de build Android passe par `termux-run` :
  - `termux-run ./build.sh` : compile, empaquette, aligne et signe l'APK
    (résultat dans `dist/<app>.apk`)
  - `termux-run cargo check` : vérification rapide du code Rust
- Ne pas installer Rust, le SDK ou le NDK Android dans Debian : ce serait de la
  cross-compilation inutile, le Rust de Termux compile nativement pour Android.
- Si `termux-run` échoue, lancer `termux-run --check` et lire le diagnostic.
- Il manque un outil dans Termux ? Indiquer à l'utilisateur la commande
  `pkg install <paquet>` à lancer dans Termux ; ne pas la lancer soi-même.

## Projets

Les projets sont dans le dossier Termux : `~/termux/` (raccourci vers
`/data/data/com.termux/files/home`). Y travailler directement, pour que
l'utilisateur les retrouve aussi depuis Termux.

Chaque projet APK contient `Cargo.toml` (crate `cdylib`), `AndroidManifest.xml`
(NativeActivity, `android.app.lib_name` = nom de la crate) et `build.sh`.
Aucune dépendance Rust : les API Android (libandroid, AAudio via dlopen) sont
appelées directement en FFI.

## Installation de l'APK

`termux-open` peut ne pas fonctionner depuis Debian. Indiquer à l'utilisateur
le chemin de l'APK produit (`dist/<app>.apk`) : il l'ouvre depuis son
gestionnaire de fichiers, ou lance `./build.sh install` dans Termux.
