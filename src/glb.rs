//! Le glTF binaire (`.glb`) : écrire un maillage (positions, normales, triangles,
//! une couleur de matière), et lire celui d'un fichier.
//!
//! La lecture prend les primitives triangles de tous les maillages, avec leurs
//! transformations de nœuds, et soude les sommets confondus (un fichier les sépare
//! souvent aux coutures de ses textures : sculpté tel quel, il se déchirerait). Les
//! normales sont refaites, les textures ignorées.

use crate::sculpture::V3;
use std::collections::HashMap;

/// Une valeur JSON.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Nul,
    Bool(bool),
    Nombre(f64),
    Texte(String),
    Liste(Vec<Json>),
    Objet(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, k: &str) -> Option<&Json> {
        match self {
            Json::Objet(v) => v.iter().find(|e| e.0 == k).map(|e| &e.1),
            _ => None,
        }
    }

    pub fn at(&self, i: usize) -> Option<&Json> {
        match self {
            Json::Liste(v) => v.get(i),
            _ => None,
        }
    }

    pub fn liste(&self) -> &[Json] {
        match self {
            Json::Liste(v) => v,
            _ => &[],
        }
    }

    pub fn nombre(&self) -> Option<f64> {
        match self {
            Json::Nombre(n) => Some(*n),
            _ => None,
        }
    }

    pub fn entier(&self, k: &str) -> Option<usize> {
        self.get(k)?.nombre().filter(|n| *n >= 0.0).map(|n| n as usize)
    }

    pub fn texte(&self) -> Option<&str> {
        match self {
            Json::Texte(s) => Some(s),
            _ => None,
        }
    }

    pub fn lire(s: &str) -> Result<Json, String> {
        let b = s.as_bytes();
        let mut i = 0;
        let v = valeur(b, &mut i)?;
        blancs(b, &mut i);
        if i != b.len() {
            return Err(format!("JSON : reste inattendu à l'octet {i}"));
        }
        Ok(v)
    }

    pub fn ecrire(&self) -> String {
        match self {
            Json::Nul => "null".into(),
            Json::Bool(b) => b.to_string(),
            Json::Nombre(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Json::Texte(s) => {
                let mut o = String::from("\"");
                for c in s.chars() {
                    match c {
                        '"' => o.push_str("\\\""),
                        '\\' => o.push_str("\\\\"),
                        '\n' => o.push_str("\\n"),
                        c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                        c => o.push(c),
                    }
                }
                o.push('"');
                o
            }
            Json::Liste(v) => format!("[{}]", v.iter().map(Json::ecrire).collect::<Vec<_>>().join(",")),
            Json::Objet(v) => format!("{{{}}}", v.iter().map(|(k, x)| format!("{}:{}", Json::Texte(k.clone()).ecrire(), x.ecrire())).collect::<Vec<_>>().join(",")),
        }
    }
}

fn blancs(b: &[u8], i: &mut usize) {
    while *i < b.len() && b[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

fn valeur(b: &[u8], i: &mut usize) -> Result<Json, String> {
    blancs(b, i);
    let err = |i: usize| format!("JSON mal formé à l'octet {i}");
    match b.get(*i).copied() {
        Some(b'{') => {
            *i += 1;
            let mut v = Vec::new();
            loop {
                blancs(b, i);
                if b.get(*i) == Some(&b'}') {
                    *i += 1;
                    return Ok(Json::Objet(v));
                }
                let Json::Texte(k) = valeur(b, i)? else { return Err(err(*i)) };
                blancs(b, i);
                if b.get(*i) != Some(&b':') {
                    return Err(err(*i));
                }
                *i += 1;
                v.push((k, valeur(b, i)?));
                blancs(b, i);
                match b.get(*i) {
                    Some(b',') => *i += 1,
                    Some(b'}') => {}
                    _ => return Err(err(*i)),
                }
            }
        }
        Some(b'[') => {
            *i += 1;
            let mut v = Vec::new();
            loop {
                blancs(b, i);
                if b.get(*i) == Some(&b']') {
                    *i += 1;
                    return Ok(Json::Liste(v));
                }
                v.push(valeur(b, i)?);
                blancs(b, i);
                match b.get(*i) {
                    Some(b',') => *i += 1,
                    Some(b']') => {}
                    _ => return Err(err(*i)),
                }
            }
        }
        Some(b'"') => {
            *i += 1;
            let mut o: Vec<u8> = Vec::new();
            loop {
                match b.get(*i).copied() {
                    None => return Err(err(*i)),
                    Some(b'"') => {
                        *i += 1;
                        return String::from_utf8(o).map(Json::Texte).map_err(|_| err(*i));
                    }
                    Some(b'\\') => {
                        let c = b.get(*i + 1).copied().ok_or_else(|| err(*i))?;
                        *i += 2;
                        let ch = match c {
                            b'n' => '\n',
                            b't' => '\t',
                            b'r' => '\r',
                            b'b' => '\u{8}',
                            b'f' => '\u{c}',
                            b'u' => {
                                let h = std::str::from_utf8(b.get(*i..*i + 4).ok_or_else(|| err(*i))?).map_err(|_| err(*i))?;
                                *i += 4;
                                char::from_u32(u32::from_str_radix(h, 16).map_err(|_| err(*i))?).unwrap_or('\u{fffd}')
                            }
                            c => c as char,
                        };
                        let mut t = [0u8; 4];
                        o.extend_from_slice(ch.encode_utf8(&mut t).as_bytes());
                    }
                    Some(c) => {
                        o.push(c);
                        *i += 1;
                    }
                }
            }
        }
        Some(b't') if b[*i..].starts_with(b"true") => {
            *i += 4;
            Ok(Json::Bool(true))
        }
        Some(b'f') if b[*i..].starts_with(b"false") => {
            *i += 5;
            Ok(Json::Bool(false))
        }
        Some(b'n') if b[*i..].starts_with(b"null") => {
            *i += 4;
            Ok(Json::Nul)
        }
        Some(c) if c == b'-' || c.is_ascii_digit() => {
            let debut = *i;
            *i += 1;
            while *i < b.len() && (b[*i].is_ascii_digit() || matches!(b[*i], b'.' | b'e' | b'E' | b'+' | b'-')) {
                *i += 1;
            }
            std::str::from_utf8(&b[debut..*i]).ok().and_then(|s| s.parse().ok()).map(Json::Nombre).ok_or_else(|| err(debut))
        }
        _ => Err(err(*i)),
    }
}

fn n(v: f64) -> Json {
    Json::Nombre(v)
}

fn o(v: Vec<(&str, Json)>) -> Json {
    Json::Objet(v.into_iter().map(|(k, x)| (k.to_string(), x)).collect())
}

/// Un maillage en glTF binaire : une scène, un nœud nommé `nom`, une matière mate
/// de couleur `couleur` (0 à 1).
pub fn ecrire(positions: &[V3], normales: &[V3], triangles: &[[u32; 3]], couleur: [f32; 3], nom: &str) -> Vec<u8> {
    let nv = positions.len();
    let mut bin: Vec<u8> = Vec::with_capacity(nv * 24 + triangles.len() * 12);
    let (mut a, mut b) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in positions {
        for k in 0..3 {
            a[k] = a[k].min(p[k]);
            b[k] = b[k].max(p[k]);
            bin.extend_from_slice(&p[k].to_le_bytes());
        }
    }
    for p in normales {
        for k in 0..3 {
            bin.extend_from_slice(&p[k].to_le_bytes());
        }
    }
    for t in triangles {
        for k in 0..3 {
            bin.extend_from_slice(&t[k].to_le_bytes());
        }
    }
    let (lp, li) = (nv * 12, triangles.len() * 12);
    let v3 = |v: [f32; 3]| Json::Liste(v.iter().map(|x| n(*x as f64)).collect());
    let json = o(vec![
        ("asset", o(vec![("version", Json::Texte("2.0".into())), ("generator", Json::Texte("Flow · Argile".into()))])),
        ("scene", n(0.0)),
        ("scenes", Json::Liste(vec![o(vec![("nodes", Json::Liste(vec![n(0.0)]))])])),
        ("nodes", Json::Liste(vec![o(vec![("mesh", n(0.0)), ("name", Json::Texte(nom.into()))])])),
        (
            "meshes",
            Json::Liste(vec![o(vec![
                ("name", Json::Texte(nom.into())),
                ("primitives", Json::Liste(vec![o(vec![("attributes", o(vec![("POSITION", n(0.0)), ("NORMAL", n(1.0))])), ("indices", n(2.0)), ("material", n(0.0))])])),
            ])]),
        ),
        (
            "materials",
            Json::Liste(vec![o(vec![
                ("name", Json::Texte("argile".into())),
                ("pbrMetallicRoughness", o(vec![("baseColorFactor", Json::Liste(vec![n(couleur[0] as f64), n(couleur[1] as f64), n(couleur[2] as f64), n(1.0)])), ("metallicFactor", n(0.0)), ("roughnessFactor", n(0.9))])),
            ])]),
        ),
        ("buffers", Json::Liste(vec![o(vec![("byteLength", n(bin.len() as f64))])])),
        (
            "bufferViews",
            Json::Liste(vec![
                o(vec![("buffer", n(0.0)), ("byteOffset", n(0.0)), ("byteLength", n(lp as f64)), ("target", n(34962.0))]),
                o(vec![("buffer", n(0.0)), ("byteOffset", n(lp as f64)), ("byteLength", n(lp as f64)), ("target", n(34962.0))]),
                o(vec![("buffer", n(0.0)), ("byteOffset", n(2.0 * lp as f64)), ("byteLength", n(li as f64)), ("target", n(34963.0))]),
            ]),
        ),
        (
            "accessors",
            Json::Liste(vec![
                o(vec![("bufferView", n(0.0)), ("componentType", n(5126.0)), ("count", n(nv as f64)), ("type", Json::Texte("VEC3".into())), ("min", v3(a)), ("max", v3(b))]),
                o(vec![("bufferView", n(1.0)), ("componentType", n(5126.0)), ("count", n(nv as f64)), ("type", Json::Texte("VEC3".into()))]),
                o(vec![("bufferView", n(2.0)), ("componentType", n(5125.0)), ("count", n((triangles.len() * 3) as f64)), ("type", Json::Texte("SCALAR".into()))]),
            ]),
        ),
    ]);
    let mut j = json.ecrire().into_bytes();
    while j.len() % 4 != 0 {
        j.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + j.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&j);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// Ce qu'on lit d'un `.glb` : des sommets soudés, des triangles, et la couleur de
/// la première matière s'il y en a une.
pub struct Lu {
    pub positions: Vec<V3>,
    pub triangles: Vec<[u32; 3]>,
    pub couleur: Option<[f32; 3]>,
}

/// La matrice locale d'un nœud (en colonnes).
fn matrice_noeud(nd: &Json) -> [f32; 16] {
    if let Some(m) = nd.get("matrix") {
        let v: Vec<f32> = m.liste().iter().filter_map(|x| x.nombre()).map(|x| x as f32).collect();
        if let Ok(a) = <[f32; 16]>::try_from(v) {
            return a;
        }
    }
    let lire = |k: &str, d: &[f32]| -> Vec<f32> {
        let v: Vec<f32> = nd.get(k).map(|x| x.liste().iter().filter_map(|y| y.nombre()).map(|y| y as f32).collect()).unwrap_or_default();
        if v.len() == d.len() { v } else { d.to_vec() }
    };
    let t = lire("translation", &[0.0, 0.0, 0.0]);
    let q = lire("rotation", &[0.0, 0.0, 0.0, 1.0]);
    let s = lire("scale", &[1.0, 1.0, 1.0]);
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let r = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
    ];
    [r[0] * s[0], r[1] * s[0], r[2] * s[0], 0.0, r[3] * s[1], r[4] * s[1], r[5] * s[1], 0.0, r[6] * s[2], r[7] * s[2], r[8] * s[2], 0.0, t[0], t[1], t[2], 1.0]
}

pub fn lire(octets: &[u8]) -> Result<Lu, String> {
    let u32_a = |i: usize| octets.get(i..i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
    if octets.get(..4) != Some(b"glTF") {
        return Err("Ce n'est pas un glTF binaire (.glb).".into());
    }
    let lj = u32_a(12).ok_or("glb tronqué")?;
    if octets.get(16..20) != Some(b"JSON") {
        return Err("glb : le premier bloc n'est pas du JSON.".into());
    }
    let texte = std::str::from_utf8(octets.get(20..20 + lj).ok_or("glb tronqué")?).map_err(|_| "glb : JSON illisible")?;
    let json = Json::lire(texte)?;
    let debut_bin = 20 + lj;
    let bin: &[u8] = match (u32_a(debut_bin), octets.get(debut_bin + 4..debut_bin + 8)) {
        (Some(l), Some(b"BIN\0")) => octets.get(debut_bin + 8..debut_bin + 8 + l).ok_or("glb : bloc binaire tronqué")?,
        _ => &[],
    };
    let vues = json.get("bufferViews").map(Json::liste).unwrap_or_default();
    let accesseurs = json.get("accessors").map(Json::liste).unwrap_or_default();
    // Les nombres d'un accesseur, en flottants, par élément
    let lire_acc = |k: usize| -> Result<Vec<Vec<f32>>, String> {
        let a = accesseurs.get(k).ok_or("accesseur absent")?;
        let compte = a.entier("count").unwrap_or(0);
        let largeur = match a.get("type").and_then(Json::texte) {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            _ => return Err("type d'accesseur inconnu".into()),
        };
        let genre = a.entier("componentType").unwrap_or(0);
        let taille = match genre {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err("composantes inconnues".into()),
        };
        let Some(v) = a.entier("bufferView").and_then(|i| vues.get(i)) else {
            return Ok(vec![vec![0.0; largeur]; compte]);
        };
        if v.entier("buffer").unwrap_or(0) != 0 {
            return Err("glb : tampon externe non pris en charge".into());
        }
        let base = v.entier("byteOffset").unwrap_or(0) + a.entier("byteOffset").unwrap_or(0);
        let pas = v.entier("byteStride").filter(|s| *s > 0).unwrap_or(largeur * taille);
        let mut out = Vec::with_capacity(compte);
        for e in 0..compte {
            let mut el = Vec::with_capacity(largeur);
            for c in 0..largeur {
                let i = base + e * pas + c * taille;
                let b = bin.get(i..i + taille).ok_or("glb : données hors du tampon")?;
                el.push(match genre {
                    5126 => f32::from_le_bytes(b.try_into().unwrap()),
                    5125 => u32::from_le_bytes(b.try_into().unwrap()) as f32,
                    5123 => u16::from_le_bytes(b.try_into().unwrap()) as f32,
                    5122 => i16::from_le_bytes(b.try_into().unwrap()) as f32,
                    5121 => b[0] as f32,
                    _ => b[0] as i8 as f32,
                });
            }
            out.push(el);
        }
        Ok(out)
    };
    // Les indices restent entiers (un f32 perd les grands u32)
    let lire_indices = |k: usize| -> Result<Vec<u32>, String> {
        let a = accesseurs.get(k).ok_or("accesseur absent")?;
        let compte = a.entier("count").unwrap_or(0);
        let genre = a.entier("componentType").unwrap_or(0);
        let taille = match genre {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            _ => return Err("indices de type inconnu".into()),
        };
        let v = a.entier("bufferView").and_then(|i| vues.get(i)).ok_or("indices sans vue")?;
        let base = v.entier("byteOffset").unwrap_or(0) + a.entier("byteOffset").unwrap_or(0);
        (0..compte)
            .map(|e| {
                let i = base + e * taille;
                let b = bin.get(i..i + taille).ok_or("glb : indices hors du tampon")?;
                Ok(match taille {
                    1 => b[0] as u32,
                    2 => u16::from_le_bytes(b.try_into().unwrap()) as u32,
                    _ => u32::from_le_bytes(b.try_into().unwrap()),
                })
            })
            .collect()
    };

    // Les maillages, par les nœuds des scènes (avec leurs transformations)
    let noeuds = json.get("nodes").map(Json::liste).unwrap_or_default();
    let maillages = json.get("meshes").map(Json::liste).unwrap_or_default();
    let mut a_faire: Vec<(usize, [f32; 16])> = Vec::new();
    let racines: Vec<usize> = match json.get("scenes").and_then(|s| s.at(json.entier("scene").unwrap_or(0))) {
        Some(sc) => sc.get("nodes").map(|v| v.liste().iter().filter_map(|x| x.nombre()).map(|x| x as usize).collect()).unwrap_or_default(),
        None => (0..noeuds.len()).collect(),
    };
    let mut pile: Vec<(usize, [f32; 16], u32)> = racines.into_iter().map(|r| (r, crate::sculpture::produit(&IDENTITE, &IDENTITE), 0)).collect();
    while let Some((k, parent, prof)) = pile.pop() {
        let Some(nd) = noeuds.get(k) else { continue };
        if prof > 64 {
            continue;
        }
        let m = crate::sculpture::produit(&parent, &matrice_noeud(nd));
        if let Some(mi) = nd.entier("mesh") {
            a_faire.push((mi, m));
        }
        for e in nd.get("children").map(Json::liste).unwrap_or_default() {
            if let Some(e) = e.nombre() {
                pile.push((e as usize, m, prof + 1));
            }
        }
    }
    if a_faire.is_empty() {
        a_faire = (0..maillages.len()).map(|i| (i, IDENTITE)).collect();
    }
    let mut positions: Vec<V3> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    let mut soudure: HashMap<[i64; 3], u32> = HashMap::new();
    let mut couleur = None;
    for (mi, m) in a_faire {
        let Some(me) = maillages.get(mi) else { continue };
        for pr in me.get("primitives").map(Json::liste).unwrap_or_default() {
            if pr.entier("mode").unwrap_or(4) != 4 {
                continue;
            }
            let Some(pa) = pr.get("attributes").and_then(|a| a.entier("POSITION")) else { continue };
            let pts = lire_acc(pa)?;
            if couleur.is_none() {
                couleur = pr.entier("material").and_then(|k| json.get("materials")?.at(k)?.get("pbrMetallicRoughness")?.get("baseColorFactor").cloned()).and_then(|c| {
                    let v: Vec<f32> = c.liste().iter().filter_map(|x| x.nombre()).map(|x| x as f32).collect();
                    (v.len() >= 3).then(|| [v[0], v[1], v[2]])
                });
            }
            // Chaque point transformé, puis soudé à un point déjà vu au même endroit
            let mut nouveaux = Vec::with_capacity(pts.len());
            for p in &pts {
                if p.len() < 3 {
                    return Err("glb : position incomplète".into());
                }
                let q = [
                    m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
                    m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
                    m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
                ];
                let cle = q.map(|x| (x as f64 * 1e5).round() as i64);
                let i = *soudure.entry(cle).or_insert_with(|| {
                    positions.push(q);
                    positions.len() as u32 - 1
                });
                nouveaux.push(i);
            }
            let ind: Vec<u32> = match pr.entier("indices") {
                Some(k) => lire_indices(k)?,
                None => (0..pts.len() as u32).collect(),
            };
            for t in ind.chunks_exact(3) {
                let g = |i: u32| nouveaux.get(i as usize).copied().ok_or("glb : indice hors limites");
                let tri = [g(t[0])?, g(t[1])?, g(t[2])?];
                // Un triangle écrasé par la soudure disparaît
                if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
                    triangles.push(tri);
                }
            }
        }
    }
    if triangles.is_empty() {
        return Err("Ce glb ne contient aucun triangle.".into());
    }
    Ok(Lu { positions, triangles, couleur })
}

const IDENTITE: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sculpture::Maillage;

    #[test]
    fn le_json_se_lit_et_s_ecrit() {
        let j = Json::lire(r#" {"a": [1, 2.5, -3e2, true, null], "b": "é\"\né", "c": {}} "#).unwrap();
        assert_eq!(j.get("a").unwrap().at(2), Some(&Json::Nombre(-300.0)));
        assert_eq!(j.get("b").and_then(Json::texte), Some("é\"\né"));
        assert_eq!(Json::lire(&j.ecrire()).unwrap(), j);
        assert!(Json::lire("[1,").is_err());
        assert!(Json::lire("{\"a\" 1}").is_err());
    }

    #[test]
    fn un_maillage_fait_l_aller_retour() {
        let m = Maillage::boule(2, 1.0);
        let o = ecrire(&m.positions, &m.normales, &m.triangles, [0.8, 0.4, 0.3], "essai");
        assert_eq!(&o[..4], b"glTF");
        assert_eq!(o.len() % 4, 0);
        let l = lire(&o).unwrap();
        assert_eq!(l.positions, m.positions);
        assert_eq!(&l.triangles, m.triangles.as_ref());
        assert_eq!(l.couleur, Some([0.8, 0.4, 0.3]));
    }

    #[test]
    fn les_coutures_se_soudent_et_les_noeuds_se_placent() {
        // Deux triangles qui partagent une arête, mais sans partager leurs sommets,
        // dans un nœud décalé de 10 en x
        let pts: [f32; 18] = [0., 0., 0., 1., 0., 0., 0., 1., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.];
        let mut bin: Vec<u8> = pts.iter().flat_map(|x| x.to_le_bytes()).collect();
        for i in [0u16, 1, 2, 3, 4, 5] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"translation":[10,0,0],"children":[1]}},{{"mesh":0}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}],"buffers":[{{"byteLength":{}}}],"bufferViews":[{{"buffer":0,"byteLength":72}},{{"buffer":0,"byteOffset":72,"byteLength":12}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":6,"type":"VEC3"}},{{"bufferView":1,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
            bin.len()
        );
        let mut j = json.into_bytes();
        while j.len() % 4 != 0 {
            j.push(b' ');
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let mut o = b"glTF".to_vec();
        o.extend_from_slice(&2u32.to_le_bytes());
        o.extend_from_slice(&((28 + j.len() + bin.len()) as u32).to_le_bytes());
        o.extend_from_slice(&(j.len() as u32).to_le_bytes());
        o.extend_from_slice(b"JSON");
        o.extend_from_slice(&j);
        o.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        o.extend_from_slice(b"BIN\0");
        o.extend_from_slice(&bin);
        let l = lire(&o).unwrap();
        assert_eq!(l.positions.len(), 4);
        assert_eq!(l.triangles.len(), 2);
        assert!(l.positions.iter().all(|p| p[0] >= 10.0));
        assert!(lire(b"pas un glb").is_err());
    }
}
