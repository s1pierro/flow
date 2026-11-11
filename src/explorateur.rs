//! L'explorateur de fichiers, sans interface : le contenu d'un dossier, trié (les
//! dossiers d'abord, puis par nom sans tenir compte de la casse ni des accents), la
//! nature de chaque fichier, les tailles en clair et le fil d'Ariane d'un chemin.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Ce qu'on sait faire d'un fichier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nature {
    Dossier,
    Audio,
    Image,
    Video,
    Texte,
    Autre,
}

impl Nature {
    pub fn de(nom: &str, dossier: bool) -> Nature {
        if dossier {
            return Nature::Dossier;
        }
        let ext = nom.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "mp3" | "ogg" | "oga" | "opus" | "flac" | "m4a" | "aac" | "wav" | "amr" | "mka" | "3gp" => Nature::Audio,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "heic" | "avif" | "svg" => Nature::Image,
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "ts" => Nature::Video,
            "txt" | "md" | "rs" | "toml" | "json" | "xml" | "html" | "css" | "js" | "sh" | "py" | "csv" | "log" | "ini" | "conf" | "java" | "c" | "h" => Nature::Texte,
            _ => Nature::Autre,
        }
    }
}

/// La famille d'un type MIME, pour la teinte de son badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Famille {
    Audio,
    Image,
    Video,
    Texte,
    Document,
    Archive,
    Application,
    Inconnue,
}

/// Le type MIME d'un fichier, d'après son extension, et sa famille.
pub fn mime(nom: &str) -> (&'static str, Famille) {
    use Famille::*;
    let ext = nom.rsplit_once('.').filter(|(a, _)| !a.is_empty()).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "mp3" => ("audio/mpeg", Audio),
        "ogg" | "oga" => ("audio/ogg", Audio),
        "opus" => ("audio/opus", Audio),
        "flac" => ("audio/flac", Audio),
        "m4a" => ("audio/mp4", Audio),
        "aac" => ("audio/aac", Audio),
        "wav" => ("audio/wav", Audio),
        "amr" => ("audio/amr", Audio),
        "mka" => ("audio/x-matroska", Audio),
        "mid" | "midi" => ("audio/midi", Audio),
        "png" => ("image/png", Image),
        "jpg" | "jpeg" => ("image/jpeg", Image),
        "gif" => ("image/gif", Image),
        "webp" => ("image/webp", Image),
        "bmp" => ("image/bmp", Image),
        "heic" => ("image/heic", Image),
        "avif" => ("image/avif", Image),
        "svg" => ("image/svg+xml", Image),
        "ico" => ("image/x-icon", Image),
        "mp4" | "m4v" => ("video/mp4", Video),
        "mkv" => ("video/x-matroska", Video),
        "webm" => ("video/webm", Video),
        "mov" => ("video/quicktime", Video),
        "avi" => ("video/x-msvideo", Video),
        "3gp" => ("video/3gpp", Video),
        "ts" => ("video/mp2t", Video),
        "txt" | "log" | "conf" | "ini" => ("text/plain", Texte),
        "md" => ("text/markdown", Texte),
        "csv" => ("text/csv", Texte),
        "html" | "htm" => ("text/html", Texte),
        "css" => ("text/css", Texte),
        "js" => ("text/javascript", Texte),
        "json" => ("application/json", Texte),
        "xml" => ("application/xml", Texte),
        "toml" => ("application/toml", Texte),
        "rs" => ("text/x-rust", Texte),
        "py" => ("text/x-python", Texte),
        "sh" => ("application/x-sh", Texte),
        "java" => ("text/x-java", Texte),
        "c" | "h" => ("text/x-c", Texte),
        "pdf" => ("application/pdf", Document),
        "odt" => ("application/vnd.oasis.opendocument.text", Document),
        "ods" => ("application/vnd.oasis.opendocument.spreadsheet", Document),
        "doc" => ("application/msword", Document),
        "docx" => ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", Document),
        "xlsx" => ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", Document),
        "epub" => ("application/epub+zip", Document),
        "zip" => ("application/zip", Archive),
        "gz" | "tgz" => ("application/gzip", Archive),
        "tar" => ("application/x-tar", Archive),
        "xz" => ("application/x-xz", Archive),
        "7z" => ("application/x-7z-compressed", Archive),
        "rar" => ("application/vnd.rar", Archive),
        "apk" => ("application/vnd.android.package-archive", Application),
        "jar" => ("application/java-archive", Application),
        "so" => ("application/x-sharedlib", Application),
        "glb" => ("model/gltf-binary", Application),
        "gltf" => ("model/gltf+json", Application),
        _ => ("application/octet-stream", Inconnue),
    }
}

/// Ce qu'affiche le badge d'un fichier : son extension en capitales (4 lettres au
/// plus), ou « ? » sans extension.
pub fn badge(nom: &str) -> String {
    match nom.rsplit_once('.').filter(|(a, e)| !a.is_empty() && !e.is_empty()) {
        Some((_, e)) => e.chars().take(4).collect::<String>().to_uppercase(),
        None => "?".into(),
    }
}

#[derive(Clone, Debug)]
pub struct Entree {
    pub nom: String,
    pub nature: Nature,
    /// Taille en octets (0 pour un dossier).
    pub taille: u64,
    pub modifie: Option<SystemTime>,
}

impl Entree {
    pub fn dossier(&self) -> bool {
        self.nature == Nature::Dossier
    }
}

/// Le contenu d'un dossier, lu une fois et gardé jusqu'à la prochaine lecture.
#[derive(Clone, Debug, Default)]
pub struct Dossier {
    pub chemin: PathBuf,
    pub entrees: Vec<Entree>,
    pub erreur: Option<String>,
}

impl Dossier {
    /// Lit `chemin`. Les fichiers cachés (commençant par un point) sont écartés sauf
    /// demande. Une erreur (accès refusé…) laisse la liste vide et se lit dans `erreur`.
    pub fn lire(chemin: &Path, caches: bool) -> Dossier {
        let mut d = Dossier { chemin: chemin.to_path_buf(), ..Dossier::default() };
        match std::fs::read_dir(chemin) {
            Ok(it) => {
                for e in it.flatten() {
                    let nom = e.file_name().to_string_lossy().into_owned();
                    if !caches && nom.starts_with('.') {
                        continue;
                    }
                    // Un lien suit sa cible : un lien vers un dossier s'ouvre comme lui
                    let meta = std::fs::metadata(e.path()).or_else(|_| e.metadata());
                    let (dossier, taille, modifie) = match &meta {
                        Ok(m) => (m.is_dir(), if m.is_dir() { 0 } else { m.len() }, m.modified().ok()),
                        Err(_) => (false, 0, None),
                    };
                    d.entrees.push(Entree { nature: Nature::de(&nom, dossier), nom, taille, modifie });
                }
                trier(&mut d.entrees);
            }
            Err(e) => {
                d.erreur = Some(match e.kind() {
                    std::io::ErrorKind::PermissionDenied => "Accès refusé".to_string(),
                    std::io::ErrorKind::NotFound => "Dossier introuvable".to_string(),
                    _ => e.to_string(),
                })
            }
        }
        d
    }

    pub fn compte(&self) -> (usize, usize) {
        let n = self.entrees.iter().filter(|e| e.dossier()).count();
        (n, self.entrees.len() - n)
    }
}

/// Les dossiers d'abord, puis l'ordre alphabétique, sans casse ni accents, les nombres
/// dans l'ordre naturel (« piste 2 » avant « piste 10 »).
pub fn trier(l: &mut [Entree]) {
    l.sort_by(|a, b| b.dossier().cmp(&a.dossier()).then_with(|| comparer(&a.nom, &b.nom)));
}

/// L'ordre des entrées : les dossiers toujours d'abord, puis selon le critère.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tri {
    /// Par nom, dans l'ordre naturel.
    #[default]
    Nom,
    /// Les plus récents d'abord.
    Date,
    /// Les plus gros d'abord (les dossiers, par nom).
    Taille,
    /// Par extension, puis par nom.
    Type,
}

impl Tri {
    pub const TOUS: [Tri; 4] = [Tri::Nom, Tri::Date, Tri::Taille, Tri::Type];

    pub fn nom(self) -> &'static str {
        match self {
            Tri::Nom => "Nom",
            Tri::Date => "Date",
            Tri::Taille => "Taille",
            Tri::Type => "Type",
        }
    }

    pub fn de(nom: &str) -> Option<Tri> {
        Tri::TOUS.into_iter().find(|t| t.nom().eq_ignore_ascii_case(nom))
    }

    pub fn suivant(self) -> Tri {
        let i = Tri::TOUS.iter().position(|t| *t == self).unwrap_or(0);
        Tri::TOUS[(i + 1) % Tri::TOUS.len()]
    }
}

/// Trie selon `tri`, les dossiers d'abord.
pub fn trier_par(l: &mut [Entree], tri: Tri) {
    use std::cmp::Ordering;
    let ext = |n: &str| n.rsplit_once('.').filter(|(a, _)| !a.is_empty()).map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    l.sort_by(|a, b| {
        let critere = match tri {
            Tri::Nom => Ordering::Equal,
            Tri::Date => b.modifie.cmp(&a.modifie),
            Tri::Taille => b.taille.cmp(&a.taille),
            Tri::Type => ext(&a.nom).cmp(&ext(&b.nom)),
        };
        b.dossier().cmp(&a.dossier()).then(critere).then_with(|| comparer(&a.nom, &b.nom))
    });
}

/// Vrai si `nom` contient `motif`, sans tenir compte de la casse ni des accents.
pub fn correspond(nom: &str, motif: &str) -> bool {
    let motif: Vec<char> = motif.trim().chars().map(cle).collect();
    if motif.is_empty() {
        return true;
    }
    let nom: Vec<char> = nom.chars().map(cle).collect();
    nom.windows(motif.len()).any(|w| w == motif.as_slice())
}

fn cle(c: char) -> char {
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' | 'ì' => 'i',
        'ô' | 'ö' | 'ó' | 'ò' | 'õ' => 'o',
        'û' | 'ü' | 'ú' | 'ù' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        'ÿ' => 'y',
        c => c,
    }
}

/// Comparaison « naturelle » de deux noms.
pub fn comparer(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Less,
            (Some(_), None) => return Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().filter(|c| c.is_ascii_digit()) {
                    n.push(*c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(c) = y.peek().filter(|c| c.is_ascii_digit()) {
                    m.push(*c);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o != Equal {
                    return o;
                }
            }
            (Some(p), Some(q)) => {
                let o = cle(p).cmp(&cle(q));
                if o != Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// Une taille en clair : « 512 o », « 3,4 Ko », « 12 Mo », « 1,2 Go ».
pub fn taille(n: u64) -> String {
    const U: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} o")
    } else if v < 10.0 {
        format!("{:.1} {}", v, U[i]).replace('.', ",")
    } else {
        format!("{:.0} {}", v, U[i])
    }
}

/// Le fil d'Ariane d'un chemin, à partir de la racine du stockage, qui s'appelle
/// « Stockage » : chaque étape avec son nom et son chemin. Hors du stockage, le fil
/// part de « / ».
pub fn ariane(chemin: &Path, racine: Option<&Path>) -> Vec<(String, PathBuf)> {
    let mut l = Vec::new();
    let (depart, reste) = match racine.and_then(|r| chemin.strip_prefix(r).ok().map(|s| (r, s))) {
        Some((r, s)) => {
            l.push(("Stockage".to_string(), r.to_path_buf()));
            (r.to_path_buf(), s.to_path_buf())
        }
        None => {
            l.push(("/".to_string(), PathBuf::from("/")));
            (PathBuf::from("/"), chemin.strip_prefix("/").unwrap_or(chemin).to_path_buf())
        }
    };
    let mut p = depart;
    for c in reste.components() {
        p.push(c);
        l.push((c.as_os_str().to_string_lossy().into_owned(), p.clone()));
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(nom: &str, dossier: bool) -> Entree {
        Entree { nom: nom.into(), nature: Nature::de(nom, dossier), taille: 0, modifie: None }
    }

    #[test]
    fn les_dossiers_d_abord_puis_l_ordre_naturel() {
        let mut l = vec![e("piste 10.mp3", false), e("Zèbre", true), e("été", true), e("piste 2.mp3", false), e("Album", true), e("b.txt", false)];
        trier(&mut l);
        let noms: Vec<&str> = l.iter().map(|e| e.nom.as_str()).collect();
        assert_eq!(noms, ["Album", "été", "Zèbre", "b.txt", "piste 2.mp3", "piste 10.mp3"]);
    }

    #[test]
    fn nature_tailles_et_fil_d_ariane() {
        assert_eq!(Nature::de("Chanson.FLAC", false), Nature::Audio);
        assert_eq!(Nature::de("photo.jpg", false), Nature::Image);
        assert_eq!(Nature::de("sans extension", false), Nature::Autre);
        assert_eq!(Nature::de("musique.mp3", true), Nature::Dossier);
        assert_eq!(taille(512), "512 o");
        assert_eq!(taille(3500), "3,4 Ko");
        assert_eq!(taille(12 * 1024 * 1024 + 5), "12 Mo");
        let r = Path::new("/storage/emulated/0");
        let a = ariane(Path::new("/storage/emulated/0/Music/Album"), Some(r));
        assert_eq!(a.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Stockage", "Music", "Album"]);
        assert_eq!(a[2].1, Path::new("/storage/emulated/0/Music/Album"));
        let a = ariane(Path::new("/system/fonts"), Some(r));
        assert_eq!(a.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["/", "system", "fonts"]);
    }

    #[test]
    fn type_mime_et_badge() {
        assert_eq!(mime("Chanson.FLAC"), ("audio/flac", Famille::Audio));
        assert_eq!(mime("plan.pdf").1, Famille::Document);
        assert_eq!(mime(".bashrc"), ("application/octet-stream", Famille::Inconnue));
        assert_eq!(badge("archive.tar.gz"), "GZ");
        assert_eq!(badge("flow-0.11.apk"), "APK");
        assert_eq!(badge("LISEZMOI"), "?");
        assert_eq!(badge("x.markdown"), "MARK");
    }

    #[test]
    fn trier_par_date_taille_type_et_chercher() {
        use std::time::Duration;
        let t = |s: u64| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(s));
        let mut l = vec![
            Entree { nom: "b.txt".into(), nature: Nature::Texte, taille: 50, modifie: t(30) },
            Entree { nom: "a.mp3".into(), nature: Nature::Audio, taille: 900, modifie: t(10) },
            Entree { nom: "Z".into(), nature: Nature::Dossier, taille: 0, modifie: t(5) },
            Entree { nom: "c.flac".into(), nature: Nature::Audio, taille: 10, modifie: t(20) },
        ];
        let noms = |l: &[Entree]| l.iter().map(|e| e.nom.clone()).collect::<Vec<_>>().join(" ");
        trier_par(&mut l, Tri::Date);
        assert_eq!(noms(&l), "Z b.txt c.flac a.mp3");
        trier_par(&mut l, Tri::Taille);
        assert_eq!(noms(&l), "Z a.mp3 b.txt c.flac");
        trier_par(&mut l, Tri::Type);
        assert_eq!(noms(&l), "Z c.flac a.mp3 b.txt");
        trier_par(&mut l, Tri::Nom);
        assert_eq!(noms(&l), "Z a.mp3 b.txt c.flac");
        assert_eq!(Tri::de("taille"), Some(Tri::Taille));
        assert_eq!(Tri::Type.suivant(), Tri::Nom);
        assert!(correspond("Été 2024.JPG", "ete 20"));
        assert!(correspond("x", "  "));
        assert!(!correspond("photo", "photos"));
    }

    #[test]
    fn un_dossier_illisible_le_dit() {
        let d = Dossier::lire(Path::new("/dossier/qui/n/existe/pas"), false);
        assert_eq!(d.erreur.as_deref(), Some("Dossier introuvable"));
        assert!(d.entrees.is_empty());
    }
}
