# Intégrer un contexte dans Flow

Flow est une *application d'exploitation* : l'écran se partage en fenêtres, et chaque
fenêtre montre un **contexte**, une sous-application (Fichiers, Disque, Radio…).
L'utilisateur compose son espace de travail en partageant les fenêtres et en
choisissant le contexte de chacune. Ce guide explique comment ajouter un contexte,
par exemple un éditeur de texte ou de SVG.

Les paramètres de l'application (apparence, clavier, gestes) ne sont pas un contexte : ils
vivent dans le panneau latéral (`src/parametres.rs`), ouvert par une pastille de la
barre du bas.

## Les principes

- **Une instance par fenêtre.** Chaque fenêtre porte sa propre instance de contexte,
  avec son propre état. Deux fenêtres Texte éditent chacune leur document, en même
  temps. Rien n'empêche un contexte de travailler sur plusieurs documents.
- **Ce qui est partagé va dans `Commun`.** L'apparence, le clavier virtuel,
  l'analyse du stockage et l'historique des radios sont communs à toutes les
  fenêtres. Une instance ne garde pas de référence vers
  `Commun` : il lui est passé à chaque appel.
- **Tout se retrouve au lancement suivant.** La disposition des fenêtres, le
  contexte de chacune et son état sont enregistrés dans un fichier texte,
  `Flow/espace.txt`, à la racine du stockage. Chaque instance y écrit ses paires
  `clé = valeur`, et les relit à l'ouverture.

## Les fichiers

| Fichier | Rôle |
| --- | --- |
| `src/contexte.rs` | Le trait `Contexte`, `Genre`, `Commun`, `Etat` (l'état enregistré) |
| `src/contextes/mod.rs` | La liste `GENRES`, et des outils partagés (`ligne_fichier`, `Puce`, `empreinte`, `couper`, `abreger`) |
| `src/contextes/<nom>.rs` | Un contexte |
| `src/espace.rs` | Le format de `espace.txt`, sa lecture et son écriture |
| `src/parametres.rs` | Le panneau des paramètres de l'application |
| `src/app.rs` | L'application : fenêtres, instances, panneau latéral, barre |

Les contextes existants servent d'exemples, du plus simple au plus complet :
`aide.rs` (rien à retenir), `radio.rs` (un champ de texte et le clavier),
`mixeur.rs` (une ressource globale, une animation, deux mises en page selon la place), `fichiers.rs` (un dossier par fenêtre, relu au retour, une demande
d'ouverture, l'appui long `cx.ui.click_long`, une boîte de dialogue, un état partagé entre
fenêtres et une opération en arrière-plan : `Commun::fichiers`, `src/operations.rs`), `disque.rs` (une ressource partagée dans `Commun`, un état enregistré
par noms et résolu plus tard) — l'analyse se résume dans `Commun::empreintes`, enregistrée
dans `Flow/empreintes.txt`, que Fichiers lit sans relancer de parcours, `texte.rs` (un contexte à document : lecture,
enregistrement, brouillons, découpage des lignes, sélection et historique (annuler, rétablir) dans `src/tampon.rs`,
une zone à gestes pour éditer : tap, glisser pour sélectionner, appui, appui long, pincement ;
le presse-papier par `acces::copier` et `acces::presse_papier`), `encre.rs`
(une zone à gestes élaborés, un document SVG enregistré à la demande), `svg.rs` (un
éditeur de dessin : le document en arbre, son rendu et ses retouches dans `src/svg.rs`,
des poignées en `cx.ui.drag`, un historique d'instantanés, et un `Texte` intégré pour la
source — `Texte::integre`, sans en-tête, dont le contexte relaie les frappes), `argile.rs`
(une vue 3D : le maillage et ses pinceaux dans `src/sculpture.rs`, le glTF binaire dans
`src/glb.rs`, et `DrawList::vue3d`, qui place une vue 3D à son rang dans la liste de dessin :
le moteur de rendu la dessine avec sa propre profondeur dans son rectangle, et ce qui suit
passe par-dessus), `orchestre.rs`
(la vue d'un module global, sans état propre).

`texte.rs` est la référence pour un contexte qui édite des fichiers ; `svg.rs` montre
comment en emprunter un autre (l'éditeur de texte) à l'intérieur du sien.

## Ajouter un contexte, pas à pas

### 1. Le module

Créer `src/contextes/texte.rs` :

```rust
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::path::PathBuf;

pub const CLE: &str = "texte";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Texte", legende: "Éditer un texte brut", icone: Icon::Fichier, ouvrir, applet: None };

const TEXTE: Id = id("texte");

#[derive(Clone, Default)]
pub struct Texte {
    chemin: Option<PathBuf>,
    // le document, le curseur, le défilement…
}

/// Crée l'instance depuis son état enregistré ; vide pour une fenêtre nouvelle.
fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    Box::new(Texte { chemin: e.texte("document").map(PathBuf::from) })
}

impl Contexte for Texte {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        Box::new(self.clone())
    }

    fn ecrire(&self, e: &mut Etat) {
        if let Some(c) = &self.chemin {
            e.mettre("document", c.display());
        }
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let r = p.contenu;
        cx.dl.rrect(r, 0.0, cx.couleurs().surface);
        let mut b = cx.body(sub_id(TEXTE, p.id as usize), r, &c.scrolls);
        cx.paragraph(&mut b, "…");
        cx.body_end(b, &mut c.scrolls);
        "ligne d'état".into()
    }
}
```

### 2. La déclaration

Dans `src/contextes/mod.rs` : `pub mod texte;`, puis `texte::GENRE` dans `GENRES`.
Le contexte apparaît alors dans le menu de chaque fenêtre, et en carte dans
l'Accueil, avec sa `legende` (une phrase courte, sur deux lignes au plus). Le test
`les_cles_sont_uniques_et_valides` vérifie la clé. `applet` reste à `None` si le
contexte n'a pas d'applet dans la barre (voir « Les applets de la barre »).

### 3. Vérifier

```
CARGO_TARGET_DIR=$HOME/.cache/rust-apk/flow cargo test
```

## Le trait `Contexte`

Seules `cle`, `dessiner` et `dupliquer` sont obligatoires ; les autres méthodes ont
une version par défaut qui ne fait rien.

| Méthode | Quand | Usage |
| --- | --- | --- |
| `cle()` | toujours | La clé du `Genre` : l'application vérifie ainsi qu'une fenêtre a la bonne instance |
| `dessiner(cx, c, p)` | chaque image | Dessine dans `p.contenu`, renvoie la ligne d'état de la barre de la fenêtre |
| `dupliquer()` | partage d'une fenêtre | L'instance de la nouvelle fenêtre. Un contexte à document peut rendre une copie (même fichier) ou une instance vide |
| `ecrire(e)` | enregistrement | Écrit dans `e` de quoi rouvrir l'instance |
| `avancer(dt, c)` | avant le dessin | Fait avancer les animations |
| `anime(c)` | après chaque image | Vrai pour obtenir une image de plus sans événement (sinon Flow ne redessine pas) |
| `frappe(f, cible, frappe, c)` | frappe du clavier virtuel | Renvoie vrai si `cible` est un champ de cette instance |
| `retour(c)` | retour dans l'application | Relit ce qui a pu changer sur le disque |
| `mettre_a_l_abri(c)` | fenêtre fermée, autre contexte, arrière-plan | Écrit un brouillon de ce qui n'est pas enregistré (dans `c.dossier`) |
| `document()` | demande d'ouverture | Le fichier édité : un même fichier ne s'ouvre pas deux fois |

### Les identifiants d'interface

Chaque widget a un identifiant (`Id`). Deux fenêtres du même contexte ne doivent pas
partager les leurs : on les dérive du numéro de fenêtre, `sub_id(BASE, p.id as usize)`,
puis `sub_id(base, n)` pour chaque élément. Le défilement d'un corps (`cx.body`) est
retenu sous son identifiant, dans `c.scrolls`.

### Le clavier virtuel

`clavier::champ(cx, &mut b, cle, "indication", &mut edition, &mut c.clavier)` dessine
un champ ; le toucher ouvre le clavier pour lui. Les frappes arrivent ensuite par
`frappe`, avec la `cible` du champ :

```rust
fn frappe(&mut self, f: FenetreId, cible: Id, frappe: Frappe, c: &mut Commun) -> bool {
    if cible != sub_id(CHAMP, f as usize) {
        return false;
    }
    if self.edition.appliquer(frappe) {
        c.clavier.fermer(); // Entrée
    }
    true
}
```

## Les gestes élaborés

Les widgets (`click`, `drag`, `body`) suffisent aux listes et aux réglages. Pour un
terrain de travail (un dessin, un plan, un texte), Flow fournit des gestes inspirés de
[tnt.js](https://github.com/s1pierro/tnt.js), dans `src/gestes.rs`. Un contexte déclare
une zone et lit les événements reçus depuis l'image précédente :

```rust
for g in cx.ui.gestes(sub_id(base, 1), zone) {
    match g {
        Geste::Tap { x, y, .. } => { /* choisir */ }
        Geste::Appui { x, y, intensite, .. } => { /* menu, variante */ }
        Geste::AppuiLong { x, y, .. } => { /* action forte */ }
        Geste::SaisieDebut { x, y, .. } => { /* le curseur déporté apparaît en (x, y) */ }
        Geste::SaisieDeplace { x, y, .. } => { /* il suit le doigt, au bout de la barre */ }
        Geste::SaisieFin { .. } => {}
        Geste::Pincement { echelle, centre, deplacement } => { /* zoom, déplacement */ }
        _ => {}
    }
}
```

Une zone passe par ces états (`Geste::Etat` annonce chaque changement) :

| État | Entrée | Sortie |
| --- | --- | --- |
| `Tap` | un doigt se pose | levé avant la durée d'appui : `Tap` |
| `Appui` | le doigt reste posé | levé : `Appui` ; bougé de la distance : `Annule` |
| `AppuiLong` | le doigt reste encore | levé : `AppuiLong` ; bougé : `Annule` |
| `Saisie` | le doigt bouge de la distance pendant `Tap` | levé : `SaisieFin` |
| `Pincement` | un second doigt pendant `Tap` | un doigt levé : `PincementFin` |

Au lever, exactement un de `Tap`, `Appui`, `AppuiLong` arrive, selon la durée, avec
son intensité (où il en était entre ses seuils) et sa précision (le plus grand écart
du doigt, en pixels). En saisie, le curseur apparaît là où le doigt s'est posé et le
suit au bout d'une barre rigide : le doigt ne cache pas ce qu'il désigne. Avec une
distance réglée à 0, il n'y a pas de barre : la saisie part dès 10 dp de glissement
et le curseur reste sous le doigt. Cinq doigts
annulent le geste (`SaisieAnnulee` pendant une saisie). Les coordonnées sont celles de
l'écran, en pixels.

Flow dessine lui-même le retour visuel (l'anneau de durée, la barre et le curseur,
le trait du pincement) : le contexte n'a qu'à réagir. La distance et les durées se
règlent dans l'onglet Gestes des paramètres (`Commun::gestes`). Le contexte Encre
(`contextes/encre.rs`) sert d'exemple : un doigt trace (saisie, tap), deux doigts
naviguent (pincement : zoom autour du centre des doigts et déplacement).

## L'Accueil et le partage par le menu

L'Accueil (`contextes/accueil.rs`, premier de `GENRES`, donc contexte de la première
fenêtre) montre les autres contextes en cartes ; toucher une carte envoie un
`Demande::Ouvrir` pour sa propre fenêtre. Le menu de la barre d'une fenêtre porte, au-dessus de
« Fermer la fenêtre », deux icônes : partager côte à côte ou l'une sous l'autre
(`Surface::scinder`, en deux moitiés). La nouvelle fenêtre montre `Surface::accueil`
avec une instance neuve (`Changement::Nouvelle`). `Surface::partager` (sans vue
donnée) copie l'instance (`Changement::Partagee`, `Contexte::dupliquer`) ; plus rien
ne s'en sert dans l'interface depuis le retrait de la pastille +.

## Ouvrir un contexte depuis un autre

Une instance peut demander l'ouverture d'un autre contexte, avec un état :

```rust
let mut etat = Etat::default();
etat.mettre("document", chemin.display());
c.demandes.push(Demande::Ouvrir { depuis: p.id, cle: texte::CLE, etat });
```

Après le dessin, l'application remplace le contexte de la fenêtre `depuis` par
celui-ci, ouvert sur cet état (l'ancienne instance passe d'abord par
`mettre_a_l_abri`). Si une fenêtre de ce contexte édite déjà le même `document`,
rien ne s'ouvre. C'est ainsi que l'explorateur ouvre un fichier texte : l'éditeur
prend sa place dans la fenêtre.

## Les documents non enregistrés

Un contexte à document ne doit rien perdre. Quand sa fenêtre se ferme, qu'elle
change de contexte ou que l'application passe en arrière-plan, `mettre_a_l_abri`
est appelé : l'instance écrit ce qui n'est pas enregistré dans un brouillon, dans
`c.dossier` (`Flow/brouillons/`), et le retient dans son état (`ecrire`). Rouverte,
elle relit le brouillon plutôt que le fichier. Une fois le document enregistré, le
brouillon est effacé. Le brouillon d'une fenêtre fermée reste dans
`Flow/brouillons/`, où on le retrouve avec l'explorateur.

## L'état enregistré

`Etat` est une liste ordonnée de paires `clé = valeur` : `mettre`, `mettre_oui`
pour écrire ; `texte`, `nombre`, `nombres`, `oui` pour lire. Les valeurs tiennent sur
une ligne (les retours à la ligne deviennent des espaces).

Règles :

- **Tolérer l'absence.** Une valeur absente ou illisible prend sa valeur par défaut :
  le fichier peut venir d'une autre version, ou avoir été retouché à la main.
- **N'enregistrer que l'état de travail** : le document ouvert, la position, les
  choix de la fenêtre. Pas le contenu d'un document : il reste dans son propre
  fichier, que le contexte enregistre lui-même.
- **Préférer ce qui survit aux changements**, par exemple des noms plutôt que des
  indices (voir `disque.rs`).
- **Écrire peu de décimales** : l'espace n'est réécrit que si son texte change, et
  pendant une animation au plus toutes les 5 s.

L'espace est enregistré quand le doigt se lève et que rien ne s'anime, ou au plus
toutes les 5 s pendant une animation, et toujours quand l'application passe en
arrière-plan.

## Le fichier `espace.txt`

```text
fenetres = C0.4000(F3:fichiers,L0.5000(F2:texte,F1:radio))

[fenetre 3]
dossier = /storage/emulated/0/Download
caches = non

[fenetre 2]
document = /storage/emulated/0/Notes/idees.txt
```

- `fenetres` : `F<numéro>:<clé>` pour une fenêtre ; `C<part>(…,…)` pour deux
  fenêtres côte à côte, `L<part>(…,…)` pour l'une sur l'autre.
- `[commun]` : ce qui est partagé et doit être retrouvé (vide pour l'instant).
- `[fenetre N]` : l'état de la fenêtre N, tel que son contexte l'écrit.

Une clé de contexte inconnue ouvre le contexte de repli (`contextes::REPLI`, l'Aide).
Une disposition illisible laisse les fenêtres comme elles sont. Le fichier se modifie
à la main : Flow le relit au retour dans l'application.

## Ce qui est partagé : `Commun`

Ajouter un champ à `Commun` quand plusieurs fenêtres, ou plusieurs contextes, doivent
voir la même chose (une ressource coûteuse comme l'analyse du stockage, un réglage
global). Un réglage de l'application va dans le panneau des paramètres et le fichier
des réglages (`reglages.rs`) ; un état partagé à retrouver au lancement suivant va
dans la section `[commun]` (voir `App::espace` et `App::ouvrir_espace`).

## La coloration syntaxique

`src/syntaxe.rs` colore un texte d'après un dictionnaire par langage
(`syntaxes/<clé>.txt` dans le projet, intégré à l'APK et recopié dans
`Flow/syntaxes/`, où il prime et peut être retouché ; un nouveau `.txt` y ajoute un
langage). `Commun::syntaxes()` donne la bibliothèque, relue au retour dans
l'application ; `Bibliotheque::pour(nom)` trouve le langage par extension et
`colorer` rend des plages `(début, fin, Classe)`. Les clés du format : `nom`,
`extensions`, `commentaire_ligne`, `commentaire_bloc`, `chaines`, `chaines_triples`,
`echappement`, `caracteres`, `annotations`, `prefixe_annotation`, `preprocesseur`,
`prefixe_mot_cle`, `diese`, `macros`, `majuscules_types`, `tirets`, `proprietes`,
`sections`, `cles`, `cles_chaines`, `balisage`, `incorpore`, `markdown`, `mots_cles`,
`types`, `constantes` (voir `syntaxes/rust.txt`). Trois analyseurs : le code, le
balisage (`balisage = oui`) et le Markdown (`markdown = oui`, ses blocs de code
colorés dans le langage de leur étiquette, `Bibliotheque::nomme`).

## L'orchestrateur

`src/orchestre.rs` tient à jour ce dont toute l'application dépend, hors de toute
fenêtre ; il vit dans `Commun::orchestre` et travaille à chaque image depuis
`Commun::orchestrer` (appelé par `App::frame` après le dessin des fenêtres). Le
contexte Orchestre (`contextes/orchestre.rs`) en montre les états et les réglages.

| Tâche | Ce qu'il fait | Fichier |
| --- | --- | --- |
| Empreintes | Refait la synthèse du stockage quand elle a vieilli au-delà de la période réglée (24 h par défaut), la reporte si la batterie est basse hors charge | `Flow/empreintes.txt` |
| Batterie | La relève à intervalle réglé (boucle de rendu, `android.rs`), garde deux jours d'historique, en tire débit et autonomie, annonce le seuil bas | `Flow/batterie.txt` |
| Recensement | Les genres de `GENRES` et les instances ouvertes (`App::recenser`), avec leur document | — |

Ses réglages sont dans `Flow/orchestre.txt` (paires `clé = valeur`, modifiables à la
main) ; ce qu'il fait s'inscrit dans un journal en mémoire (`Orchestre::noter`).
Un contexte n'a rien à faire pour être recensé. Une nouvelle tâche de fond (une
synchronisation, un nettoyage des brouillons…) s'ajoute dans `Commun::orchestrer`,
avec son état exposé dans le contexte Orchestre.

## Les applets de la barre

À la façon du tableau de bord de GNOME 2, la barre du bas accueille des **applets** :
de petits éléments de contrôle ou de suivi qui pilotent ce qui tourne en
arrière-plan, sans ouvrir de fenêtre. Un contexte en déclare un (ou aucun) dans son
`Genre` :

```rust
pub const GENRE: Genre = Genre { cle: CLE, nom: "Radio", /* … */ ouvrir, applet: Some(applet) };

struct AppletRadio;

fn applet() -> Box<dyn crate::applet::Applet> {
    Box::new(AppletRadio)
}
```

L'applet est **unique pour toute l'application**, créé au lancement, qu'une fenêtre
du contexte soit ouverte ou non : il lit et règle l'état commun (`Commun`, le
lecteur de la radio, le mixeur…), jamais une instance. Le trait
`crate::applet::Applet` :

| Méthode | Rôle |
| --- | --- |
| `nom()` | Le nom montré dans les paramètres, s'il diffère de celui du contexte (Énergie, pour l'Orchestre) |
| `largeur(cx, c, h, compact)` | La largeur voulue dans la barre de hauteur `h` ; 0 retire l'applet (rien à montrer pour l'instant). `compact` : la barre manque de place, l'essentiel seulement |
| `barre(cx, c, r, id, compact)` | Dessine dans la barre : icône, texte court, vumètre… Ses propres commandes prennent des identifiants `sub_id(id, …)` et passent devant le toucher qui ouvre le volet |
| `hauteur_volet(cx, c, w)` | La hauteur voulue du volet (0 : pas de volet) ; il défile si elle ne tient pas |
| `volet(cx, c, r, id)` | Dessine le volet ; renvoie vrai pour le refermer |
| `anime(c)` | Vrai s'il faut redessiner sans événement (un vumètre, un avancement) |

Les applets se rangent à droite de la barre, avant l'heure, dans l'ordre de
`GENRES`. S'ils ne tiennent pas tous, ils passent en compact, et la priorité va aux
plus à droite (l'Énergie, près de l'heure) ; ceux qui ne tiennent toujours pas sont
laissés de côté. Toucher un
applet ouvre son volet au-dessus de la barre, toucher ailleurs le referme. Un volet
peut demander à montrer son contexte (`Demande::Montrer { cle }`) : si aucune
fenêtre ne le montre, la plus grande se scinde pour lui faire place.

Le choix des applets montrés se fait dans les paramètres (Interface, section Barre)
et s'enregistre dans `interface.txt` : `applets = fichiers radio mixeur orchestre` (vide :
aucun ; les clés inconnues sont oubliées).

| Applet | Dans la barre | Dans le volet |
| --- | --- | --- |
| Fichiers | Seulement pendant une copie, un déplacement ou une suppression : le pourcentage et un filet d'avancement | Le détail, Arrêter, Ouvrir les Fichiers |
| Radio | Le poste (à l'accent pendant la lecture), le flux en cours, lecture ou arrêt (relance le dernier flux) ; en compact, le poste seul | L'état et le format, le volume de la piste Radio, les flux récents, Ouvrir la Radio |
| Mixeur | L'icône (pâle si le général est à zéro) et le vumètre stéréo de la sortie | Volume, Muet et Égaliseur (allumé ou non) de chaque piste, volume général, égaliseur de la sortie, Ouvrir le Mixeur |
| Énergie (Orchestre) | La batterie (rouge sous le seuil bas) et, au choix, le niveau, le temps restant (avant la batterie vide, ou avant la charge complète) ou les deux ; le niveau tant que le temps n'est pas connu, et seul en compact | La courbe du jour, niveau, débit, autonomie, intervalle des relevés, seuil bas, le choix de l'affichage, l'économie, Ouvrir l'Orchestre |

## À venir

- Argile : des pinceaux qui affinent la matière là où ils passent (topologie
  dynamique), la peinture de la terre, des formes de départ autres que la boule.
- SVG : dégradés et motifs rendus fidèlement,
  sélection de plusieurs formes, entrer dans un groupe.
