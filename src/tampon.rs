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
}

impl Tampon {
    pub fn new(texte: &str) -> Tampon {
        Tampon { car: texte.chars().collect(), curseur: 0, revision: 0 }
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

    /// Applique une frappe. Renvoie vrai si le texte a changé.
    pub fn appliquer(&mut self, f: Frappe) -> bool {
        self.curseur = self.curseur.min(self.car.len());
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
}
