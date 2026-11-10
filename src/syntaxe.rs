//! La coloration syntaxique, pilotée par des dictionnaires en texte : un fichier par
//! langage dans `Flow/syntaxes/` (`rust.txt`, `html.txt`…), en paires `clé = valeur`.
//! Ceux de Flow sont intégrés à l'application (dossier `syntaxes/` du projet) et
//! recopiés là s'ils n'y sont pas : on peut les retoucher, ou ajouter un langage en
//! déposant un nouveau fichier. Rien ici ne dessine.
//!
//! Deux sortes de langages :
//!
//! - le **code** : commentaires, chaînes, nombres, mots-clés, types, constantes,
//!   fonctions (un mot suivi de `(`), macros (`mot!`), annotations (`#[…]`),
//!   directives du préprocesseur (`#include`), propriétés CSS ;
//! - le **balisage** (`balisage = oui`) : balises, attributs, valeurs, entités,
//!   commentaires, et le contenu de certaines balises coloré avec un autre langage
//!   (`incorpore = script:js style:css`).
//!
//! L'analyse rend des plages `(début, fin, Classe)` en caractères, croissantes et
//! disjointes ; le reste du texte garde la couleur ordinaire.

use std::collections::HashMap;
use std::path::Path;

/// Ce que désigne une plage colorée.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classe {
    MotCle,
    Type,
    Constante,
    Chaine,
    Commentaire,
    Nombre,
    Fonction,
    Macro,
    Balise,
    Attribut,
    Preprocesseur,
}

/// Une plage colorée : caractères `début..fin`.
pub type Plage = (usize, usize, Classe);

/// Les dictionnaires livrés avec Flow : clé et contenu.
pub const INTEGRES: [(&str, &str); 7] = [
    ("rust", include_str!("../syntaxes/rust.txt")),
    ("c", include_str!("../syntaxes/c.txt")),
    ("cpp", include_str!("../syntaxes/cpp.txt")),
    ("js", include_str!("../syntaxes/js.txt")),
    ("css", include_str!("../syntaxes/css.txt")),
    ("html", include_str!("../syntaxes/html.txt")),
    ("svg", include_str!("../syntaxes/svg.txt")),
];

/// Le dossier des dictionnaires, dans celui de Flow.
pub const DOSSIER: &str = "syntaxes";

/// Un langage, tel que le décrit son dictionnaire.
#[derive(Clone, Debug, Default)]
pub struct Syntaxe {
    /// La clé (le nom du fichier sans `.txt`).
    pub cle: String,
    pub nom: String,
    pub extensions: Vec<String>,
    commentaire_ligne: Vec<Vec<char>>,
    commentaire_bloc: Option<(Vec<char>, Vec<char>)>,
    chaines: Vec<char>,
    echappement: Option<char>,
    caracteres: bool,
    annotations: Vec<Vec<char>>,
    preprocesseur: Option<char>,
    prefixe_mot_cle: Option<char>,
    diese: bool,
    macros: bool,
    majuscules_types: bool,
    tirets: bool,
    proprietes: bool,
    balisage: bool,
    incorpore: Vec<(String, String)>,
    mots: HashMap<String, Classe>,
}

fn oui(v: &str) -> bool {
    matches!(v.trim().to_lowercase().as_str(), "oui" | "vrai" | "1" | "yes" | "true")
}

impl Syntaxe {
    /// Lit un dictionnaire. Une clé inconnue est ignorée ; une liste répétée s'allonge.
    pub fn lire(cle: &str, texte: &str) -> Syntaxe {
        let mut s = Syntaxe { cle: cle.to_string(), nom: cle.to_string(), ..Syntaxe::default() };
        for l in texte.lines() {
            if l.trim_start().starts_with('#') {
                continue;
            }
            let Some((k, v)) = l.split_once('=') else { continue };
            let v = v.trim();
            let liste = || v.split_whitespace().map(str::to_string);
            let premier = v.chars().next();
            match k.trim() {
                "nom" => s.nom = v.to_string(),
                "extensions" => s.extensions.extend(liste().map(|e| e.trim_start_matches('.').to_lowercase())),
                "commentaire_ligne" => s.commentaire_ligne.extend(liste().map(|m| m.chars().collect())),
                "commentaire_bloc" => {
                    let mut it = v.split_whitespace();
                    if let (Some(a), Some(b)) = (it.next(), it.next()) {
                        s.commentaire_bloc = Some((a.chars().collect(), b.chars().collect()));
                    }
                }
                "chaines" => s.chaines.extend(liste().filter_map(|m| m.chars().next())),
                "echappement" => s.echappement = premier,
                "caracteres" => s.caracteres = oui(v),
                "annotations" => s.annotations.extend(liste().map(|m| m.chars().collect())),
                "preprocesseur" => s.preprocesseur = premier,
                "prefixe_mot_cle" => s.prefixe_mot_cle = premier,
                "diese" => s.diese = oui(v),
                "macros" => s.macros = oui(v),
                "majuscules_types" => s.majuscules_types = oui(v),
                "tirets" => s.tirets = oui(v),
                "proprietes" => s.proprietes = oui(v),
                "balisage" => s.balisage = oui(v),
                "incorpore" => s.incorpore.extend(liste().filter_map(|m| m.split_once(':').map(|(a, b)| (a.to_lowercase(), b.to_string())))),
                "mots_cles" => s.mots.extend(liste().map(|m| (m, Classe::MotCle))),
                "types" => s.mots.extend(liste().map(|m| (m, Classe::Type))),
                "constantes" => s.mots.extend(liste().map(|m| (m, Classe::Constante))),
                _ => {}
            }
        }
        s
    }
}

/// Les langages connus.
#[derive(Clone, Debug, Default)]
pub struct Bibliotheque {
    pub langues: Vec<Syntaxe>,
}

impl Bibliotheque {
    /// Les dictionnaires intégrés seuls.
    pub fn integree() -> Bibliotheque {
        Bibliotheque { langues: INTEGRES.iter().map(|(k, t)| Syntaxe::lire(k, t)).collect() }
    }

    /// Les dictionnaires du dossier `dossier` : ceux de Flow y sont recopiés s'ils
    /// manquent, ceux qui s'y trouvent priment, et tout autre `.txt` ajoute un langage.
    pub fn charger(dossier: Option<&Path>) -> Bibliotheque {
        let Some(d) = dossier else { return Bibliotheque::integree() };
        let _ = std::fs::create_dir_all(d);
        let mut langues = Vec::new();
        for (cle, texte) in INTEGRES {
            let f = d.join(format!("{cle}.txt"));
            let lu = std::fs::read_to_string(&f).ok();
            if lu.is_none() {
                if let Err(e) = std::fs::write(&f, texte) {
                    crate::android::log(&format!("dictionnaire {cle} non recopié : {e}"));
                }
            }
            langues.push(Syntaxe::lire(cle, lu.as_deref().unwrap_or(texte)));
        }
        if let Ok(it) = std::fs::read_dir(d) {
            let mut autres: Vec<_> = it.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect();
            autres.sort();
            for p in autres {
                let Some(cle) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
                if langues.iter().any(|l| l.cle == cle) {
                    continue;
                }
                if let Ok(t) = std::fs::read_to_string(&p) {
                    langues.push(Syntaxe::lire(&cle, &t));
                }
            }
        }
        Bibliotheque { langues }
    }

    pub fn par_cle(&self, cle: &str) -> Option<&Syntaxe> {
        self.langues.iter().find(|l| l.cle == cle)
    }

    /// Le langage d'un fichier, d'après son extension.
    pub fn pour(&self, nom: &str) -> Option<&Syntaxe> {
        let ext = nom.rsplit_once('.').filter(|(a, _)| !a.is_empty())?.1.to_lowercase();
        self.langues.iter().find(|l| l.extensions.contains(&ext))
    }

    /// Les plages colorées de `car` dans le langage `s`.
    pub fn colorer(&self, car: &[char], s: &Syntaxe) -> Vec<Plage> {
        let mut out = Vec::new();
        if s.balisage {
            balisage(car, 0, car.len(), s, self, &mut out);
        } else {
            code(car, 0, car.len(), s, &mut out);
        }
        out
    }
}

fn commence(car: &[char], i: usize, m: &[char]) -> bool {
    !m.is_empty() && car.len() >= i + m.len() && car[i..i + m.len()] == *m
}

/// Comme `commence`, sans tenir compte de la casse (ASCII).
fn commence_i(car: &[char], i: usize, m: &str) -> bool {
    let n = m.chars().count();
    car.len() >= i + n && car[i..i + n].iter().zip(m.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b))
}

fn mot(c: char, tirets: bool) -> bool {
    c.is_alphanumeric() || c == '_' || (tirets && c == '-')
}

/// Le prochain caractère qui n'est pas une espace, à partir de `i`, avant `fin`.
fn suivant(car: &[char], mut i: usize, fin: usize) -> Option<char> {
    while i < fin && (car[i] == ' ' || car[i] == '\t') {
        i += 1;
    }
    (i < fin).then(|| car[i])
}

/// Après un mot suivi de « : », vrai si c'est une déclaration (`color: red;`) et non
/// un sélecteur (`a:hover {`) : la suite se termine par `;` ou `}` avant un `{`.
fn declaration(car: &[char], j: usize, fin: usize) -> bool {
    car[j..fin].iter().find(|&&c| matches!(c, ';' | '}' | '{')).is_none_or(|&c| c != '{')
}

/// Une chaîne qui commence en `i` (sur son délimiteur) : la position après sa fin.
fn chaine(car: &[char], i: usize, fin: usize, delim: char, echappement: Option<char>) -> usize {
    let mut j = i + 1;
    while j < fin && car[j] != delim {
        if Some(car[j]) == echappement {
            j += 1;
        }
        j += 1;
    }
    (j + 1).min(fin)
}

/// Le code de `debut..fin`.
fn code(car: &[char], debut: usize, fin: usize, s: &Syntaxe, out: &mut Vec<Plage>) {
    let mut i = debut;
    let mut profondeur = 0i32;
    let mut debut_de_ligne = true;
    while i < fin {
        let c = car[i];
        if c == '\n' {
            debut_de_ligne = true;
            i += 1;
            continue;
        }
        // Rien d'autre que des espaces avant, sur la ligne
        let ligne_vierge = debut_de_ligne;
        if c != ' ' && c != '\t' {
            debut_de_ligne = false;
        }
        let precede_mot = i > debut && mot(car[i - 1], s.tirets);
        // Commentaires
        if s.commentaire_ligne.iter().any(|m| commence(car, i, m)) {
            let j = car[i..fin].iter().position(|&x| x == '\n').map_or(fin, |k| i + k);
            out.push((i, j, Classe::Commentaire));
            i = j;
            continue;
        }
        if let Some((a, b)) = &s.commentaire_bloc {
            if commence(car, i, a) {
                let mut j = i + a.len();
                while j < fin && !commence(car, j, b) {
                    j += 1;
                }
                let j = (j + b.len()).min(fin);
                out.push((i, j, Classe::Commentaire));
                i = j;
                continue;
            }
        }
        // Préprocesseur : toute la ligne (et ses suites après « \ »)
        if s.preprocesseur == Some(c) && (ligne_vierge || i == debut) {
            let mut j = i;
            while j < fin && !(car[j] == '\n' && (j == 0 || car[j - 1] != '\\')) {
                j += 1;
            }
            out.push((i, j, Classe::Preprocesseur));
            i = j;
            continue;
        }
        // Annotations : jusqu'au crochet qui ferme
        if let Some(m) = s.annotations.iter().find(|m| commence(car, i, m)) {
            let mut j = i + m.len();
            let mut p = 1;
            while j < fin && p > 0 {
                match car[j] {
                    '[' => p += 1,
                    ']' => p -= 1,
                    '"' => j = chaine(car, j, fin, '"', s.echappement) - 1,
                    _ => {}
                }
                j += 1;
            }
            out.push((i, j, Classe::Preprocesseur));
            i = j;
            continue;
        }
        // Chaînes, caractères
        if s.chaines.contains(&c) {
            let j = chaine(car, i, fin, c, s.echappement);
            out.push((i, j, Classe::Chaine));
            i = j;
            continue;
        }
        if s.caracteres && c == '\'' {
            let j = if car.get(i + 2) == Some(&'\'') && car.get(i + 1) != Some(&'\\') {
                Some(i + 3)
            } else if car.get(i + 1) == Some(&'\\') {
                // '\n', '\'', '\u{1F600}'
                (i + 3..fin.min(i + 12)).find(|&k| car[k] == '\'').map(|k| k + 1)
            } else {
                None
            };
            if let Some(j) = j.filter(|&j| j <= fin) {
                out.push((i, j, Classe::Chaine));
                i = j;
                continue;
            }
        }
        // Nombres (pas au milieu d'un mot)
        if !precede_mot && (c.is_ascii_digit() || (c == '.' && car.get(i + 1).is_some_and(|d| d.is_ascii_digit()) && (i == debut || car[i - 1] != '.'))) {
            let mut j = i + 1;
            while j < fin && (car[j].is_ascii_alphanumeric() || car[j] == '_' || car[j] == '%' || (car[j] == '.' && car.get(j + 1) != Some(&'.') && car.get(j + 1).is_some_and(|d| d.is_ascii_digit()))) {
                j += 1;
            }
            out.push((i, j, Classe::Nombre));
            i = j;
            continue;
        }
        // @media, #3b82f6, !important
        let prefixe = (s.prefixe_mot_cle == Some(c) && car.get(i + 1).is_some_and(|&x| x.is_alphabetic())).then_some(Classe::MotCle).or_else(|| (s.diese && c == '#' && car.get(i + 1).is_some_and(|&x| x.is_alphanumeric())).then_some(Classe::Constante));
        if let Some(classe) = prefixe {
            let mut j = i + 1;
            while j < fin && mot(car[j], s.tirets) {
                j += 1;
            }
            out.push((i, j, classe));
            i = j;
            continue;
        }
        if c == '!' && car.get(i + 1).is_some_and(|&x| x.is_alphabetic()) {
            let mut j = i + 1;
            while j < fin && mot(car[j], s.tirets) {
                j += 1;
            }
            let m: String = car[i..j].iter().collect();
            if let Some(&classe) = s.mots.get(&m) {
                out.push((i, j, classe));
                i = j;
                continue;
            }
        }
        // Les mots
        if (c.is_alphabetic() || c == '_' || (s.tirets && c == '-' && car.get(i + 1).is_some_and(|&x| x.is_alphabetic() || x == '-'))) && !precede_mot {
            let mut j = i + 1;
            while j < fin && mot(car[j], s.tirets) {
                j += 1;
            }
            let m: String = car[i..j].iter().collect();
            let apres = suivant(car, j, fin);
            let classe = if let Some(&k) = s.mots.get(&m) {
                Some(k)
            } else if s.macros && car.get(j) == Some(&'!') && car.get(j + 1) != Some(&'=') {
                out.push((i, j + 1, Classe::Macro));
                i = j + 1;
                continue;
            } else if s.proprietes && profondeur > 0 && apres == Some(':') && declaration(car, j, fin) {
                Some(Classe::Attribut)
            } else if apres == Some('(') {
                Some(Classe::Fonction)
            } else if s.majuscules_types && c.is_uppercase() {
                Some(Classe::Type)
            } else {
                None
            };
            if let Some(k) = classe {
                out.push((i, j, k));
            }
            i = j;
            continue;
        }
        match c {
            '{' => profondeur += 1,
            '}' => profondeur -= 1,
            _ => {}
        }
        i += 1;
    }
}

/// Le balisage de `debut..fin`.
fn balisage(car: &[char], debut: usize, fin: usize, s: &Syntaxe, bib: &Bibliotheque, out: &mut Vec<Plage>) {
    let nom_car = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.');
    let jusqua = |i: usize, m: &str| -> usize {
        let m: Vec<char> = m.chars().collect();
        let mut j = i;
        while j < fin && !commence(car, j, &m) {
            j += 1;
        }
        (j + m.len()).min(fin)
    };
    let mut i = debut;
    while i < fin {
        let c = car[i];
        if commence_i(car, i, "<!--") {
            let j = jusqua(i + 4, "-->");
            out.push((i, j, Classe::Commentaire));
            i = j;
        } else if commence_i(car, i, "<![CDATA[") {
            let j = jusqua(i + 9, "]]>");
            out.push((i, j, Classe::Chaine));
            i = j;
        } else if c == '<' && matches!(car.get(i + 1), Some('!') | Some('?')) {
            let j = jusqua(i + 2, ">");
            out.push((i, j, Classe::Preprocesseur));
            i = j;
        } else if c == '<' && car.get(i + 1).is_some_and(|&x| x == '/' || x.is_alphabetic()) {
            // Une balise : son nom, ses attributs et leurs valeurs
            let fermante = car[i + 1] == '/';
            let mut j = i + 1 + fermante as usize;
            let n0 = j;
            while j < fin && nom_car(car[j]) {
                j += 1;
            }
            let nom: String = car[n0..j].iter().collect::<String>().to_lowercase();
            out.push((i, j, Classe::Balise));
            let mut auto = false;
            while j < fin {
                let x = car[j];
                if x == '>' {
                    out.push((j, j + 1, Classe::Balise));
                    j += 1;
                    break;
                } else if x == '/' && car.get(j + 1) == Some(&'>') {
                    out.push((j, j + 2, Classe::Balise));
                    j += 2;
                    auto = true;
                    break;
                } else if x == '"' || x == '\'' {
                    let k = chaine(car, j, fin, x, None);
                    out.push((j, k, Classe::Chaine));
                    j = k;
                } else if nom_car(x) {
                    let k0 = j;
                    while j < fin && nom_car(car[j]) {
                        j += 1;
                    }
                    out.push((k0, j, Classe::Attribut));
                } else {
                    j += 1;
                }
            }
            i = j;
            // Le contenu d'une balise d'un autre langage
            if !fermante && !auto {
                if let Some(langue) = s.incorpore.iter().find(|(b, _)| *b == nom).and_then(|(_, k)| bib.par_cle(k)) {
                    let fermeture = format!("</{nom}");
                    let mut k = i;
                    while k < fin && !commence_i(car, k, &fermeture) {
                        k += 1;
                    }
                    if langue.balisage {
                        balisage(car, i, k, langue, bib, out);
                    } else {
                        code(car, i, k, langue, out);
                    }
                    i = k;
                }
            }
        } else if c == '&' {
            let k = (i + 1..fin.min(i + 12)).find(|&k| !(car[k].is_alphanumeric() || car[k] == '#'));
            match k {
                Some(k) if car[k] == ';' && k > i + 1 => {
                    out.push((i, k + 1, Classe::Constante));
                    i = k + 1;
                }
                _ => i += 1,
            }
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plages(cle: &str, t: &str) -> Vec<(String, Classe)> {
        let b = Bibliotheque::integree();
        let car: Vec<char> = t.chars().collect();
        let p = b.colorer(&car, b.par_cle(cle).unwrap());
        // Croissantes et disjointes
        assert!(p.windows(2).all(|w| w[0].1 <= w[1].0), "{p:?}");
        p.into_iter().map(|(a, e, k)| (car[a..e].iter().collect(), k)).collect()
    }

    fn a(m: &str, k: Classe) -> (String, Classe) {
        (m.to_string(), k)
    }

    #[test]
    fn les_langages_se_trouvent_par_extension() {
        let b = Bibliotheque::integree();
        assert_eq!(b.langues.len(), 7);
        assert_eq!(b.pour("main.RS").unwrap().nom, "Rust");
        assert_eq!(b.pour("x.hpp").unwrap().cle, "cpp");
        assert_eq!(b.pour("dessin.svg").unwrap().cle, "svg");
        assert!(b.pour("LISEZMOI").is_none());
        assert!(b.pour(".rs").is_none());
    }

    #[test]
    fn rust() {
        let p = plages("rust", "#[derive(Debug)]\npub fn a<'b>(x: &'b str) -> Option<u8> {\n    // note\n    let c = '{'; println!(\"{x}\\\"\"); Some(0x1F)\n}");
        assert_eq!(
            p,
            vec![
                a("#[derive(Debug)]", Classe::Preprocesseur),
                a("pub", Classe::MotCle),
                a("fn", Classe::MotCle),
                a("str", Classe::Type),
                a("Option", Classe::Type),
                a("u8", Classe::Type),
                a("// note", Classe::Commentaire),
                a("let", Classe::MotCle),
                a("'{'", Classe::Chaine),
                a("println!", Classe::Macro),
                a("\"{x}\\\"\"", Classe::Chaine),
                a("Some", Classe::Constante),
                a("0x1F", Classe::Nombre),
            ]
        );
    }

    #[test]
    fn c_et_cpp() {
        let p = plages("c", "#include <stdio.h>\nint main(void) { return 0; } /* fin */");
        assert_eq!(p, vec![a("#include <stdio.h>", Classe::Preprocesseur), a("int", Classe::Type), a("main", Classe::Fonction), a("void", Classe::Type), a("return", Classe::MotCle), a("0", Classe::Nombre), a("/* fin */", Classe::Commentaire)]);
        let p = plages("cpp", "  #define X 1\nauto p = nullptr;");
        assert_eq!(p, vec![a("#define X 1", Classe::Preprocesseur), a("auto", Classe::MotCle), a("nullptr", Classe::Constante)]);
    }

    #[test]
    fn javascript_et_css() {
        let p = plages("js", "const s = `a\nb`; Math.max(1.5, x)");
        assert_eq!(p, vec![a("const", Classe::MotCle), a("`a\nb`", Classe::Chaine), a("Math", Classe::Type), a("max", Classe::Fonction), a("1.5", Classe::Nombre)]);
        let p = plages("css", "@media (min-width: 600px) { a:hover { font-size: 12px !important; color: #3b82f6; } }");
        assert_eq!(
            p,
            vec![
                a("@media", Classe::MotCle),
                a("600px", Classe::Nombre),
                a("font-size", Classe::Attribut),
                a("12px", Classe::Nombre),
                a("!important", Classe::MotCle),
                a("color", Classe::Attribut),
                a("#3b82f6", Classe::Constante),
            ]
        );
    }

    #[test]
    fn html_avec_script_et_style() {
        let p = plages("html", "<!-- x --><p class=\"a\">&amp;</p><script>let n = 1;</script><style>b { color: red }</style>");
        assert_eq!(
            p,
            vec![
                a("<!-- x -->", Classe::Commentaire),
                a("<p", Classe::Balise),
                a("class", Classe::Attribut),
                a("\"a\"", Classe::Chaine),
                a(">", Classe::Balise),
                a("&amp;", Classe::Constante),
                a("</p", Classe::Balise),
                a(">", Classe::Balise),
                a("<script", Classe::Balise),
                a(">", Classe::Balise),
                a("let", Classe::MotCle),
                a("1", Classe::Nombre),
                a("</script", Classe::Balise),
                a(">", Classe::Balise),
                a("<style", Classe::Balise),
                a(">", Classe::Balise),
                a("color", Classe::Attribut),
                a("</style", Classe::Balise),
                a(">", Classe::Balise),
            ]
        );
        let p = plages("svg", "<?xml version=\"1.0\"?><path d=\"M0 0\"/>");
        assert_eq!(p, vec![a("<?xml version=\"1.0\"?>", Classe::Preprocesseur), a("<path", Classe::Balise), a("d", Classe::Attribut), a("\"M0 0\"", Classe::Chaine), a("/>", Classe::Balise)]);
    }

    #[test]
    fn le_dossier_prime_et_ajoute_des_langages() {
        let d = std::env::temp_dir().join(format!("flow-syntaxes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("rust.txt"), "nom = Rouille\nextensions = rs\nmots_cles = fn\n").unwrap();
        std::fs::write(d.join("toml.txt"), "nom = TOML\nextensions = toml\ncommentaire_ligne = #\n").unwrap();
        let b = Bibliotheque::charger(Some(&d));
        assert_eq!(b.pour("a.rs").unwrap().nom, "Rouille");
        assert_eq!(b.pour("Cargo.toml").unwrap().nom, "TOML");
        // Les dictionnaires manquants sont recopiés, et la retouche est gardée
        assert!(d.join("html.txt").exists());
        assert!(std::fs::read_to_string(d.join("rust.txt")).unwrap().contains("Rouille"));
        let _ = std::fs::remove_dir_all(d);
    }
}
