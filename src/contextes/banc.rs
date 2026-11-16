//! Banc : le banc d'essai des modules chargés à chaud (`banc.rs`, interface
//! `banc/abi`). La vue montre la scène du module (des triangles colorés) ; un doigt la
//! fait tourner, deux doigts rapprochent et déplacent. Le panneau liste les modules de
//! `Flow/banc/modules` (les charger, recharger le module ouvert) et le journal des
//! commandes.
//!
//! Pilote : `banc` (l'état), `banc modules`, `banc charger <nom>`, `banc recharger`,
//! `banc vue <lacet> <tangage> <distance> <x> <y> <z>`, `banc cadrer`, et `banc <mots…>`
//! : une commande du module.

use crate::banc::{self, banc};
use crate::contexte::{Commun, Contexte, Etat, Genre};
use crate::draw::{Maillage3d, Rect, Terrain3d};
use crate::gestes::Geste;
use crate::sculpture::{self as sc, Camera};
use crate::surface::Placee;
use crate::ui::{id, sub_id, Id};
use crate::widgets::{Cx, Icon};
use std::sync::atomic::{AtomicU64, Ordering};

pub const CLE: &str = "banc";
pub const GENRE: Genre = Genre { cle: CLE, nom: "Banc", legende: "Le banc d'essai : des modules chargés à chaud", icone: Icon::Cube, ouvrir, applet: None };

const BANC: Id = id("banc");
static INSTANCES: AtomicU64 = AtomicU64::new(1);

pub struct Banc {
    camera: Camera,
    /// Le module à charger à l'ouverture (celui de la session passée).
    a_charger: Option<String>,
    num: u64,
    dp: f32,
    zone: Rect,
    orbite: Option<(f32, f32)>,
    pincement: f32,
    message: Option<String>,
}

fn ouvrir(e: &Etat) -> Box<dyn Contexte> {
    let mut camera = Camera { lacet: 0.6, tangage: 0.5, distance: 12.0, cible: [0.0; 3] };
    if let Some(v) = e.texte("vue") {
        let n: Vec<f32> = v.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if let [l, t, d, x, y, z] = n[..] {
            camera = Camera { lacet: l, tangage: t, distance: d.max(0.05), cible: [x, y, z] };
        }
    }
    Box::new(Banc {
        camera,
        a_charger: e.texte("module").filter(|n| !n.is_empty()).map(String::from),
        num: INSTANCES.fetch_add(1, Ordering::Relaxed),
        dp: 1.0,
        zone: Rect::new(0.0, 0.0, 0.0, 0.0),
        orbite: None,
        pincement: 1.0,
        message: None,
    })
}

impl Banc {
    fn cadrer(&mut self) {
        let b = banc();
        let Some(m) = &b.module else { return };
        let s = &m.sommets;
        if s.len() < 9 {
            return;
        }
        let (mut a, mut z) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in s.chunks_exact(9) {
            for k in 0..3 {
                a[k] = a[k].min(p[k]);
                z[k] = z[k].max(p[k]);
            }
        }
        drop(b);
        self.camera.cadrer((a, z));
    }

    fn recevoir(&mut self, g: Geste) {
        match g {
            Geste::SaisieDebut { doigt, .. } => self.orbite = Some(doigt),
            Geste::SaisieDeplace { doigt, .. } => {
                if let Some(avant) = self.orbite {
                    let k = 0.006 / self.dp.max(0.1);
                    self.camera.lacet -= (doigt.0 - avant.0) * k;
                    self.camera.tangage = (self.camera.tangage + (doigt.1 - avant.1) * k).clamp(-1.45, 1.45);
                    self.orbite = Some(doigt);
                }
            }
            Geste::SaisieFin { .. } | Geste::SaisieAnnulee { .. } => self.orbite = None,
            Geste::PincementDebut { .. } => self.pincement = 1.0,
            Geste::Pincement { echelle, deplacement, .. } => {
                let f = echelle / self.pincement.max(1e-3);
                self.pincement = echelle;
                let (_, r, u) = self.camera.axes();
                let k = self.camera.taille_a(self.camera.cible, 1.0, self.zone.h);
                self.camera.cible = sc::oter(self.camera.cible, sc::ajouter(sc::fois(r, deplacement.0 * k), sc::fois(u, -deplacement.1 * k)));
                self.camera.distance = (self.camera.distance / f.max(1e-3)).clamp(0.05, 50_000.0);
            }
            _ => {}
        }
    }

    fn vue(&mut self, cx: &mut Cx, z: Rect) {
        let col = cx.couleurs();
        cx.dl.rrect(z, 0.0, col.fond);
        let b = banc();
        let Some(m) = &b.module else {
            drop(b);
            cx.centered(z, "Aucun module : chargez-en un dans le panneau.", 15.0, col.pale);
            return;
        };
        if m.indices.is_empty() || z.w < 2.0 || z.h < 2.0 {
            drop(b);
            cx.centered(z, "Le module ne montre rien.", 15.0, col.pale);
            return;
        }
        let maillage = Maillage3d { id: 0xBA4C_0000_0000 + self.num * 1000 + m.numero, revision: m.revision, sommets: m.sommets.clone(), indices: m.indices.clone() };
        drop(b);
        let c = [col.fond[0] as f32 / 255.0, col.fond[1] as f32 / 255.0, col.fond[2] as f32 / 255.0];
        cx.clip(Some(z));
        cx.dl.terrain(Terrain3d {
            rect: z,
            mvp: self.camera.mvp(z.w / z.h),
            oeil: self.camera.oeil(),
            ciel: c,
            brume: [1e7, 2e7],
            morceaux: Vec::new(),
            bati: vec![maillage],
            blanc: false,
            aretes: Vec::new(),
            aretes_bati: Vec::new(),
            choix: Vec::new(),
            accent: [0.2, 0.4, 0.9],
            eaux: Vec::new(),
            defonces: Vec::new(),
            defonces_aretes: Vec::new(),
            placage: None,
        });
        cx.clip(None);
    }

    fn panneau(&mut self, cx: &mut Cx, c: &mut Commun, r: Rect, base: Id) {
        let col = cx.couleurs();
        cx.dl.rrect(r, 0.0, col.surface);
        let mut b = cx.body(sub_id(base, 3), r, &c.scrolls);
        if let Some(m) = &self.message {
            cx.paragraph(&mut b, m);
        }
        let ouvert = banc().module.as_ref().map(|m| m.nom.clone());
        cx.section(&mut b, "Modules");
        let liste = c.dossier.as_deref().map(banc::disponibles).unwrap_or_default();
        if liste.is_empty() {
            cx.paragraph(&mut b, "Aucun dans Flow/banc/modules : déposez-y un .so (banc/construire.sh).");
        }
        let mut choix = None;
        for (i, n) in liste.iter().enumerate() {
            let label = if ouvert.as_deref() == Some(n.as_str()) { format!("{n} (recharger)") } else { n.clone() };
            if cx.button(&mut b, sub_id(base, 100 + i), &label) {
                choix = Some(n.clone());
            }
        }
        if ouvert.is_some() && cx.button(&mut b, sub_id(base, 90), "Cadrer") {
            self.cadrer();
        }
        let (fabrique, sur) = { let x = banc(); (x.module.as_ref().is_some_and(|m| m.fabrique_lieux()), x.lieux) };
        if fabrique {
            cx.section(&mut b, "Lieux");
            let t = if sur { "Lieux sur le banc (retirer)" } else { "Mettre Lieux sur le banc" };
            if cx.button(&mut b, sub_id(base, 91), t) {
                let _ = banc::piloter(c.dossier.as_deref(), &["banc".into(), "lieux".into(), if sur { "non" } else { "oui" }.into()]);
            }
        }
        cx.section(&mut b, "Journal");
        let journal: Vec<String> = banc().journal.iter().rev().take(30).cloned().collect();
        if journal.is_empty() {
            cx.paragraph(&mut b, "Vide.");
        }
        for l in journal {
            cx.paragraph(&mut b, &l);
        }
        cx.body_end(b, &mut c.scrolls);
        if let Some(n) = choix {
            self.message = self.charger(c, &n).err();
        }
    }

    fn charger(&mut self, c: &Commun, nom: &str) -> Result<String, String> {
        let d = c.dossier.as_deref().ok_or("pas de dossier Flow")?;
        let premier = banc().module.is_none();
        let r = banc().charger(d, nom);
        if r.is_ok() && premier {
            self.cadrer();
        }
        r
    }
}

impl Contexte for Banc {
    fn cle(&self) -> &'static str {
        CLE
    }

    fn dessiner(&mut self, cx: &mut Cx, c: &mut Commun, p: &Placee) -> String {
        let base = sub_id(BANC, p.id as usize);
        self.dp = cx.dp;
        if let Some(n) = self.a_charger.take() {
            if banc().module.is_none() {
                self.message = self.charger(c, &n).err();
            }
        }
        let r = p.contenu;
        let d = cx.dp;
        let (zone, panneau) = if r.w > r.h * 1.1 {
            let w = (320.0 * d).min(r.w * 0.4);
            (Rect::new(r.x, r.y, r.w - w, r.h), Rect::new(r.right() - w, r.y, w, r.h))
        } else {
            let h = (r.h * 0.4).max(160.0 * d).min(r.h * 0.55);
            (Rect::new(r.x, r.y, r.w, r.h - h), Rect::new(r.x, r.bottom() - h, r.w, h))
        };
        self.zone = zone;
        for g in cx.ui.gestes(sub_id(base, 1), zone) {
            self.recevoir(g);
        }
        self.vue(cx, zone);
        self.panneau(cx, c, panneau, base);
        match &banc().module {
            Some(m) => format!("{} · n° {} · {} triangles", m.nom, m.numero, m.indices.len() / 3),
            None => "Banc".into(),
        }
    }

    fn avancer(&mut self, dt: f32, _c: &Commun) {
        if let Some(m) = &mut banc().module {
            m.pas(dt);
        }
    }

    fn anime(&self, _c: &Commun) -> bool {
        // Le module peut animer sa scène : elle est suivie à chaque image
        banc().module.is_some()
    }

    fn dupliquer(&self) -> Box<dyn Contexte> {
        let mut e = Etat::default();
        self.ecrire(&mut e);
        ouvrir(&e)
    }

    fn ecrire(&self, e: &mut Etat) {
        let c = &self.camera;
        e.mettre("vue", format!("{} {} {} {} {} {}", c.lacet, c.tangage, c.distance, c.cible[0], c.cible[1], c.cible[2]));
        if let Some(m) = &banc().module {
            e.mettre("module", &m.nom);
        }
    }

    fn piloter(&mut self, m: &[String], _c: &mut Commun) -> Option<Result<String, String>> {
        // Les autres commandes du banc sont globales (`banc::piloter`)
        if m[0] != "banc" {
            return None;
        }
        match m.get(1).map(|s| s.as_str()) {
            Some("cadrer") => {
                self.cadrer();
                Some(Ok("cadré".into()))
            }
            Some("vue") => {
                let n: Vec<f32> = m[2..].iter().filter_map(|x| x.parse().ok()).collect();
                Some(match n[..] {
                    [l, t, d, x, y, z] => {
                        self.camera = Camera { lacet: l, tangage: t.clamp(-1.45, 1.45), distance: d.max(0.05), cible: [x, y, z] };
                        Ok("vue réglée".into())
                    }
                    _ => {
                        let k = &self.camera;
                        Ok(format!("vue {} {} {} {} {} {}", k.lacet, k.tangage, k.distance, k.cible[0], k.cible[1], k.cible[2]))
                    }
                })
            }
            _ => None,
        }
    }
}
