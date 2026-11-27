//! Le cache sur disque des calculs faits une fois par lieu (le réseau nivelé…), dans
//! `Flow/cache` : un fichier par lieu et par calcul, reconnu par l'empreinte de ce
//! dont il dépend et la version de Flow (une nouvelle version refait les calculs).

use crate::reseau::{Arete, Noeud, Origine, Passage, Reseau};
use sage_reel::{Files, Lieu};
use std::path::PathBuf;
use std::sync::Mutex;

static DOSSIER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Le dossier de Flow, où ranger le cache (`None` : pas de cache).
pub fn regler(flow: Option<PathBuf>) {
    if let Ok(mut d) = DOSSIER.lock() {
        *d = flow.map(|f| f.join("cache"));
    }
}

/// Le fichier d'un calcul : son nom, la version de Flow et l'empreinte.
fn fichier(nom: &str, h: u64) -> Option<PathBuf> {
    DOSSIER.lock().ok()?.as_ref().map(|d| d.join(format!("{nom}-v{}-{h:016x}.bin", env!("CARGO_PKG_VERSION"))))
}

/// Efface les anciens fichiers du calcul `nom` autres que `f` : tous ceux d'une autre
/// version de Flow (ils ne resserviront pas), et ceux de cette version laissés depuis
/// plus d'une semaine (un autre lieu, pas rouvert).
fn nettoyer(nom: &str, f: &std::path::Path) {
    let Some(d) = f.parent() else { return };
    let _ = std::fs::create_dir_all(d);
    let version = format!("{nom}-v{}-", env!("CARGO_PKG_VERSION"));
    let Ok(v) = std::fs::read_dir(d) else { return };
    for e in v.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !n.starts_with(&format!("{nom}-")) || e.path() == f {
            continue;
        }
        let vieux = e.metadata().is_ok_and(|m| m.modified().is_ok_and(|t| t.elapsed().is_ok_and(|a| a.as_secs() > 7 * 86400)));
        if !n.starts_with(&version) || vieux {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// FNV-1a sur 64 bits.
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn octets(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn f(&mut self, x: f32) {
        self.octets(&x.to_le_bytes());
    }
    fn n(&mut self, x: u64) {
        self.octets(&x.to_le_bytes());
    }
}

fn routes(h: &mut Fnv, v: &[sage_reel::Route]) {
    h.n(v.len() as u64);
    for r in v {
        h.octets(&[r.niveau as u8, r.revetement, r.genre, r.files.total, r.files.avant, r.files.arriere, u8::from(r.files.osm)]);
        h.n(u64::from(r.files.bus));
        h.f(r.largeur);
        h.n(r.points.len() as u64);
        for p in &r.points {
            h.f(p[0]);
            h.f(p[1]);
            h.f(p[2]);
        }
    }
}

/// L'empreinte de ce dont le réseau nivelé dépend : les voies, les ponts, les
/// tunnels, le relief ; et la version de Flow.
pub fn empreinte_reseau(l: &Lieu) -> u64 {
    let mut h = Fnv::new();
    h.octets(env!("CARGO_PKG_VERSION").as_bytes());
    routes(&mut h, &l.routes);
    routes(&mut h, &l.ponts);
    routes(&mut h, &l.transports.tunnels);
    for x in [l.pas as f32, l.niveau, l.base, l.echelle] {
        h.f(x);
    }
    h.n(l.h.len() as u64);
    // Le relief : ses hauteurs, par paquets (plus vite qu'octet par octet)
    let mut acc = 0u64;
    for (i, &v) in l.h.iter().enumerate() {
        acc = acc.rotate_left(5) ^ u64::from(v).wrapping_add(i as u64);
        if i % 4096 == 4095 {
            h.n(acc);
        }
    }
    h.n(acc);
    h.0
}

// --- Écriture et lecture ---------------------------------------------------------

const MAGIE: &[u8; 8] = b"FLRESEA2";

pub(crate) struct Ecrit(pub(crate) Vec<u8>);

impl Ecrit {
    pub(crate) fn u(&mut self, x: u32) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
    pub(crate) fn f(&mut self, x: f32) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }
    pub(crate) fn b(&mut self, x: u8) {
        self.0.push(x);
    }
    pub(crate) fn fs(&mut self, v: &[f32]) {
        for &x in v {
            self.f(x);
        }
    }
    pub(crate) fn p(&mut self, p: [f32; 3]) {
        self.f(p[0]);
        self.f(p[1]);
        self.f(p[2]);
    }
}

pub(crate) struct Lu<'a>(pub(crate) &'a [u8], pub(crate) usize);

impl Lu<'_> {
    fn prendre(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.0.get(self.1..self.1 + n)?;
        self.1 += n;
        Some(s)
    }
    pub(crate) fn u(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.prendre(4)?.try_into().ok()?))
    }
    pub(crate) fn f(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.prendre(4)?.try_into().ok()?))
    }
    pub(crate) fn b(&mut self) -> Option<u8> {
        Some(self.prendre(1)?[0])
    }
    pub(crate) fn p(&mut self) -> Option<[f32; 3]> {
        Some([self.f()?, self.f()?, self.f()?])
    }
    pub(crate) fn fs<const N: usize>(&mut self) -> Option<[f32; N]> {
        let mut v = [0.0f32; N];
        for x in &mut v {
            *x = self.f()?;
        }
        Some(v)
    }
    pub(crate) fn fini(&self) -> bool {
        self.1 == self.0.len()
    }
}

fn ecrire_reseau(r: &Reseau) -> Vec<u8> {
    let mut e = Ecrit(MAGIE.to_vec());
    e.u(r.raccords as u32);
    e.u(r.a_niveau as u32);
    e.u(r.noeuds.len() as u32);
    for n in &r.noeuds {
        e.p(n.p);
        e.b(match n.origine {
            Origine::Donnees => 0,
            Origine::Bord => 1,
            Origine::Raccord => 2,
            Origine::RaccordSommet => 3,
            Origine::Croisement => 4,
        });
        e.u(n.aretes.len() as u32);
        for &k in &n.aretes {
            e.u(k as u32);
        }
    }
    e.u(r.aretes.len() as u32);
    for a in &r.aretes {
        e.u(a.a as u32);
        e.u(a.b as u32);
        e.b(match a.passage {
            Passage::Sol => 0,
            Passage::Pont => 1,
            Passage::Tunnel => 2,
        });
        e.b(a.genre);
        e.f(a.largeur);
        e.b(a.revetement);
        let f = a.files;
        for x in [f.total, f.avant, f.arriere, u8::from(f.osm)] {
            e.b(x);
        }
        e.u(u32::from(f.bus));
        e.b(a.niveau as u8);
        e.u(a.points.len() as u32);
        for &p in &a.points {
            e.p(p);
        }
    }
    for v in [&r.tabliers, &r.fixes] {
        e.u(v.len() as u32);
        for &k in v.iter() {
            e.u(k);
        }
    }
    e.0
}

fn lire_reseau(o: &[u8]) -> Option<Reseau> {
    if o.get(..8)? != MAGIE {
        return None;
    }
    let mut l = Lu(o, 8);
    let raccords = l.u()? as usize;
    let a_niveau = l.u()? as usize;
    let nn = l.u()? as usize;
    let mut noeuds = Vec::with_capacity(nn);
    for _ in 0..nn {
        let p = l.p()?;
        let origine = match l.b()? {
            1 => Origine::Bord,
            2 => Origine::Raccord,
            3 => Origine::RaccordSommet,
            4 => Origine::Croisement,
            _ => Origine::Donnees,
        };
        let k = l.u()? as usize;
        let mut aretes = Vec::with_capacity(k);
        for _ in 0..k {
            aretes.push(l.u()? as usize);
        }
        noeuds.push(Noeud { p, aretes, origine });
    }
    let na = l.u()? as usize;
    let mut aretes = Vec::with_capacity(na);
    for _ in 0..na {
        let (a, b) = (l.u()? as usize, l.u()? as usize);
        let passage = match l.b()? {
            0 => Passage::Sol,
            1 => Passage::Pont,
            _ => Passage::Tunnel,
        };
        let genre = l.b()?;
        let largeur = l.f()?;
        let revetement = l.b()?;
        let (total, avant, arriere, osm) = (l.b()?, l.b()?, l.b()?, l.b()? != 0);
        let bus = l.u()? as u16;
        let niveau = l.b()? as i8;
        let np = l.u()? as usize;
        let mut points = Vec::with_capacity(np);
        for _ in 0..np {
            points.push(l.p()?);
        }
        aretes.push(Arete { a, b, points, passage, genre, largeur, revetement, files: Files { total, avant, arriere, osm, bus }, niveau });
    }
    let mut listes = [Vec::new(), Vec::new()];
    for v in listes.iter_mut() {
        let n = l.u()? as usize;
        for _ in 0..n {
            v.push(l.u()?);
        }
    }
    let [tabliers, fixes] = listes;
    (l.1 == o.len()).then_some(Reseau { noeuds, aretes, raccords, a_niveau, tabliers, fixes })
}

/// Un fichier du cache (`nom`, empreinte `h`), s'il y est.
pub fn lire(nom: &str, h: u64) -> Option<Vec<u8>> {
    std::fs::read(fichier(nom, h)?).ok()
}

/// Range un fichier dans le cache (sans erreur : le cache est facultatif), et efface
/// ceux du même nom vieux de plus d'une semaine.
pub fn garder(nom: &str, h: u64, o: &[u8]) {
    let Some(f) = fichier(nom, h) else { return };
    nettoyer(nom, &f);
    let tmp = f.with_extension("tmp");
    if std::fs::write(&tmp, o).is_ok() {
        let _ = std::fs::rename(&tmp, &f);
    }
}

/// Le réseau nivelé du cache, s'il y est pour cette empreinte.
pub fn reseau(h: u64) -> Option<Reseau> {
    let f = fichier("reseau", h)?;
    let o = std::fs::read(f).ok()?;
    lire_reseau(&o)
}

/// Range le réseau nivelé dans le cache (sans erreur : le cache est facultatif).
pub fn garder_reseau(h: u64, r: &Reseau) {
    let Some(f) = fichier("reseau", h) else { return };
    nettoyer("reseau", &f);
    let tmp = f.with_extension("tmp");
    if std::fs::write(&tmp, ecrire_reseau(r)).is_ok() {
        let _ = std::fs::rename(&tmp, &f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_reseau_fait_l_aller_retour() {
        let r = Reseau {
            noeuds: vec![Noeud { p: [1.0, 2.0, 3.0], aretes: vec![0], origine: Origine::Raccord }, Noeud { p: [4.0, 5.0, 6.0], aretes: vec![0], origine: Origine::Donnees }],
            aretes: vec![Arete { a: 0, b: 1, points: vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]], passage: Passage::Pont, genre: 1, largeur: 7.5, revetement: 2, files: Files { total: 2, avant: 1, arriere: 1, osm: true, bus: 2 }, niveau: -1 }],
            raccords: 3,
            a_niveau: 4,
            tabliers: vec![0],
            fixes: vec![],
        };
        let l = lire_reseau(&ecrire_reseau(&r)).unwrap();
        assert_eq!(l.aretes[0].points, r.aretes[0].points);
        assert_eq!(l.aretes[0].files.bus, 2);
        assert_eq!(l.aretes[0].niveau, -1);
        assert_eq!(l.noeuds[1].p, r.noeuds[1].p);
    }
}
