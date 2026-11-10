//! L'orchestrateur : le module qui tient à jour ce dont toute l'application dépend,
//! hors de toute fenêtre. Il vit dans `Commun` (`Commun::orchestre`), travaille à
//! chaque image depuis `Commun::orchestrer`, et se montre dans le contexte Orchestre
//! (`contextes/orchestre.rs`), qui expose ses états et ses réglages.
//!
//! Ses tâches :
//!
//! - **les empreintes** : la synthèse de l'analyse du stockage (`Flow/empreintes.txt`)
//!   est refaite quand elle a vieilli au-delà de la période réglée, hors des demandes
//!   de l'utilisateur, et reportée quand la batterie est basse ;
//! - **la batterie** : relevée à intervalle réglé, son historique (deux jours) est
//!   gardé dans `Flow/batterie.txt` ; il en tire le débit et l'autonomie ;
//! - **le recensement des contextes** : les genres disponibles (`contextes::GENRES`)
//!   et les instances ouvertes dans les fenêtres, avec leur document.
//!
//! Ses réglages s'enregistrent dans `Flow/orchestre.txt`, en paires `clé = valeur`
//! qu'on peut modifier à la main. Ce qu'il fait s'inscrit dans un journal (en mémoire).

use crate::surface::FenetreId;
use std::collections::VecDeque;
use std::path::PathBuf;

/// Le fichier des réglages de l'orchestrateur, dans le dossier de Flow.
pub const FICHIER: &str = "orchestre.txt";
/// L'historique de la batterie, dans le dossier de Flow.
pub const BATTERIE: &str = "batterie.txt";
/// L'historique de la batterie ne remonte pas plus loin (secondes).
pub const HISTOIRE: u64 = 48 * 3600;
/// Un relevé identique au précédent n'est gardé qu'après ce délai (secondes) : la
/// courbe reste lisible sans un point par minute.
const REPETITION: u64 = 30 * 60;
/// Lignes gardées dans le journal.
const JOURNAL: usize = 120;

/// Ce que l'utilisateur règle dans le contexte Orchestre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReglagesOrchestre {
    /// Refaire les empreintes de soi-même.
    pub empreintes_auto: bool,
    /// L'âge, en heures, au-delà duquel les empreintes sont refaites.
    pub empreintes_heures: f32,
    /// L'intervalle des relevés de la batterie, en minutes.
    pub batterie_minutes: f32,
    /// Le niveau, en %, en dessous duquel la batterie est basse.
    pub seuil_batterie: f32,
    /// Batterie basse et hors charge : pas d'analyse automatique du stockage.
    pub economie: bool,
}

impl Default for ReglagesOrchestre {
    fn default() -> Self {
        ReglagesOrchestre { empreintes_auto: true, empreintes_heures: 24.0, batterie_minutes: 1.0, seuil_batterie: 15.0, economie: true }
    }
}

impl ReglagesOrchestre {
    /// Ramène chaque réglage dans ses bornes, à des valeurs rondes.
    pub fn borner(mut self) -> Self {
        let r = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi).round().max(d) } else { d };
        self.empreintes_heures = r(self.empreintes_heures, 1.0, 168.0, 1.0);
        self.batterie_minutes = r(self.batterie_minutes, 1.0, 30.0, 1.0);
        self.seuil_batterie = r(self.seuil_batterie, 5.0, 50.0, 5.0);
        self
    }

    /// La période des empreintes, en secondes.
    pub fn periode(&self) -> u64 {
        (self.empreintes_heures.max(1.0) * 3600.0) as u64
    }

    pub fn ecrire(&self) -> String {
        let oui = |b: bool| if b { "oui" } else { "non" };
        let mut s = String::new();
        s += "# Réglages de l'orchestrateur de Flow (contexte Orchestre). Fichier modifiable à la main.\n";
        s += "# empreintes_auto : refaire la synthèse du stockage de soi-même (oui/non)\n";
        s += &format!("empreintes_auto = {}\n", oui(self.empreintes_auto));
        s += "# empreintes_heures : âge de la synthèse au-delà duquel elle est refaite (1 à 168)\n";
        s += &format!("empreintes_heures = {}\n", self.empreintes_heures);
        s += "# batterie_minutes : intervalle des relevés de la batterie (1 à 30)\n";
        s += &format!("batterie_minutes = {}\n", self.batterie_minutes);
        s += "# seuil_batterie : en dessous (en %), la batterie est basse (5 à 50)\n";
        s += &format!("seuil_batterie = {}\n", self.seuil_batterie);
        s += "# economie : batterie basse et hors charge, pas d'analyse automatique (oui/non)\n";
        s += &format!("economie = {}\n", oui(self.economie));
        s
    }

    /// Relit des réglages ; ce qui manque ou est illisible garde sa valeur par défaut.
    pub fn lire(t: &str) -> Self {
        let mut r = ReglagesOrchestre::default();
        for l in t.lines() {
            let Some((k, v)) = l.split_once('=') else { continue };
            if l.trim_start().starts_with('#') {
                continue;
            }
            let v = v.trim().to_lowercase();
            let nombre = v.replace(',', ".").parse::<f32>().ok();
            let oui = match v.as_str() {
                "oui" | "vrai" | "1" | "yes" | "true" => Some(true),
                "non" | "faux" | "0" | "no" | "false" => Some(false),
                _ => None,
            };
            match k.trim() {
                "empreintes_auto" => r.empreintes_auto = oui.unwrap_or(r.empreintes_auto),
                "empreintes_heures" => r.empreintes_heures = nombre.unwrap_or(r.empreintes_heures),
                "batterie_minutes" => r.batterie_minutes = nombre.unwrap_or(r.batterie_minutes),
                "seuil_batterie" => r.seuil_batterie = nombre.unwrap_or(r.seuil_batterie),
                "economie" => r.economie = oui.unwrap_or(r.economie),
                _ => {}
            }
        }
        r.borner()
    }
}

/// Un relevé de la batterie.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Releve {
    /// Secondes Unix.
    pub date: u64,
    pub niveau: u8,
    pub charge: bool,
}

/// Une instance de contexte ouverte dans une fenêtre, telle que l'application la
/// déclare à chaque image.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub fenetre: FenetreId,
    pub cle: &'static str,
    pub document: Option<PathBuf>,
}

pub struct Orchestre {
    pub reglages: ReglagesOrchestre,
    /// Le texte des réglages tel qu'il est sur le disque : ils ne s'écrivent que
    /// s'ils changent.
    pub(crate) ecrits: Option<String>,
    /// Le dossier d'où les réglages et l'historique ont été lus.
    pub(crate) dossier: Option<PathBuf>,
    /// L'historique de la batterie, du plus ancien au plus récent.
    pub releves: VecDeque<Releve>,
    /// La dernière lecture de la batterie (secondes Unix), gardée ou non.
    lue: Option<u64>,
    /// La dernière lecture, même non gardée dans l'historique.
    derniere: Option<Releve>,
    /// L'historique a changé et doit être réécrit.
    pub(crate) batterie_a_ecrire: bool,
    /// La batterie basse a déjà été annoncée au journal (jusqu'à la remontée).
    alerte: bool,
    /// Le report d'une analyse a déjà été annoncé.
    pub(crate) report_annonce: bool,
    /// Les instances ouvertes, par fenêtre.
    pub instances: Vec<Instance>,
    /// Ce qui s'est passé : date (secondes Unix) et texte, du plus ancien au plus récent.
    pub journal: VecDeque<(u64, String)>,
}

impl Default for Orchestre {
    fn default() -> Self {
        Orchestre {
            reglages: ReglagesOrchestre::default(),
            ecrits: None,
            dossier: None,
            releves: VecDeque::new(),
            lue: None,
            derniere: None,
            batterie_a_ecrire: false,
            alerte: false,
            report_annonce: false,
            instances: Vec::new(),
            journal: VecDeque::new(),
        }
    }
}

impl Orchestre {
    /// Inscrit une ligne au journal.
    pub fn noter(&mut self, date: u64, texte: impl Into<String>) {
        let texte = texte.into();
        crate::android::log(&format!("orchestre : {texte}"));
        self.journal.push_back((date, texte));
        while self.journal.len() > JOURNAL {
            self.journal.pop_front();
        }
    }

    /// Vrai s'il est temps de relever la batterie.
    pub fn batterie_due(&self, maintenant: u64) -> bool {
        let pas = (self.reglages.batterie_minutes.max(1.0) * 60.0) as u64;
        // Quelques secondes de marge : le réveil de la minute n'est pas exact
        self.lue.is_none_or(|t| maintenant + 5 >= t + pas || maintenant < t)
    }

    /// Un relevé de la batterie. Il entre dans l'historique s'il diffère du dernier
    /// gardé, ou si celui-ci est ancien ; le passage sous le seuil et les changements
    /// de charge s'inscrivent au journal.
    pub fn relever(&mut self, niveau: u8, charge: bool, maintenant: u64) {
        let r = Releve { date: maintenant, niveau: niveau.min(100), charge };
        self.lue = Some(maintenant);
        let avant = self.derniere.replace(r);
        if let Some(a) = avant {
            if a.charge != charge {
                self.noter(maintenant, if charge { format!("batterie en charge ({niveau} %)") } else { format!("batterie débranchée ({niveau} %)") });
            }
        }
        let basse = self.basse();
        if basse && !self.alerte {
            self.noter(maintenant, format!("batterie basse : {niveau} % (seuil {:.0} %)", self.reglages.seuil_batterie));
        }
        self.alerte = basse;
        let garder = self.releves.back().is_none_or(|g| g.niveau != r.niveau || g.charge != r.charge || maintenant >= g.date + REPETITION || maintenant < g.date);
        if garder {
            self.releves.push_back(r);
            self.batterie_a_ecrire = true;
        }
        self.elaguer(maintenant);
    }

    /// Oublie les relevés de plus de deux jours.
    fn elaguer(&mut self, maintenant: u64) {
        while self.releves.front().is_some_and(|r| r.date + HISTOIRE < maintenant) {
            self.releves.pop_front();
            self.batterie_a_ecrire = true;
        }
    }

    /// Le dernier état connu de la batterie : niveau, en charge.
    pub fn batterie(&self) -> Option<(u8, bool)> {
        self.derniere.or(self.releves.back().copied()).map(|r| (r.niveau, r.charge))
    }

    /// Vrai si la batterie est sous le seuil, hors charge.
    pub fn basse(&self) -> bool {
        self.batterie().is_some_and(|(n, c)| !c && (n as f32) < self.reglages.seuil_batterie)
    }

    /// Le débit de la batterie, en % par heure (positif en charge, négatif en
    /// décharge), tiré des relevés depuis le dernier branchement ou débranchement, sur
    /// six heures au plus. `None` tant qu'il n'y a pas assez de recul.
    pub fn debit(&self) -> Option<f32> {
        let fin = self.derniere.or(self.releves.back().copied())?;
        let mut debut = fin;
        for r in self.releves.iter().rev() {
            if r.charge != fin.charge || r.date + 6 * 3600 < fin.date || r.date > fin.date {
                break;
            }
            debut = *r;
        }
        let duree = fin.date.saturating_sub(debut.date) as f32 / 3600.0;
        let ecart = fin.niveau as f32 - debut.niveau as f32;
        (duree >= 10.0 / 60.0 && ecart != 0.0).then(|| ecart / duree)
    }

    /// Le temps, en heures, avant la batterie vide (en décharge) ou pleine (en charge).
    pub fn autonomie(&self) -> Option<f32> {
        let (n, charge) = self.batterie()?;
        let d = self.debit()?;
        match (charge, d > 0.0) {
            (true, true) => Some((100.0 - n as f32) / d),
            (false, false) => Some(n as f32 / -d),
            _ => None,
        }
    }

    /// L'historique en texte : une ligne par relevé, date, niveau, charge.
    pub fn ecrire_releves(&self) -> String {
        let mut s = String::from("# Historique de la batterie (deux jours) : date Unix, niveau en %, en charge (1/0), séparés par des tabulations.\n");
        for r in &self.releves {
            s += &format!("{}\t{}\t{}\n", r.date, r.niveau, r.charge as u8);
        }
        s
    }

    /// Relit un historique ; les lignes illisibles sont ignorées.
    pub fn lire_releves(t: &str) -> Vec<Releve> {
        let mut v: Vec<Releve> = t
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut c = l.split('\t');
                let date = c.next()?.trim().parse().ok()?;
                let niveau: u8 = c.next()?.trim().parse().ok()?;
                let charge = c.next()?.trim() == "1";
                Some(Releve { date, niveau: niveau.min(100), charge })
            })
            .collect();
        v.sort_by_key(|r| r.date);
        v
    }

    /// Reprend un historique relu : il passe avant les relevés déjà faits.
    pub fn reprendre(&mut self, lus: Vec<Releve>, maintenant: u64) {
        let premier = self.releves.front().map_or(u64::MAX, |r| r.date);
        for r in lus.into_iter().rev().filter(|r| r.date < premier) {
            self.releves.push_front(r);
        }
        self.elaguer(maintenant);
    }

    /// Le nombre d'instances ouvertes d'un contexte.
    pub fn ouvertes(&self, cle: &str) -> usize {
        self.instances.iter().filter(|i| i.cle == cle).count()
    }
}

/// Une durée lisible : « 45 s », « 12 min », « 3 h 05 », « 2 j 4 h ».
pub fn duree(s: u64) -> String {
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        3600..86400 => format!("{} h {:02}", s / 3600, s % 3600 / 60),
        _ => format!("{} j {} h", s / 86400, s % 86400 / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_reglages_se_lisent_et_s_ecrivent() {
        let r = ReglagesOrchestre { empreintes_auto: false, empreintes_heures: 6.0, batterie_minutes: 5.0, seuil_batterie: 20.0, economie: false };
        assert_eq!(ReglagesOrchestre::lire(&r.ecrire()), r);
        // Bornes, valeurs absentes ou illisibles
        let l = ReglagesOrchestre::lire("empreintes_heures = 900\nseuil_batterie = rien\n# economie = non\n");
        assert_eq!(l.empreintes_heures, 168.0);
        assert_eq!(l.seuil_batterie, 15.0);
        assert!(l.economie);
    }

    #[test]
    fn la_batterie_donne_son_debit_et_son_autonomie() {
        let mut o = Orchestre::default();
        assert!(o.batterie_due(1000));
        o.relever(80, false, 1000);
        assert!(!o.batterie_due(1030));
        assert!(o.batterie_due(1060));
        // Un relevé identique n'entre pas dans l'historique
        o.relever(80, false, 1060);
        assert_eq!(o.releves.len(), 1);
        // 10 % en deux heures : -5 %/h, 14 h d'autonomie à 70 %
        o.relever(70, false, 1000 + 7200);
        assert_eq!(o.debit(), Some(-5.0));
        assert_eq!(o.autonomie(), Some(14.0));
        // Branchée : plus de débit tant qu'il n'y a pas de recul
        o.relever(70, true, 1000 + 7260);
        assert_eq!(o.debit(), None);
        assert!(o.journal.iter().any(|(_, t)| t.contains("en charge")));
        // L'historique s'écrit et se relit
        let lus = Orchestre::lire_releves(&o.ecrire_releves());
        assert_eq!(lus, o.releves.iter().copied().collect::<Vec<_>>());
        let mut p = Orchestre::default();
        p.relever(71, true, 1000 + 7500);
        p.reprendre(lus, 1000 + 7500);
        assert_eq!(p.releves.len(), 4);
        // La batterie basse s'annonce une fois
        let mut b = Orchestre::default();
        b.relever(10, false, 0);
        b.relever(9, false, 60);
        assert!(b.basse());
        assert_eq!(b.journal.iter().filter(|(_, t)| t.contains("basse")).count(), 1);
    }

    #[test]
    fn les_durees_se_lisent() {
        assert_eq!(duree(45), "45 s");
        assert_eq!(duree(720), "12 min");
        assert_eq!(duree(3 * 3600 + 300), "3 h 05");
        assert_eq!(duree(2 * 86400 + 4 * 3600), "2 j 4 h");
    }
}
