//! Flow : application Android en Rust, née d'une copie de Rustine (interface tactile et 3D).
//!
//! - `gles`, `renderer`, `android` : ce qui touche à OpenGL ES et à Android ;
//! - `draw`, `text`, `ui`, `widgets`, `shell` : l'interface, sans aucun appel
//!   OpenGL, donc testable sur n'importe quelle machine ;
//! - `contexte`, `contextes` : les sous-applications que montrent les fenêtres ;
//! - `espace` : l'espace de travail enregistré dans un fichier texte ;
//! - `app` : l'application, qui réunit tout.

pub mod acces;
pub mod app;
pub mod android;
pub mod audio;
pub mod clavier;
pub mod contexte;
pub mod contextes;
pub mod disque;
pub mod draw;
pub mod espace;
pub mod explorateur;
pub mod gles;
pub mod mixage;
pub mod parametres;
pub mod radio;
pub mod reglages;
pub mod renderer;
pub mod shell;
pub mod surface;
pub mod tampon;
pub mod text;
pub mod ui;
pub mod widgets;
