//! Les fourches de la voirie (`champ_voirie::fourches`) :
//! `FLOW_FOURCHE="x z" cargo run --release --example fourches -- lieu.reel`.
use flow::carte::Donnees;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let l = sage_reel::Lieu::lire(std::path::Path::new(&a[1])).expect("lieu");
    let d = Donnees::new(l);
    let _ = d.champ_voirie();
}
