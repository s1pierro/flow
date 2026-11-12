//! L'égaliseur du mixeur : cinq bandes (une étagère grave, trois cloches, une étagère
//! aiguë), chacune réglée en fréquence, en gain (± 12 dB) et en largeur (Q). Il y en a
//! un par piste, appliqué à chaque voie de la piste avant son gain, et un sur la
//! sortie, avant le gain général et le limiteur.
//!
//! - Les filtres sont des biquads (formules de R. Bristow-Johnson, « Audio EQ
//!   Cookbook »), en forme directe II transposée, à la fréquence de la sortie.
//! - Les réglages passent de l'interface au rappel audio sans verrou (`Partage` : des
//!   valeurs atomiques et un numéro de version, relu avant et après) ; le rappel
//!   recalcule ses coefficients quand la version change (`Filtre::suivre`), sans rien
//!   allouer.
//! - Un égaliseur à plat (tous les gains à 0 dB) ou éteint ne coûte rien : le son le
//!   traverse sans calcul.

use std::sync::atomic::{AtomicU32, Ordering};

/// Le nombre de bandes.
pub const BANDES: usize = 5;
/// Les bornes des réglages.
pub const GAIN_MAX: f32 = 12.0;
pub const FREQ_MIN: f32 = 20.0;
pub const FREQ_MAX: f32 = 20_000.0;
pub const Q_MIN: f32 = 0.3;
pub const Q_MAX: f32 = 6.0;

/// La forme d'une bande.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Forme {
    /// Étagère grave : tout ce qui est sous la fréquence monte ou descend.
    Grave,
    /// Cloche : autour de la fréquence, sur une largeur qui dépend de Q.
    Cloche,
    /// Étagère aiguë : tout ce qui est au-dessus de la fréquence.
    Aigu,
}

/// La forme de chaque bande, de la plus grave à la plus aiguë.
pub const FORMES: [Forme; BANDES] = [Forme::Grave, Forme::Cloche, Forme::Cloche, Forme::Cloche, Forme::Aigu];

/// Une bande : fréquence (Hz), gain (dB), largeur (Q).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bande {
    pub frequence: f32,
    pub gain: f32,
    pub q: f32,
}

/// Les réglages d'un égaliseur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Egaliseur {
    pub actif: bool,
    pub bandes: [Bande; BANDES],
}

impl Default for Egaliseur {
    fn default() -> Egaliseur {
        let f = [80.0, 250.0, 1000.0, 4000.0, 12_000.0];
        let q = [0.707, 1.0, 1.0, 1.0, 0.707];
        Egaliseur { actif: true, bandes: std::array::from_fn(|i| Bande { frequence: f[i], gain: 0.0, q: q[i] }) }
    }
}

impl Egaliseur {
    /// Vrai si le son le traverse sans changer : éteint, ou tous les gains à 0 dB.
    pub fn neutre(&self) -> bool {
        !self.actif || self.bandes.iter().all(|b| b.gain.abs() < 0.05)
    }

    /// Tous les gains à 0 dB (fréquences et largeurs gardées).
    pub fn a_plat(&mut self) {
        for b in &mut self.bandes {
            b.gain = 0.0;
        }
    }

    /// Ramène chaque réglage dans ses bornes.
    pub fn borner(mut self) -> Egaliseur {
        for b in &mut self.bandes {
            b.frequence = if b.frequence.is_finite() { b.frequence.clamp(FREQ_MIN, FREQ_MAX) } else { 1000.0 };
            b.gain = if b.gain.is_finite() { b.gain.clamp(-GAIN_MAX, GAIN_MAX) } else { 0.0 };
            b.q = if b.q.is_finite() { b.q.clamp(Q_MIN, Q_MAX) } else { 1.0 };
        }
        self
    }

    /// La réponse en dB à la fréquence `f`, pour une sortie à `fs` (la courbe de
    /// l'interface).
    pub fn reponse(&self, f: f32, fs: f32) -> f32 {
        if !self.actif {
            return 0.0;
        }
        (0..BANDES).filter(|&i| self.bandes[i].gain.abs() >= 0.05).map(|i| coefs(FORMES[i], self.bandes[i], fs).reponse(f, fs)).sum()
    }

    /// Les réglages en texte : `actif` (0 ou 1), puis fréquence, gain et Q de chaque
    /// bande, séparés par des espaces.
    pub fn ecrire(&self) -> String {
        let mut s = format!("{}", self.actif as u8);
        for b in &self.bandes {
            s += &format!(" {} {} {}", b.frequence, b.gain, b.q);
        }
        s
    }

    /// Relit `ecrire` ; `None` si le texte est incomplet ou illisible.
    pub fn lire(t: &str) -> Option<Egaliseur> {
        let n: Vec<f32> = t.split_whitespace().map(|v| v.replace(',', ".").parse().ok()).collect::<Option<_>>()?;
        if n.len() != 1 + 3 * BANDES {
            return None;
        }
        let mut e = Egaliseur { actif: n[0] != 0.0, ..Egaliseur::default() };
        for (i, b) in e.bandes.iter_mut().enumerate() {
            *b = Bande { frequence: n[1 + 3 * i], gain: n[2 + 3 * i], q: n[3 + 3 * i] };
        }
        Some(e.borner())
    }
}

/// Les coefficients d'un biquad, normalisés (a0 = 1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coefs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefs {
    const IDENTITE: Coefs = Coefs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    /// Le gain en dB à la fréquence `f`.
    pub fn reponse(&self, f: f32, fs: f32) -> f32 {
        let w = std::f64::consts::TAU * f as f64 / fs as f64;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let (b0, b1, b2, a1, a2) = (self.b0 as f64, self.b1 as f64, self.b2 as f64, self.a1 as f64, self.a2 as f64);
        let num = (b0 + b1 * c1 + b2 * c2).powi(2) + (b1 * s1 + b2 * s2).powi(2);
        let den = (1.0 + a1 * c1 + a2 * c2).powi(2) + (a1 * s1 + a2 * s2).powi(2);
        (10.0 * (num / den).log10()) as f32
    }
}

/// Les coefficients d'une bande, à la fréquence d'échantillonnage `fs`.
pub fn coefs(forme: Forme, b: Bande, fs: f32) -> Coefs {
    if b.gain.abs() < 0.05 || fs <= 0.0 {
        return Coefs::IDENTITE;
    }
    // Calculés en f64 : les basses fréquences ont des coefficients très proches
    let f = (b.frequence as f64).clamp(10.0, 0.45 * fs as f64);
    let a = 10f64.powf(b.gain as f64 / 40.0);
    let w0 = std::f64::consts::TAU * f / fs as f64;
    let (cw, sw) = (w0.cos(), w0.sin());
    let alpha = sw / (2.0 * (b.q as f64).max(0.05));
    let ra = 2.0 * a.sqrt() * alpha;
    let (b0, b1, b2, a0, a1, a2) = match forme {
        Forme::Cloche => (1.0 + alpha * a, -2.0 * cw, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cw, 1.0 - alpha / a),
        Forme::Grave => (
            a * ((a + 1.0) - (a - 1.0) * cw + ra),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cw),
            a * ((a + 1.0) - (a - 1.0) * cw - ra),
            (a + 1.0) + (a - 1.0) * cw + ra,
            -2.0 * ((a - 1.0) + (a + 1.0) * cw),
            (a + 1.0) + (a - 1.0) * cw - ra,
        ),
        Forme::Aigu => (
            a * ((a + 1.0) + (a - 1.0) * cw + ra),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
            a * ((a + 1.0) + (a - 1.0) * cw - ra),
            (a + 1.0) - (a - 1.0) * cw + ra,
            2.0 * ((a - 1.0) - (a + 1.0) * cw),
            (a + 1.0) - (a - 1.0) * cw - ra,
        ),
    };
    Coefs { b0: (b0 / a0) as f32, b1: (b1 / a0) as f32, b2: (b2 / a0) as f32, a1: (a1 / a0) as f32, a2: (a2 / a0) as f32 }
}

/// Des réglages partagés sans verrou entre l'interface et le rappel audio.
pub struct Partage {
    valeurs: [AtomicU32; 1 + 3 * BANDES],
    version: AtomicU32,
}

impl Partage {
    pub fn new(e: &Egaliseur) -> Partage {
        let p = Partage { valeurs: std::array::from_fn(|_| AtomicU32::new(0)), version: AtomicU32::new(0) };
        p.ecrire(e);
        p
    }

    /// Publie des réglages (côté interface). Un seul écrivain à la fois : le mixeur
    /// écrit sous le verrou de ses pistes.
    pub fn ecrire(&self, e: &Egaliseur) {
        if self.lire().as_ref() == Some(e) {
            return;
        }
        // Version impaire pendant l'écriture : le lecteur réessaiera
        self.version.fetch_add(1, Ordering::AcqRel);
        self.valeurs[0].store(e.actif as u32, Ordering::Relaxed);
        for (i, b) in e.bandes.iter().enumerate() {
            self.valeurs[1 + 3 * i].store(b.frequence.to_bits(), Ordering::Relaxed);
            self.valeurs[2 + 3 * i].store(b.gain.to_bits(), Ordering::Relaxed);
            self.valeurs[3 + 3 * i].store(b.q.to_bits(), Ordering::Relaxed);
        }
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    /// La version des réglages : elle change à chaque écriture.
    pub fn version(&self) -> u32 {
        self.version.load(Ordering::Acquire)
    }

    /// Les réglages, ou `None` si une écriture est en cours (à relire plus tard).
    pub fn lire(&self) -> Option<Egaliseur> {
        let v = self.version.load(Ordering::Acquire);
        if v % 2 == 1 {
            return None;
        }
        let f = |i: usize| f32::from_bits(self.valeurs[i].load(Ordering::Relaxed));
        let mut e = Egaliseur { actif: self.valeurs[0].load(Ordering::Relaxed) != 0, ..Egaliseur::default() };
        for (i, b) in e.bandes.iter_mut().enumerate() {
            *b = Bande { frequence: f(1 + 3 * i), gain: f(2 + 3 * i), q: f(3 + 3 * i) };
        }
        (self.version.load(Ordering::Acquire) == v).then_some(e)
    }
}

/// L'état d'un égaliseur dans le rappel audio : ses coefficients et la mémoire de
/// ses filtres, pour deux canaux.
#[derive(Clone, Copy, Debug)]
pub struct Filtre {
    version: u32,
    fs: f32,
    neutre: bool,
    coefs: [Coefs; BANDES],
    /// Par bande et par canal : les deux états de la forme transposée.
    z: [[[f32; 2]; 2]; BANDES],
}

impl Default for Filtre {
    fn default() -> Filtre {
        Filtre { version: u32::MAX, fs: 0.0, neutre: true, coefs: [Coefs::IDENTITE; BANDES], z: [[[0.0; 2]; 2]; BANDES] }
    }
}

impl Filtre {
    /// Reprend les réglages s'ils ont changé (ou la fréquence de sortie). N'alloue rien.
    pub fn suivre(&mut self, p: &Partage, fs: f32) {
        let v = p.version();
        if v == self.version && fs == self.fs {
            return;
        }
        let Some(e) = p.lire() else { return };
        self.regler(&e, fs);
        self.version = v;
    }

    /// Calcule les coefficients de ces réglages.
    pub fn regler(&mut self, e: &Egaliseur, fs: f32) {
        self.fs = fs;
        let neutre = e.neutre();
        if neutre && !self.neutre {
            // Rien ne reste dans la mémoire des filtres quand on les rallumera
            self.remettre();
        }
        self.neutre = neutre;
        for i in 0..BANDES {
            self.coefs[i] = coefs(FORMES[i], e.bandes[i], fs);
        }
    }

    /// Vide la mémoire des filtres (une voie qui commence).
    pub fn remettre(&mut self) {
        self.z = [[[0.0; 2]; 2]; BANDES];
    }

    pub fn neutre(&self) -> bool {
        self.neutre
    }

    /// Filtre un échantillon du canal `ch` (0 ou 1).
    #[inline]
    pub fn traiter(&mut self, ch: usize, x: f32) -> f32 {
        if self.neutre {
            return x;
        }
        let mut x = x;
        for (c, z) in self.coefs.iter().zip(self.z.iter_mut()) {
            let z = &mut z[ch];
            let y = c.b0 * x + z[0];
            z[0] = c.b1 * x - c.a1 * y + z[1];
            z[1] = c.b2 * x - c.a2 * y;
            x = y;
        }
        x
    }

    /// À la fin d'un tampon : les états minuscules passent à zéro (les nombres
    /// dénormalisés coûtent cher au processeur).
    pub fn nettoyer(&mut self) {
        for z in self.z.iter_mut().flatten().flatten() {
            if z.abs() < 1e-15 {
                *z = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48_000.0;

    /// Le gain mesuré (en dB) d'une sinusoïde de fréquence `f` à travers le filtre.
    fn mesure(e: &Egaliseur, f: f32) -> f32 {
        let mut filtre = Filtre::default();
        filtre.regler(e, FS);
        let n = 48_000;
        let (mut entree, mut sortie) = (0.0f64, 0.0f64);
        for i in 0..n {
            let x = (std::f32::consts::TAU * f * i as f32 / FS).sin() * 0.5;
            let y = filtre.traiter(0, x);
            // Après une demi-seconde, le régime est établi
            if i > n / 2 {
                entree += (x * x) as f64;
                sortie += (y * y) as f64;
            }
        }
        (10.0 * (sortie / entree).log10()) as f32
    }

    #[test]
    fn a_plat_le_son_passe_tel_quel() {
        let e = Egaliseur::default();
        assert!(e.neutre());
        let mut f = Filtre::default();
        f.regler(&e, FS);
        for x in [0.3, -0.7, 0.0, 1.0] {
            assert_eq!(f.traiter(0, x), x);
        }
        assert_eq!(e.reponse(1000.0, FS), 0.0);
    }

    #[test]
    fn chaque_forme_agit_ou_on_l_attend() {
        // Une cloche de +6 dB à 1 kHz : +6 à 1 kHz, presque rien loin d'elle
        let mut e = Egaliseur::default();
        e.bandes[2].gain = 6.0;
        assert!((mesure(&e, 1000.0) - 6.0).abs() < 0.2, "{}", mesure(&e, 1000.0));
        assert!(mesure(&e, 60.0).abs() < 0.5 && mesure(&e, 15_000.0).abs() < 0.5);
        assert!((e.reponse(1000.0, FS) - 6.0).abs() < 0.05);
        // L'étagère grave à -9 dB : les graves baissent, les aigus non
        let mut e = Egaliseur::default();
        e.bandes[0].gain = -9.0;
        assert!((mesure(&e, 25.0) + 9.0).abs() < 0.5, "{}", mesure(&e, 25.0));
        assert!(mesure(&e, 5000.0).abs() < 0.3);
        // L'étagère aiguë à +4 dB
        let mut e = Egaliseur::default();
        e.bandes[4].gain = 4.0;
        assert!((mesure(&e, 18_000.0) - 4.0).abs() < 0.5, "{}", mesure(&e, 18_000.0));
        assert!(mesure(&e, 200.0).abs() < 0.3);
        // La courbe de l'interface suit la mesure
        for f in [40.0, 400.0, 3000.0, 16_000.0] {
            assert!((e.reponse(f, FS) - mesure(&e, f)).abs() < 0.3);
        }
        // Éteint : plus rien
        e.actif = false;
        assert!(e.neutre() && mesure(&e, 18_000.0).abs() < 1e-3);
    }

    #[test]
    fn les_reglages_passent_au_rappel_par_leur_version() {
        let mut e = Egaliseur::default();
        let p = Partage::new(&e);
        let mut f = Filtre::default();
        f.suivre(&p, FS);
        assert!(f.neutre());
        e.bandes[1].gain = 5.0;
        p.ecrire(&e);
        assert_eq!(p.lire(), Some(e));
        f.suivre(&p, FS);
        assert!(!f.neutre());
        // Les mêmes réglages ne changent pas la version
        let v = p.version();
        p.ecrire(&e);
        assert_eq!(p.version(), v);
    }

    #[test]
    fn les_reglages_s_ecrivent_en_texte() {
        let mut e = Egaliseur::default();
        e.bandes[3] = Bande { frequence: 3200.0, gain: -4.5, q: 2.5 };
        e.actif = false;
        assert_eq!(Egaliseur::lire(&e.ecrire()), Some(e));
        assert_eq!(Egaliseur::lire("1 2 3"), None);
        // Hors bornes : ramené
        let l = Egaliseur::lire(&"1 5 40 99 ".repeat(5).trim().split(' ').take(16).collect::<Vec<_>>().join(" ")).unwrap();
        assert_eq!((l.bandes[0].frequence, l.bandes[0].gain, l.bandes[0].q), (20.0, 12.0, 6.0));
    }
}
