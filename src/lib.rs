//! Flow : application Android en Rust, née d'une copie de Rustine (interface tactile et 3D).
//!
//! - `gles`, `renderer`, `android` : ce qui touche à OpenGL ES et à Android ;
//! - `draw`, `text`, `ui`, `widgets`, `shell` : l'interface, sans aucun appel
//!   OpenGL, donc testable sur n'importe quelle machine ;
//! - `demo` : l'application de démonstration.

pub mod acces;
pub mod android;
pub mod audio;
pub mod clavier;
pub mod demo;
pub mod disque;
pub mod draw;
pub mod explorateur;
pub mod gles;
pub mod math;
pub mod mixage;
pub mod radio;
pub mod reglages;
pub mod renderer;
pub mod shell;
pub mod surface;
pub mod text;
pub mod ui;
pub mod widgets;
