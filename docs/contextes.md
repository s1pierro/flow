# Intégrer un contexte dans Flow

Flow est une *application d'exploitation* : l'écran se partage en fenêtres, et chaque
fenêtre montre un **contexte**, une sous-application (Fichiers, Disque, Radio…).
L'utilisateur compose son espace de travail en partageant les fenêtres et en
choisissant le contexte de chacune. Ce guide explique comment ajouter un contexte,
par exemple un éditeur de texte ou de SVG.

Les paramètres de l'application (apparence, clavier) ne sont pas un contexte : ils
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
| `src/contextes/mod.rs` | La liste `GENRES`, et des outils de liste partagés (`ligne_fichier`, `Puce`) |
| `src/contextes/<nom>.rs` | Un contexte |
| `src/espace.rs` | Le format de `espace.txt`, sa lecture et son écriture |
| `src/parametres.rs` | Le panneau des paramètres de l'application |
| `src/app.rs` | L'application : fenêtres, instances, panneau latéral, barre |

Les contextes existants servent d'exemples, du plus simple au plus complet :
`aide.rs` (rien à retenir), `radio.rs` (un champ de texte et le clavier, une
animation), `fichiers.rs` (un dossier par fenêtre, relu au retour), `disque.rs`
(une ressource partagée dans `Commun`, un état enregistré par noms et résolu plus
tard).

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
pub const GENRE: Genre = Genre { cle: CLE, nom: "Texte", icone: Icon::Fichier, ouvrir };

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
Le contexte apparaît alors dans le menu de chaque fenêtre. Le test
`les_cles_sont_uniques_et_valides` vérifie la clé.

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
| `geste(g, p, c)` | geste commencé dans la fenêtre | Glissement d'un doigt, déplacement et zoom à deux doigts (pour un plan de travail, un dessin) |
| `frappe(f, cible, frappe, c)` | frappe du clavier virtuel | Renvoie vrai si `cible` est un champ de cette instance |
| `retour(c)` | retour dans l'application | Relit ce qui a pu changer sur le disque |

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

## À venir

- Des contextes **Texte** et **SVG**, chacun éditant ses propres documents.
- Un changement de contexte détruit aujourd'hui l'instance et son état : un contexte
  à document devra proposer d'enregistrer avant.
