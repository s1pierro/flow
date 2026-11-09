# Construire des APK Android en Rust, sur Android, avec Termux

Ce tutoriel part d'une tablette ou d'un téléphone Android et arrive à un APK
installé, construit sur l'appareil lui-même : pas d'ordinateur, pas d'Android
Studio, pas de SDK ni de NDK à installer. Il prend Rustine comme exemple, puis
montre comment en faire votre propre application.

## Le principe

Termux est un environnement Linux qui tourne comme une application Android. Son
compilateur Rust est **natif Android** : ce qu'il produit est déjà une
bibliothèque Android, il n'y a rien à « cross-compiler ». Il reste à l'emballer
dans un APK, avec les outils d'Android que Termux fournit aussi :

| Étape | Outil | Ce qu'elle produit |
| --- | --- | --- |
| 1. Compilation Rust | `cargo` | `librustine.so`, la bibliothèque native |
| 2. Empaquetage | `aapt` | un APK contenant le manifeste et les ressources (icônes) |
| 3. Code Java (facultatif) | `javac` puis `d8` | `classes.dex` |
| 4. Assemblage | `zip` | la bibliothèque et le `.dex` ajoutés à l'APK |
| 5. Alignement | `zipalign` | l'APK aligné, exigé par Android |
| 6. Signature | `apksigner` | l'APK signé, installable |

Le script `build.sh` de Rustine fait ces six étapes.

L'application n'a pas d'activité Java : elle utilise `android.app.NativeActivity`,
fournie par Android, qui charge la bibliothèque Rust et lui passe la main. Les API
d'Android (fenêtre, entrées tactiles, OpenGL ES, son) sont appelées directement
depuis Rust. Les bibliothèques du système (OpenGL ES, AAudio, `libmediandk`) sont
chargées à l'exécution avec `dlopen` : rien n'est lié à la compilation.

## 1. Installer Termux

Installez Termux depuis [F-Droid](https://f-droid.org/packages/com.termux/) ou
depuis les [versions GitHub](https://github.com/termux/termux-app/releases). La
version du Play Store est ancienne et ne reçoit plus les paquets à jour.

Ouvrez Termux, puis mettez-le à jour :

    pkg update && pkg upgrade

Pour que Termux voie le stockage partagé (`/sdcard`), lancez une fois :

    termux-setup-storage

et acceptez l'autorisation.

## 2. Installer la chaîne de construction

    pkg install rust aapt apksigner zip unzip curl git openjdk-21 d8

- `rust` : `cargo` et `rustc`, natifs Android ;
- `aapt` : l'empaqueteur d'Android, avec `zipalign` ;
- `apksigner` : la signature ;
- `openjdk-21` : `keytool` (pour créer la clé de signature) et `javac` ;
- `d8` : convertit les classes Java en `.dex` ;
- `zip`, `unzip`, `curl`, `git` : utilitaires.

Vérifiez :

    cargo --version && aapt version && apksigner --version && d8 --version

## 3. Construire Rustine

    cd ~
    git clone https://github.com/s1pierro/rustine
    cd rustine
    bash build.sh

La première fois, deux choses se font une seule fois :

- **`android.jar`**, la définition de l'API Android dont `aapt` et `javac` ont
  besoin, est téléchargé depuis le dépôt officiel de Google (≈ 50 Mo) et placé
  dans le dossier. Le script le cherche aussi dans les projets voisins
  (`../*/android.jar`) : un seul téléchargement sert à tous vos projets.
- **Une clé de signature de debug** est créée dans
  `~/.android/debug.keystore`. Gardez-la : Android refuse de mettre à jour une
  application signée avec une autre clé.

La compilation Rust prend quelques minutes la première fois, quelques secondes
ensuite. L'APK est produit dans `dist/rustine.apk`.

> **Pourquoi la compilation a lieu dans `~/.cache/rust-apk/`** : rien ne peut
> s'exécuter depuis le stockage partagé (`/sdcard`), or `cargo` lance les
> scripts de build des dépendances. Le script place donc la compilation dans le
> dossier de Termux. Vos sources, elles, peuvent rester sur `/sdcard`, où un
> gestionnaire de fichiers ou un éditeur les voit.

## 4. Installer l'APK

Depuis Termux :

    bash build.sh install

ouvre l'installeur d'Android sur l'APK construit (la commande `termux-open`
vient du paquet `termux-tools`, normalement déjà présent). Sinon, copiez l'APK
sur le stockage partagé et ouvrez-le depuis un gestionnaire de fichiers :

    cp dist/rustine.apk /sdcard/Download/

Android demande d'autoriser l'installation depuis une source inconnue : l'APK
est signé avec une clé de debug, pas par une boutique.

## 5. Vérifier sans construire d'APK

Le compilateur de Termux tournant sur Android, les tests s'exécutent
directement :

    cargo test

L'interface de Rustine (widgets, fenêtres, clavier, mixeur, explorateur) ne fait
aucun appel OpenGL : elle est testée sans écran. Pour une vérification rapide :

    cargo check

`examples/apercu.rs` dessine l'interface hors écran, sur le processeur graphique
de l'appareil, et en écrit des images PPM dans un dossier :

    cargo run --example apercu -- /sdcard/Download/apercu

## 6. Faire sa propre application à partir de Rustine

Copiez le dossier, puis renommez l'application à quatre endroits.

1. **`Cargo.toml`** : le nom de la crate, qui devient celui de la bibliothèque
   (`libmonapp.so`).

       [package]
       name = "monapp"

2. **`AndroidManifest.xml`** : l'identifiant du paquet (unique sur l'appareil),
   le nom affiché, et le nom de la bibliothèque, qui doit être celui de la crate.

       package="fr.exemple.monapp"
       android:label="Mon app"
       <meta-data android:name="android.app.lib_name" android:value="monapp" />

3. **Le service Java** (`java/fr/s1pierro/rustine/Lecture.java`), s'il est
   gardé : déplacez-le dans le dossier de votre paquet, changez sa ligne
   `package`, le nom de la classe dans le manifeste et `System.loadLibrary`.
   La fonction Rust qu'il appelle porte le nom du paquet et de la classe :
   `Java_fr_s1pierro_rustine_Lecture_commande` (dans `src/mixage.rs`) devient
   `Java_fr_exemple_monapp_Lecture_commande`, et le nom de la classe passé au
   service dans `src/acces.rs` change de même. Sans son, supprimez le dossier
   `java/`, le service du manifeste, et passez `android:hasCode` à `false`.
4. **`res/`** : l'icône (`drawable/ic_foreground.xml`, un dessin vectoriel) et sa
   couleur de fond (`values/colors.xml`).

Puis `bash build.sh`. Le reste du script se règle tout seul : il lit le nom de
la crate dans `Cargo.toml`.

### Ce qu'il faut savoir en écrivant le code

- **Le point d'entrée** est `ANativeActivity_onCreate` (`src/android.rs`) : il
  reçoit l'activité et branche les rappels (fenêtre créée, entrées, reprise…).
  Le rendu tourne dans son propre thread.
- **JNI à la main** : pour appeler Java (autorisation d'accès aux fichiers,
  batterie, démarrer un service), `src/acces.rs` prend les fonctions JNI dans
  leur table, à leur rang fixé par `jni.h`. Un thread créé en Rust doit
  s'attacher à la machine virtuelle avant d'appeler Java, et s'en détacher
  avant de se terminer, sinon Android arrête l'application.
- **Les bibliothèques du système** se chargent avec `dlopen` et `dlsym` : voir la
  macro `bibliotheque!` de `src/audio.rs`. Ne liez jamais sur les bibliothèques
  de Termux (`libEGL` de Mesa, par exemple) : elles n'existent pas dans
  l'application installée.
- **Une seule dépendance** dans Rustine (fontdue, pour le texte). Toute
  dépendance pure Rust se compile ; celles qui demandent une bibliothèque C du
  système sont à éviter.
- **Les autorisations** se déclarent dans le manifeste ; celles qui demandent
  l'accord de l'utilisateur se demandent par JNI (`src/acces.rs`).

## 7. Facultatif : Claude Code dans Debian, avec la passerelle

On peut aussi faire travailler Claude Code (ou tout outil qui tourne sous
Debian) sur ces projets. Debian s'installe dans Termux avec `proot-distro`, mais
ses propres outils ne savent pas produire d'APK Android. Le dossier
[`passerelle/`](../passerelle) fournit `termux-run`, qui lance depuis Debian les
outils de Termux :

    pkg install proot-distro
    proot-distro install debian
    proot-distro login debian --bind /data/data/com.termux/files

L'option `--bind` rend visibles, depuis Debian, les outils et le dossier de
Termux. Ensuite, dans Debian, suivez [`passerelle/LISEZMOI.md`](../passerelle/LISEZMOI.md)
(installation de `termux-run`, consignes pour Claude Code, diagnostic), puis
construisez depuis le dossier d'un projet :

    termux-run bash build.sh

## Dépannage

| Symptôme | Cause et remède |
| --- | --- |
| `Permission denied` en lançant un programme depuis `/sdcard` | Rien ne s'exécute sur le stockage partagé : lancez `bash build.sh` plutôt que `./build.sh`, et laissez `CARGO_TARGET_DIR` dans le dossier de Termux (le script s'en charge). |
| « Commandes introuvables » au début du build | Le script indique le paquet à installer avec `pkg install`. |
| Téléchargement d'`android.jar` impossible | Récupérez `platform-30_r03.zip` sur `dl.google.com/android/repository/`, extrayez `android.jar` et placez-le dans le dossier du projet. |
| « Le paquet est en conflit avec un paquet existant » à l'installation | L'application déjà installée est signée avec une autre clé (autre appareil, clé recréée) : désinstallez-la d'abord. |
| L'application se ferme dès son ouverture | Lisez son journal avec `adb logcat -s Rustine` (ou le nom de votre application), depuis un ordinateur ou par le débogage sans fil d'Android. Cause fréquente : `android.app.lib_name` ne correspond pas au nom de la crate. |
| `UnsatisfiedLinkError` dans le journal | La fonction Rust appelée par Java ne porte pas le nom attendu (paquet et classe), ou n'est pas marquée `#[no_mangle] pub extern "C"`. |
