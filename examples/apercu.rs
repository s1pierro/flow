//! Dessine l'interface sur le vrai processeur graphique, hors écran, et écrit
//! des images PPM. Sert à valider nuanceurs et mise en page sans installer l'APK.
//!
//!   cargo run --example apercu -- <dossier>

use flow::app::App;
use flow::contextes::indice;
use flow::draw::{DrawList, Rect};
use flow::gles::{load_gl, Egl};
use flow::renderer::Renderer;
use flow::text::Text;
use flow::ui::{Event, Ui};
use flow::widgets::Cx;

struct Shot {
    name: &'static str,
    w: i32,
    h: i32,
    dp: f32,
    end: bool,
    help: bool,
    fenetres: bool,
    /// Le clavier ouvert sur le champ d'essai des paramètres.
    clavier: bool,
    /// L'explorateur à côté de l'éditeur de texte, ouvert sur le README, clavier sorti.
    texte: bool,
    /// Un dossier à analyser dans une fenêtre Disque.
    disque: Option<&'static str>,
    /// Encre : 1, des traits et un tracé en cours (la barre et le curseur déporté) ;
    /// 2, un appui tenu (l'anneau de durée), avec l'onglet Gestes des paramètres.
    gestes: u8,
}

/// Où toucher le dessin SVG d'essai pour choisir le groupe tourné.
const SVG_TAP: (f32, f32) = (700.0, 600.0);

/// Les touchers de l'aperçu `svg-simplifier` : la tache tremblée, « Simplifier »,
/// un écart de 1, « Courbes ».
const SIMPLIFIER: [(f32, f32); 4] = [(960.0, 560.0), (242.0, 1023.0), (549.0, 942.0), (683.0, 942.0)];

/// Les touchers de l'aperçu `svg-sommets` : la pilule, la forme, un sommet, puis un
/// sommet à tirer.
const SOMMETS: [(f32, f32); 4] = [(210.0, 944.0), (750.0, 445.0), (746.0, 384.0), (746.0, 384.0)];

fn main() {
    let dir = std::env::args().nth(1).expect("dossier de sortie");
    let shots = [
        Shot { name: "tablette-ferme", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "tablette-parametres", w: 1920, h: 1200, dp: 1.5, end: true, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "tablette-portrait", w: 1200, h: 1920, dp: 1.5, end: true, help: true, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "tablette-fenetres", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: true, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "disque", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: Some(env!("CARGO_MANIFEST_DIR")), gestes: 0 },
        Shot { name: "clavier", w: 1920, h: 1200, dp: 1.5, end: true, help: false, fenetres: false, clavier: true, texte: false, disque: None, gestes: 0 },
        Shot { name: "texte", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: true, disque: None, gestes: 0 },
        Shot { name: "telephone", w: 2400, h: 1080, dp: 2.5, end: true, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "mixeur", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "mixeur-etroit", w: 1200, h: 1920, dp: 2.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "encre", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 1 },
        Shot { name: "texte-code", w: 1200, h: 1920, dp: 2.0, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "texte-html", w: 1200, h: 1920, dp: 2.0, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "texte-python", w: 1200, h: 1920, dp: 2.0, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "texte-selection", w: 1200, h: 1920, dp: 2.0, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 3 },
        Shot { name: "fichiers-selection", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 4 },
        Shot { name: "fichiers-nouveau", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 5 },
        Shot { name: "svg", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 6 },
        Shot { name: "svg-portrait", w: 1200, h: 1920, dp: 2.0, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "svg-source", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 7 },
        Shot { name: "svg-sommets", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 8 },
        Shot { name: "svg-simplifier", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 10 },
        Shot { name: "argile", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 9 },
        Shot { name: "argile-portrait", w: 1200, h: 1920, dp: 2.0, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 9 },
        Shot { name: "accueil", w: 1920, h: 1200, dp: 1.5, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "accueil-menu", w: 1200, h: 1920, dp: 2.0, end: false, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 0 },
        Shot { name: "encre-portrait", w: 1200, h: 1920, dp: 1.5, end: true, help: false, fenetres: false, clavier: false, texte: false, disque: None, gestes: 2 },
    ];
    let mut egl = Egl::new(true).expect("EGL");
    egl.attach_pbuffer(2400, 2400).expect("pbuffer");
    let render = Renderer::new(load_gl(true).expect("GL")).expect("nuanceurs");
    println!("nuanceurs compilés et liés");
    let mut text = Text::new();

    // FLOW_APERCU=nom : une seule prise de vue
    let seule = std::env::var("FLOW_APERCU").ok();
    for s in shots.into_iter().filter(|s| seule.as_deref().is_none_or(|n| n == s.name)) {
        let (w, h) = (s.w as f32, s.h as f32);
        let screen = Rect::new(0.0, 0.0, w, h);
        let mut app = App::new(s.dp);
        let mut ui = Ui::new(s.dp);
        let mut dl = DrawList::default();
        // L'explorateur part du dossier du projet
        app.racine_en(Some(env!("CARGO_MANIFEST_DIR").into()));
        app.shell.end.open = s.end;
        if s.fenetres {
            // Trois fenêtres : l'explorateur à gauche, la radio et l'aide à droite
            use flow::surface::Sens;
            app.surface.partager(1, Sens::Colonnes, 0.6);
            let liste = app.surface.partager(1, Sens::Lignes, 0.55).unwrap();
            app.surface.changer_vue(liste, indice("radio").unwrap());
            app.surface.changer_vue(1, indice("aide").unwrap());
        }

        if s.texte {
            let readme = concat!(env!("CARGO_MANIFEST_DIR"), "/README.md");
            app.ouvrir_espace(&format!("fenetres = C0.3800(F1:fichiers,F2:texte)\n[fenetre 2]\ndocument = {readme}\ncurseur = 120\n"));
            app.commun.clavier.ouvrir(flow::ui::sub_id(flow::ui::id("texte/champ"), 2));
        }
        if s.clavier {
            app.commun.clavier.ouvrir(flow::parametres::ESSAI);
        }

        if let Some(d) = s.disque {
            // Le graphique en anneaux, à côté de l'explorateur
            use flow::surface::Sens;
            app.racine_en(Some(d.into()));
            let f = app.surface.partager(1, Sens::Colonnes, 0.62).unwrap();
            app.surface.changer_vue(1, indice("disque").unwrap());
            app.surface.changer_vue(f, indice("fichiers").unwrap());
        }

        if s.name.starts_with("mixeur") {
            // Trois pistes connues (sans rien jouer : l'aperçu reste muet)
            use flow::mixage::{mixeur, Reglage};
            mixeur().regler("Radio", Reglage { gain: 0.65, balance: -0.3, ..Reglage::default() });
            mixeur().regler("Fichier", Reglage { muet: true, ..Reglage::default() });
            mixeur().regler("Essai", Reglage { gain: 0.4, prioritaire: true, ..Reglage::default() });
            app.ouvrir_espace("fenetres = F1:mixeur\n");
        }
        if s.gestes > 0 {
            // Le dessin s'enregistre : pas dans le projet
            let d = std::env::temp_dir().join("flow-apercu");
            let _ = std::fs::remove_dir_all(&d);
            app.racine_en(Some(d));
            app.ouvrir_espace("fenetres = F1:encre\n[fenetre 1]\ntaille = 2\ncouleur = #2563eb\n");
            app.onglet_parametres = 2;
            // Le portrait en thème sombre : la barre doit y rester lisible
            app.commun.theme.sombre = s.gestes == 2;
        }

        if s.name == "texte-code" {
            // Un fichier Rust, deux blocs repliés, le curseur plus bas
            let f = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tampon.rs");
            let car: Vec<char> = std::fs::read_to_string(f).unwrap().chars().collect();
            let b = flow::tampon::blocs(&car);
            let plis = format!("{} {}", b[1].0, b[3].0);
            app.ouvrir_espace(&format!("fenetres = F1:texte\n[fenetre 1]\ndocument = {f}\nplis = {plis}\n"));
        }
        if s.name == "texte-html" {
            // Une page HTML (script et style), en thème sombre
            let d = std::env::temp_dir().join("flow-apercu-html");
            let _ = std::fs::create_dir_all(&d);
            let f = d.join("page.html");
            std::fs::write(&f, "<!DOCTYPE html>\n<!-- Une page d'essai -->\n<html lang=\"fr\">\n<head>\n  <meta charset=\"utf-8\">\n  <style>\n    @media (min-width: 600px) {\n      body { font-size: 18px; color: #1e1e25 !important; }\n    }\n  </style>\n</head>\n<body>\n  <p class=\"note\">Bonjour &amp; bienvenue</p>\n  <script>\n    // Compte jusqu'à dix\n    const n = 10;\n    for (let i = 0; i < n; i++) {\n      console.log(`pas ${i}`, Math.sqrt(i * 2.5));\n    }\n  </script>\n</body>\n</html>\n").unwrap();
            app.ouvrir_espace(&format!("fenetres = F1:texte\n[fenetre 1]\ndocument = {}\n", f.display()));
            app.commun.theme.sombre = true;
        }
        if s.name == "texte-python" {
            // Python à gauche, Markdown avec un bloc TOML et un JSON à droite
            let d = std::env::temp_dir().join("flow-apercu-python");
            let _ = std::fs::create_dir_all(&d);
            let py = d.join("essai.py");
            std::fs::write(&py, "import json\nfrom pathlib import Path\n\n\n@dataclass\nclass Note:\n    \"\"\"Une note, avec\n    son titre.\"\"\"\n    titre: str\n    poids: float = 1.5\n\n    def lire(self, chemin: Path) -> dict:\n        # Lit le fichier\n        if not chemin.exists():\n            return None\n        return json.loads(chemin.read_text())\n").unwrap();
            let md = d.join("notes.md");
            std::fs::write(&md, "# Réglages\n\nLe fichier **Cargo.toml** décrit le `crate` :\n\n```toml\n[package]\nname = \"flow\"  # le nom\nversion = \"0.15.0\"\n```\n\n> Voir [la doc](https://doc.rust-lang.org).\n\n```json\n{\"actif\": true, \"taille\": 12}\n```\n\n- un\n- deux\n").unwrap();
            app.ouvrir_espace(&format!("fenetres = L0.5000(F1:texte,F2:texte)\n[fenetre 1]\ndocument = {}\n[fenetre 2]\ndocument = {}\n", py.display(), md.display()));
            app.commun.theme.sombre = true;
        }
        if s.name == "texte-selection" {
            // Un doigt glisse dans le texte : la sélection et ses actions
            let f = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tampon.rs");
            app.ouvrir_espace(&format!("fenetres = F1:texte\n[fenetre 1]\ndocument = {f}\ntaille = 17\n"));
        }
        if s.name == "fichiers-nouveau" {
            // La boîte « Nouveau », au-dessus de Fichiers, en sombre
            app.ouvrir_espace(&format!("fenetres = F1:fichiers\n[fenetre 1]\ndossier = {}\n", env!("CARGO_MANIFEST_DIR")));
            app.commun.theme.sombre = true;
        }
        if s.name == "fichiers-selection" {
            // Fichiers sur le projet, trié par taille : un appui long sélectionne, un toucher ajoute
            app.ouvrir_espace(&format!("fenetres = F1:fichiers\n[fenetre 1]\ndossier = {}\ntri = Taille\n", env!("CARGO_MANIFEST_DIR")));
        }
        if s.name == "svg-simplifier" {
            // Une tache et une vague tracées à main levée (tremblées, très denses)
            let d = std::env::temp_dir().join("flow-apercu-svg");
            let _ = std::fs::create_dir_all(&d);
            let f = d.join("main-levee.svg");
            let mut tache = String::new();
            for k in 0..=240 {
                let a = k as f32 / 240.0 * std::f32::consts::TAU;
                let r = 70.0 + 12.0 * (3.0 * a).sin() + 0.6 * (k as f32 * 2.3).sin();
                tache += &format!("{}{:.1} {:.1}", if k == 0 { "M" } else { " L" }, 200.0 + r * a.cos(), 150.0 + r * a.sin());
            }
            let mut vague = String::new();
            for k in 0..=200 {
                let x = 40.0 + k as f32 * 1.6;
                let y = 265.0 + 14.0 * (x / 30.0).sin() + 0.5 * (k as f32 * 1.9).sin() + if x > 200.0 { 0.0 } else { (200.0 - x) * 0.1 };
                vague += &format!("{}{:.1} {:.1}", if k == 0 { "M" } else { " L" }, x, y);
            }
            std::fs::write(&f, format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 400 300\" width=\"400\" height=\"300\">\n<g stroke-linecap=\"round\" stroke-linejoin=\"round\">\n<path d=\"{tache} Z\" fill=\"#facc15\" stroke=\"#1e1e25\" stroke-width=\"3\"/>\n<path d=\"{vague}\" fill=\"none\" stroke=\"#2563eb\" stroke-width=\"4\"/>\n</g>\n</svg>\n")).unwrap();
            app.ouvrir_espace(&format!("fenetres = F1:svg\n[fenetre 1]\ndocument = {}\nsimplifier = 0 0\n", f.display()));
        } else if s.name.starts_with("svg") {
            // Un dessin d'essai : groupe tourné, chemin à arcs et courbes, dégradé, texte
            let d = std::env::temp_dir().join("flow-apercu-svg");
            let _ = std::fs::remove_dir_all(&d);
            let _ = std::fs::create_dir_all(&d);
            let f = d.join("essai.svg");
            std::fs::write(&f, r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300" width="400" height="300">
  <defs>
    <linearGradient id="ciel"><stop offset="0" stop-color="#93c5fd"/><stop offset="1" stop-color="#e0f2fe"/></linearGradient>
  </defs>
  <rect width="400" height="300" fill="url(#ciel)"/>
  <circle cx="320" cy="60" r="30" fill="#facc15" stroke="#ea8a0c" stroke-width="4"/>
  <path d="M0 230 Q 100 160 200 220 T 400 210 L 400 300 L 0 300 Z" fill="#16a34a"/>
  <g transform="rotate(-8 120 170)" fill="#dc2626" stroke="#1e1e25" stroke-width="3">
    <rect x="70" y="150" width="100" height="70"/>
    <polygon points="60,150 120,100 180,150" fill="#9333ea"/>
  </g>
  <path d="M250 120 a40 25 0 1 0 80 0 a40 25 0 1 0 -80 0" fill="none" stroke="#2563eb" stroke-width="5" stroke-dasharray="4"/>
  <text x="200" y="285" font-size="22" text-anchor="middle" fill="#ffffff">Flow · SVG</text>
</svg>
"##).unwrap();
            app.ouvrir_espace(&format!("fenetres = F1:svg\n[fenetre 1]\ndocument = {}\n", f.display()));
            app.commun.theme.sombre = s.name == "svg-portrait";
        }
        if s.name.starts_with("argile") {
            // Une boule fraîche, quelques coups de pinceau (en sombre pour le portrait)
            app.ouvrir_espace("fenetres = F1:argile\n");
            app.commun.theme.sombre = s.name == "argile-portrait";
        }
        if s.name == "accueil" {
            // L'Accueil à côté d'une fenêtre Encre, née du menu
            app.ouvrir_espace("fenetres = C0.6200(F1:accueil,F2:orchestre)\n");
        }
        let mut scene = None;
        for i in 0..60 {
            let t = i as f64 * 0.05;
            ui.horloge(t);
            match (s.gestes, i) {
                // Trois traits en spirale, puis un tracé en cours
                (1, 2..=16) => {
                    let n = (i - 2) / 5;
                    let k = ((i - 2) % 5) as f32;
                    let (x0, y0) = (300.0 + 300.0 * n as f32, 300.0);
                    let t0 = t - 0.01;
                    match k as i32 {
                        0 => ui.event(Event::Down { id: 0, x: x0, y: y0 }, t0),
                        4 => ui.event(Event::Up { id: 0 }, t0),
                        _ => {
                            for q in 0..12 {
                                let a = (k - 1.0) * 2.0 + q as f32 * 0.18;
                                ui.event(Event::Move { id: 0, x: x0 + 120.0 + a.cos() * (40.0 + 12.0 * a), y: y0 + 200.0 + a.sin() * (40.0 + 12.0 * a) }, t0);
                            }
                        }
                    }
                }
                (1, 20) => ui.event(Event::Down { id: 0, x: 420.0, y: 700.0 }, t),
                (1, 21..=40) => {
                    let k = (i - 20) as f32;
                    ui.event(Event::Move { id: 0, x: 420.0 + 24.0 * k, y: 700.0 + 9.0 * k - 0.5 * k * k }, t)
                }
                (2, 30) => ui.event(Event::Down { id: 0, x: 300.0, y: 500.0 }, t),
                (3, 10) => ui.event(Event::Down { id: 0, x: 430.0, y: 700.0 }, t),
                (3, 11..=20) => {
                    let k = (i - 10) as f32;
                    ui.event(Event::Move { id: 0, x: 430.0 + 50.0 * k, y: 700.0 + 26.0 * k }, t)
                }
                (3, 21) => ui.event(Event::Up { id: 0 }, t),
                (5, 10) => ui.event(Event::Down { id: 0, x: 1850.0, y: 136.0 }, t),
                (5, 11) => ui.event(Event::Up { id: 0 }, t),
                (6, 10) => ui.event(Event::Down { id: 0, x: SVG_TAP.0, y: SVG_TAP.1 }, t),
                (6, 11) => ui.event(Event::Up { id: 0 }, t),
                // La pilule « Source »
                (7, 10) => ui.event(Event::Down { id: 0, x: 1664.0, y: 58.0 }, t),
                (7, 11) => ui.event(Event::Up { id: 0 }, t),
                // L'outil Sommets, puis le soleil, un de ses sommets, tiré
                (8, 10 | 20 | 30) => ui.event(Event::Down { id: 0, x: SOMMETS[(i / 10 - 1) as usize].0, y: SOMMETS[(i / 10 - 1) as usize].1 }, t),
                (8, 11 | 21 | 31) => ui.event(Event::Up { id: 0 }, t),
                (8, 40) => ui.event(Event::Down { id: 0, x: SOMMETS[3].0, y: SOMMETS[3].1 }, t),
                (8, 41..=50) => {
                    let k = (i - 40) as f32;
                    ui.event(Event::Move { id: 0, x: SOMMETS[3].0 + 22.0 * k, y: SOMMETS[3].1 - 6.0 * k }, t)
                }
                // La tache, « Simplifier », un écart, « Courbes »
                (10, 10 | 20 | 30 | 40) if SIMPLIFIER[(i / 10 - 1) as usize].0 > 0.0 => ui.event(Event::Down { id: 0, x: SIMPLIFIER[(i / 10 - 1) as usize].0, y: SIMPLIFIER[(i / 10 - 1) as usize].1 }, t),
                (10, 11 | 21 | 31 | 41) => ui.event(Event::Up { id: 0 }, t),
                // Trois traits sur la boule, le dernier en cours (le pinceau se voit)
                (9, 5 | 20 | 35) => {
                    let k = (i / 15) as f32;
                    ui.event(Event::Down { id: 0, x: w * (0.40 + 0.04 * k), y: h * (0.36 + 0.05 * k) }, t)
                }
                (9, 6..=16 | 21..=31 | 36..=46) => {
                    let k = (i / 15) as f32;
                    let j = ((i - 1) % 15) as f32 - 4.0;
                    ui.event(Event::Move { id: 0, x: w * (0.40 + 0.04 * k) + j * 14.0 * s.dp, y: h * (0.36 + 0.05 * k) - j * 3.0 * s.dp }, t)
                }
                (9, 17 | 32) => ui.event(Event::Up { id: 0 }, t),
                (4, 10) => ui.event(Event::Down { id: 0, x: 700.0, y: 330.0 }, t),
                (4, 25) => ui.event(Event::Up { id: 0 }, t),
                (4, 30) => ui.event(Event::Down { id: 0, x: 1100.0, y: 330.0 }, t),
                (4, 31) => ui.event(Event::Up { id: 0 }, t),
                // Puis la poignée de fin descend de quelques lignes, tenue
                (3, 30) => ui.event(Event::Down { id: 0, x: 360.0, y: 950.0 }, t),
                (3, 31..=40) => {
                    let k = (i - 30) as f32;
                    ui.event(Event::Move { id: 0, x: 360.0 + 30.0 * k, y: 950.0 + 16.0 * k }, t)
                }
                _ => {}
            }
            if i == 30 && s.name == "accueil-menu" {
                // Le titre de la fenêtre ouvre son menu
                let c = 48.0 * s.dp;
                ui.event(Event::Down { id: 0, x: 80.0, y: h - c - 0.375 * c }, t);
                ui.event(Event::Up { id: 0 }, t + 0.02);
            }
            if i == 30 && s.help {
                // Vraie séquence tactile : appui sur la pastille d'aide, dans la barre du bas.
                let c = 48.0 * s.dp;
                ui.event(Event::Down { id: 0, x: 2.5 * c, y: h - c / 2.0 }, 0.0);
                ui.event(Event::Up { id: 0 }, 0.05);
            }
            dl.clear();
            let mut cx = Cx { ui: &mut ui, text: &mut text, dl: &mut dl, dp: s.dp, theme: Default::default() };
            scene = Some(app.frame(&mut cx, screen, 0.05));
            ui.end_frame();
            if s.disque.is_some() {
                // Laisse l'analyse finir
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
        }
        render.draw(w, h, scene.unwrap().fond, &dl, &mut text);

        let gl = render.gl();
        let mut px = vec![0u8; (s.w * s.h * 4) as usize];
        unsafe {
            (gl.glReadPixels)(0, 0, s.w, s.h, 0x1908, 0x1401, px.as_mut_ptr() as *mut _);
            let e = (gl.glGetError)();
            println!("{} : {} sommets, erreur GL {e:#x}", s.name, dl.vertices.len());
        }
        let mut out = format!("P6\n{} {}\n255\n", s.w, s.h).into_bytes();
        for y in (0..s.h as usize).rev() {
            for x in 0..s.w as usize {
                out.extend_from_slice(&px[(y * s.w as usize + x) * 4..][..3]);
            }
        }
        std::fs::write(format!("{dir}/{}.ppm", s.name), out).unwrap();
    }
}
