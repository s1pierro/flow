//! Le tampon d'un éditeur de texte : les caractères, le curseur, et le découpage en
//! lignes à l'écran (retour à la ligne automatique, au mot si possible). Rien ici ne
//! dessine : tout se teste sur n'importe quelle machine.

use crate::clavier::Frappe;

/// Un texte en cours d'édition, sur plusieurs lignes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tampon {
    /// Les caractères ; les fins de ligne sont des `'\n'`.
    pub car: Vec<char>,
    /// Le curseur, en caractères (0 à `car.len()`).
    pub curseur: usize,
    /// Augmente à chaque modification du texte (pour les caches).
    pub revision: u64,
    /// L'autre bout de la sélection, s'il y en a une : elle va de l'ancre au curseur.
    pub ancre: Option<usize>,
}

impl Tampon {
    pub fn new(texte: &str) -> Tampon {
        Tampon { car: texte.chars().collect(), curseur: 0, revision: 0, ancre: None }
    }

    pub fn texte(&self) -> String {
        self.car.iter().collect()
    }

    /// Le nombre de lignes du texte (au moins une).
    pub fn lignes(&self) -> usize {
        1 + self.car.iter().filter(|c| **c == '\n').count()
    }

    /// La ligne et la colonne du curseur, à partir de 1.
    pub fn position(&self) -> (usize, usize) {
        let avant = &self.car[..self.curseur.min(self.car.len())];
        let ligne = 1 + avant.iter().filter(|c| **c == '\n').count();
        let colonne = 1 + avant.iter().rev().take_while(|c| **c != '\n').count();
        (ligne, colonne)
    }

    /// La sélection, du premier au dernier caractère (exclu), si elle n'est pas vide.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let n = self.car.len();
        let (a, b) = (self.ancre?.min(n), self.curseur.min(n));
        (a != b).then(|| (a.min(b), a.max(b)))
    }

    pub fn texte_selection(&self) -> Option<String> {
        self.selection().map(|(a, b)| self.car[a..b].iter().collect())
    }

    /// Sélectionne de `a` à `b` (le curseur va en `b`).
    pub fn selectionner(&mut self, a: usize, b: usize) {
        let n = self.car.len();
        self.ancre = Some(a.min(n));
        self.curseur = b.min(n);
    }

    pub fn tout_selectionner(&mut self) {
        self.selectionner(0, self.car.len());
    }

    /// Place le curseur en `i`, sans sélection.
    pub fn placer(&mut self, i: usize) {
        self.curseur = i.min(self.car.len());
        self.ancre = None;
    }

    /// Le mot autour du caractère `i` : lettres, chiffres et `_` ; sinon une suite
    /// d'espaces, ou le caractère seul. Vide au bout d'une ligne.
    pub fn mot_en(&self, i: usize) -> (usize, usize) {
        let n = self.car.len();
        let i = i.min(n);
        let sorte = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c == ' ' || c == '\t' { 1 } else if c == '\n' { 3 } else { 2 };
        // Au bout d'une ligne (ou du texte), c'est le dernier mot
        let k = if (i == n || sorte(self.car[i]) == 3) && i > 0 && sorte(self.car[i - 1]) != 3 { i - 1 } else { i };
        if k >= n || sorte(self.car[k]) == 3 {
            return (i, i);
        }
        let s = sorte(self.car[k]);
        if s == 2 {
            return (k, k + 1);
        }
        let mut a = k;
        while a > 0 && sorte(self.car[a - 1]) == s {
            a -= 1;
        }
        let mut b = k + 1;
        while b < n && sorte(self.car[b]) == s {
            b += 1;
        }
        (a, b)
    }

    /// Efface la sélection. Vrai s'il y en avait une.
    pub fn effacer_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            self.ancre = None;
            return false;
        };
        self.car.drain(a..b);
        self.curseur = a;
        self.ancre = None;
        self.revision += 1;
        true
    }

    /// Écrit un texte au curseur, à la place de la sélection s'il y en a une.
    pub fn inserer(&mut self, texte: &str) -> bool {
        let efface = self.effacer_selection();
        self.curseur = self.curseur.min(self.car.len());
        let v: Vec<char> = texte.chars().filter(|c| *c != '\r').collect();
        if v.is_empty() {
            return efface;
        }
        let n = v.len();
        self.car.splice(self.curseur..self.curseur, v);
        self.curseur += n;
        self.revision += 1;
        true
    }

    /// Applique une frappe. Renvoie vrai si le texte a changé. Une frappe remplace la
    /// sélection ; Effacer l'efface ; les flèches la quittent par le bord voulu.
    pub fn appliquer(&mut self, f: Frappe) -> bool {
        self.curseur = self.curseur.min(self.car.len());
        if let Some((a, b)) = self.selection() {
            match f {
                Frappe::Gauche | Frappe::Droite => {
                    self.placer(if f == Frappe::Gauche { a } else { b });
                    return false;
                }
                Frappe::Effacer => return self.effacer_selection(),
                _ => {
                    self.effacer_selection();
                    self.appliquer(f);
                    return true;
                }
            }
        }
        self.ancre = None;
        let change = match f {
            Frappe::Car(c) => {
                self.car.insert(self.curseur, c);
                self.curseur += 1;
                true
            }
            Frappe::Entree | Frappe::Ligne => {
                self.car.insert(self.curseur, '\n');
                self.curseur += 1;
                true
            }
            Frappe::Effacer if self.curseur > 0 => {
                self.curseur -= 1;
                self.car.remove(self.curseur);
                true
            }
            Frappe::Effacer => false,
            Frappe::Gauche => {
                self.curseur = self.curseur.saturating_sub(1);
                false
            }
            Frappe::Droite => {
                self.curseur = (self.curseur + 1).min(self.car.len());
                false
            }
        };
        if change {
            self.revision += 1;
        }
        change
    }
}

/// Une ligne à l'écran : ses caractères `debut..fin` dans le tampon (sans le `'\n'`).
pub type Segment = (usize, usize);

/// Découpe le texte en lignes d'au plus `colonnes` caractères. Une ligne trop longue
/// se coupe après la dernière espace qui tient, ou au ras de la largeur s'il n'y en a
/// pas. Il y a toujours au moins une ligne.
pub fn couper(car: &[char], colonnes: usize) -> Vec<Segment> {
    let colonnes = colonnes.max(1);
    let mut out = Vec::new();
    let mut debut = 0;
    loop {
        let fin = car[debut..].iter().position(|c| *c == '\n').map_or(car.len(), |i| debut + i);
        let mut pos = debut;
        while fin - pos > colonnes {
            // La dernière espace qui tient : la ligne se coupe juste après
            let coupe = (pos + 1..=pos + colonnes).rev().find(|&k| car[k - 1] == ' ').unwrap_or(pos + colonnes);
            out.push((pos, coupe));
            pos = coupe;
        }
        out.push((pos, fin));
        if fin >= car.len() {
            return out;
        }
        debut = fin + 1;
    }
}

/// La ligne à l'écran où se trouve le caractère `i` (le curseur) : la dernière qui
/// commence avant lui.
pub fn ligne_de(segments: &[Segment], i: usize) -> usize {
    segments.partition_point(|s| s.0 <= i).saturating_sub(1)
}

/// Le caractère sous la colonne `colonne` de la ligne `ligne` à l'écran.
pub fn position_en(segments: &[Segment], ligne: usize, colonne: usize) -> usize {
    let Some(&(debut, fin)) = segments.get(ligne.min(segments.len().saturating_sub(1))) else { return 0 };
    (debut + colonne).min(fin)
}

/// La ligne du texte (à partir de 0) de chaque ligne à l'écran.
pub fn logiques(car: &[char], segments: &[Segment]) -> Vec<usize> {
    let mut out = Vec::with_capacity(segments.len());
    let mut l = 0;
    for (k, s) in segments.iter().enumerate() {
        if k > 0 {
            let p = segments[k - 1].1;
            if s.0 == p + 1 && car.get(p) == Some(&'\n') {
                l += 1;
            }
        }
        out.push(l);
    }
    out
}

/// Les blocs entre accolades qui tiennent sur plusieurs lignes : la ligne de
/// l'accolade ouvrante et celle de la fermante, par ligne de départ croissante (pour
/// une ligne qui en ouvre plusieurs, le plus grand). Les accolades des chaînes, des
/// caractères (`'{'`) et des commentaires (`//`, `/* */`) ne comptent pas.
pub fn blocs(car: &[char]) -> Vec<(usize, usize)> {
    let mut pile: Vec<usize> = Vec::new();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let (mut i, mut ligne) = (0, 0);
    while i < car.len() {
        let c = car[i];
        match c {
            '\n' => ligne += 1,
            '/' if car.get(i + 1) == Some(&'/') => {
                while i < car.len() && car[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if car.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < car.len() && !(car[i] == '*' && car.get(i + 1) == Some(&'/')) {
                    if car[i] == '\n' {
                        ligne += 1;
                    }
                    i += 1;
                }
                i += 2;
                continue;
            }
            '"' => {
                i += 1;
                while i < car.len() && car[i] != '"' {
                    if car[i] == '\\' {
                        i += 1;
                    }
                    if car.get(i) == Some(&'\n') {
                        ligne += 1;
                    }
                    i += 1;
                }
            }
            // Un caractère : 'x' ou '\x' (pas une durée de vie, 'a sans guillemet fermant)
            '\'' if car.get(i + 2) == Some(&'\'') && car.get(i + 1) != Some(&'\\') => i += 2,
            '\'' if car.get(i + 1) == Some(&'\\') && car.get(i + 3) == Some(&'\'') => i += 3,
            '{' => pile.push(ligne),
            '}' => {
                if let Some(debut) = pile.pop() {
                    if ligne > debut {
                        match out.iter_mut().find(|b| b.0 == debut) {
                            Some(b) => b.1 = b.1.max(ligne),
                            None => out.push((debut, ligne)),
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoupe(t: &str, n: usize) -> Vec<String> {
        let c: Vec<char> = t.chars().collect();
        couper(&c, n).into_iter().map(|(a, b)| c[a..b].iter().collect()).collect()
    }

    #[test]
    fn les_lignes_se_coupent_au_mot() {
        assert_eq!(decoupe("", 10), vec![""]);
        assert_eq!(decoupe("abc\n\nde", 10), vec!["abc", "", "de"]);
        assert_eq!(decoupe("abc\n", 10), vec!["abc", ""]);
        assert_eq!(decoupe("un deux trois quatre", 10), vec!["un deux ", "trois ", "quatre"]);
        // Sans espace, la coupe tombe au ras
        assert_eq!(decoupe("abcdefghijkl", 5), vec!["abcde", "fghij", "kl"]);
    }

    #[test]
    fn le_curseur_trouve_sa_ligne() {
        let c: Vec<char> = "un deux trois\nfin".chars().collect();
        let s = couper(&c, 8); // « un deux », « trois », « fin »
        assert_eq!(s, vec![(0, 8), (8, 13), (14, 17)]);
        assert_eq!(ligne_de(&s, 0), 0);
        // À la coupe d'un retour automatique, le curseur passe à la ligne suivante
        assert_eq!(ligne_de(&s, 8), 1);
        // Juste avant le '\n', il reste au bout de sa ligne
        assert_eq!(ligne_de(&s, 13), 1);
        assert_eq!(ligne_de(&s, 17), 2);
        assert_eq!(position_en(&s, 1, 2), 10);
        assert_eq!(position_en(&s, 1, 99), 13);
        assert_eq!(position_en(&s, 9, 0), 14);
    }

    #[test]
    fn les_blocs_entre_accolades() {
        let t: Vec<char> = "fn a() {\n    let s = \"{\";\n    if x { y }\n    // {\n    let c = '{';\n    if b {\n        z\n    } else {\n        w\n    }\n}\n".chars().collect();
        assert_eq!(blocs(&t), vec![(0, 10), (5, 7), (7, 9)]);
        let s = couper(&t, 8);
        let l = logiques(&t, &s);
        assert_eq!(l.len(), s.len());
        assert_eq!(*l.last().unwrap(), 11);
        assert!(l.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1));
    }

    #[test]
    fn les_frappes_editent_le_texte() {
        let mut t = Tampon::new("ab");
        t.curseur = 2;
        assert!(t.appliquer(Frappe::Entree));
        t.appliquer(Frappe::Car('é'));
        assert_eq!(t.texte(), "ab\né");
        assert_eq!(t.position(), (2, 2));
        assert_eq!(t.lignes(), 2);
        assert!(!t.appliquer(Frappe::Gauche));
        assert!(t.appliquer(Frappe::Effacer));
        assert_eq!(t.texte(), "abé");
        assert_eq!(t.revision, 3);
        t.curseur = 0;
        assert!(!t.appliquer(Frappe::Effacer));
    }

    #[test]
    fn la_selection_se_remplace_et_s_efface() {
        let mut t = Tampon::new("un deux_3 trois, fin\nsuite");
        assert_eq!(t.mot_en(4), (3, 9));
        // Sur une espace, les espaces ; au bout d'une ligne, le mot qui la finit
        assert_eq!(t.mot_en(9), (9, 10));
        assert_eq!(t.mot_en(15), (15, 16));
        assert_eq!(t.mot_en(1), (0, 2));
        assert_eq!(t.mot_en(2), (2, 3));
        assert_eq!(t.mot_en(20), (17, 20));
        let (a, b) = t.mot_en(4);
        t.selectionner(b, a);
        assert_eq!(t.texte_selection().as_deref(), Some("deux_3"));
        assert!(t.appliquer(Frappe::Car('X')));
        assert_eq!(t.texte(), "un X trois, fin\nsuite");
        assert_eq!((t.curseur, t.ancre), (4, None));
        t.selectionner(0, 3);
        assert!(t.appliquer(Frappe::Effacer));
        assert_eq!(t.texte(), "X trois, fin\nsuite");
        t.selectionner(2, 7);
        assert!(!t.appliquer(Frappe::Droite));
        assert_eq!((t.curseur, t.selection()), (7, None));
        t.selectionner(0, 1);
        assert!(t.inserer("a\r\nb"));
        assert_eq!(t.texte(), "a\nb trois, fin\nsuite");
        t.tout_selectionner();
        assert_eq!(t.selection(), Some((0, t.car.len())));
        // Une sélection vide n'en est pas une
        t.selectionner(3, 3);
        assert_eq!(t.selection(), None);
    }
}
