//! Les documents SVG, sans interface : lire et réécrire le XML (un arbre de nœuds
//! qu'on peut retoucher), comprendre les styles, les transformations et les formes
//! (rect, cercle, ellipse, ligne, polyligne, polygone, chemins complets avec courbes
//! et arcs, texte, `use`), et les réduire en `Forme`s : des contours en lignes
//! brisées, dans les coordonnées du document, avec leur remplissage et leur trait.
//!
//! Le remplissage se dessine par balayage (`balayer`) : des bandes horizontales d'un
//! pixel, selon la règle `nonzero` ou `evenodd` ; les trous et les formes croisées
//! sont donc justes. Les dégradés prennent la couleur moyenne de leurs arrêts ; les
//! feuilles de style `<style>`, les masques, les découpes et les images sont ignorés.

use crate::draw::Color;

pub type P = (f32, f32);

// --- L'arbre XML ---------------------------------------------------------------

/// Un nœud : un élément (`nom`), un texte (`nom` vide) ou un commentaire
/// (`#comment`). Les nœuds ne bougent pas dans `Document::noeuds` : un nœud retiré
/// est seulement détaché de son parent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Noeud {
    pub nom: String,
    pub attrs: Vec<(String, String)>,
    pub enfants: Vec<usize>,
    pub parent: Option<usize>,
    pub texte: String,
}

impl Noeud {
    pub fn attr(&self, nom: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.0 == nom).map(|a| a.1.as_str())
    }

    pub fn element(&self) -> bool {
        !self.nom.is_empty() && !self.nom.starts_with('#')
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub noeuds: Vec<Noeud>,
    pub racine: usize,
    /// Ce qui précède la racine (`<?xml …?>`, `<!DOCTYPE …>`), tel quel.
    pub prologue: String,
}

fn entites(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut reste = s;
    while let Some(i) = reste.find('&') {
        out.push_str(&reste[..i]);
        reste = &reste[i..];
        let Some(fin) = reste.find(';').filter(|f| *f < 12) else {
            out.push('&');
            reste = &reste[1..];
            continue;
        };
        let nom = &reste[1..fin];
        let c = match nom {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ if nom.starts_with("#x") || nom.starts_with("#X") => u32::from_str_radix(&nom[2..], 16).ok().and_then(char::from_u32),
            _ if nom.starts_with('#') => nom[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                reste = &reste[fin + 1..];
            }
            None => {
                out.push('&');
                reste = &reste[1..];
            }
        }
    }
    out.push_str(reste);
    out
}

fn echapper(s: &str, guillemet: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if guillemet => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

impl Document {
    /// Un document vide de `largeur` sur `hauteur`.
    pub fn nouveau(largeur: f32, hauteur: f32) -> Document {
        let racine = Noeud {
            nom: "svg".into(),
            attrs: vec![
                ("xmlns".into(), "http://www.w3.org/2000/svg".into()),
                ("width".into(), nombre(largeur)),
                ("height".into(), nombre(hauteur)),
                ("viewBox".into(), format!("0 0 {} {}", nombre(largeur), nombre(hauteur))),
            ],
            ..Noeud::default()
        };
        Document { noeuds: vec![racine], racine: 0, prologue: String::new() }
    }

    /// Lit un document. La racine doit être un élément `svg`.
    pub fn lire(texte: &str) -> Result<Document, String> {
        let s = texte;
        let b = s.as_bytes();
        let mut doc = Document::default();
        let mut pile: Vec<usize> = Vec::new();
        let mut racine = None;
        let mut i = 0;
        let ligne = |i: usize| 1 + s[..i].bytes().filter(|c| *c == b'\n').count();
        let ajouter = |doc: &mut Document, pile: &[usize], n: Noeud| -> usize {
            let k = doc.noeuds.len();
            let parent = pile.last().copied();
            doc.noeuds.push(Noeud { parent, ..n });
            if let Some(p) = parent {
                doc.noeuds[p].enfants.push(k);
            }
            k
        };
        while i < b.len() {
            if b[i] != b'<' {
                let fin = s[i..].find('<').map_or(s.len(), |f| i + f);
                let t = &s[i..fin];
                if !pile.is_empty() && !t.trim().is_empty() {
                    ajouter(&mut doc, &pile, Noeud { texte: entites(t), ..Noeud::default() });
                }
                i = fin;
                continue;
            }
            let reste = &s[i..];
            if reste.starts_with("<!--") {
                let fin = reste.find("-->").ok_or_else(|| format!("Commentaire non fermé (ligne {}).", ligne(i)))?;
                if pile.is_empty() {
                    if racine.is_none() {
                        doc.prologue.push_str(&reste[..fin + 3]);
                        doc.prologue.push('\n');
                    }
                } else {
                    ajouter(&mut doc, &pile, Noeud { nom: "#comment".into(), texte: reste[4..fin].to_string(), ..Noeud::default() });
                }
                i += fin + 3;
            } else if reste.starts_with("<![CDATA[") {
                let fin = reste.find("]]>").ok_or_else(|| format!("CDATA non fermé (ligne {}).", ligne(i)))?;
                if !pile.is_empty() {
                    ajouter(&mut doc, &pile, Noeud { texte: reste[9..fin].to_string(), ..Noeud::default() });
                }
                i += fin + 3;
            } else if reste.starts_with("<?") || reste.starts_with("<!") {
                // Déclaration, DOCTYPE (avec un éventuel sous-ensemble entre crochets)
                let mut fin = reste.find('>').ok_or_else(|| format!("Balise non fermée (ligne {}).", ligne(i)))?;
                if let Some(c) = reste.find('[').filter(|c| *c < fin) {
                    fin = reste[c..].find("]>").map_or(fin, |f| c + f + 1);
                }
                if pile.is_empty() && racine.is_none() {
                    doc.prologue.push_str(&reste[..fin + 1]);
                    doc.prologue.push('\n');
                }
                i += fin + 1;
            } else if reste.starts_with("</") {
                let fin = reste.find('>').ok_or_else(|| format!("Balise non fermée (ligne {}).", ligne(i)))?;
                let nom = reste[2..fin].trim();
                let Some(pos) = pile.iter().rposition(|k| doc.noeuds[*k].nom == nom) else {
                    return Err(format!("</{nom}> sans ouverture (ligne {}).", ligne(i)));
                };
                pile.truncate(pos);
                i += fin + 1;
            } else {
                // Une balise ouvrante : son nom, ses attributs
                let mut j = i + 1;
                while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
                    j += 1;
                }
                let nom = s[i + 1..j].to_string();
                if nom.is_empty() {
                    return Err(format!("Balise sans nom (ligne {}).", ligne(i)));
                }
                let mut attrs = Vec::new();
                let ferme;
                loop {
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j >= b.len() {
                        return Err(format!("<{nom}> non fermée (ligne {}).", ligne(i)));
                    }
                    if b[j] == b'>' {
                        ferme = false;
                        j += 1;
                        break;
                    }
                    if b[j] == b'/' && b.get(j + 1) == Some(&b'>') {
                        ferme = true;
                        j += 2;
                        break;
                    }
                    let a0 = j;
                    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'=' && b[j] != b'>' && b[j] != b'/' {
                        j += 1;
                    }
                    let an = s[a0..j].to_string();
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if b.get(j) != Some(&b'=') {
                        // Attribut sans valeur : toléré
                        if an.is_empty() {
                            j += 1;
                        } else {
                            attrs.push((an, String::new()));
                        }
                        continue;
                    }
                    j += 1;
                    while j < b.len() && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    let q = *b.get(j).ok_or_else(|| format!("Attribut {an} sans valeur (ligne {}).", ligne(i)))?;
                    if q != b'"' && q != b'\'' {
                        return Err(format!("Valeur de {an} sans guillemets (ligne {}).", ligne(j)));
                    }
                    let fin = s[j + 1..].find(q as char).ok_or_else(|| format!("Valeur de {an} non fermée (ligne {}).", ligne(j)))?;
                    attrs.push((an, entites(&s[j + 1..j + 1 + fin])));
                    j += fin + 2;
                }
                if pile.is_empty() && racine.is_some() {
                    return Err(format!("Contenu après la racine (ligne {}).", ligne(i)));
                }
                let k = ajouter(&mut doc, &pile, Noeud { nom, attrs, ..Noeud::default() });
                if racine.is_none() {
                    racine = Some(k);
                }
                if !ferme {
                    pile.push(k);
                }
                i = j;
            }
        }
        let racine = racine.ok_or("Aucun élément : ce n'est pas un SVG.")?;
        let nom = &doc.noeuds[racine].nom;
        if nom != "svg" && !nom.ends_with(":svg") {
            return Err(format!("La racine est <{nom}>, pas <svg>."));
        }
        if let Some(k) = pile.first() {
            return Err(format!("<{}> n'est pas fermée.", doc.noeuds[*k].nom));
        }
        doc.racine = racine;
        Ok(doc)
    }

    /// Le texte du document, indenté de deux espaces.
    pub fn ecrire(&self) -> String {
        let mut out = String::new();
        let p = self.prologue.trim();
        if !p.is_empty() {
            out.push_str(p);
            out.push('\n');
        }
        self.ecrire_noeud(self.racine, 0, &mut out);
        out
    }

    fn ecrire_noeud(&self, k: usize, niveau: usize, out: &mut String) {
        let n = &self.noeuds[k];
        let marge = "  ".repeat(niveau);
        if n.nom.is_empty() {
            out.push_str(&marge);
            out.push_str(&echapper(n.texte.trim(), false));
            out.push('\n');
            return;
        }
        if n.nom == "#comment" {
            out.push_str(&format!("{marge}<!--{}-->\n", n.texte));
            return;
        }
        out.push_str(&marge);
        out.push('<');
        out.push_str(&n.nom);
        for (a, v) in &n.attrs {
            out.push_str(&format!(" {a}=\"{}\"", echapper(v, true)));
        }
        if n.enfants.is_empty() {
            out.push_str("/>\n");
        } else if n.enfants.iter().any(|e| self.noeuds[*e].nom.is_empty()) {
            // Du texte mêlé : tout sur la même ligne, espaces compris
            out.push('>');
            for e in &n.enfants {
                self.ecrire_en_ligne(*e, out);
            }
            out.push_str(&format!("</{}>\n", n.nom));
        } else {
            out.push_str(">\n");
            for e in &n.enfants {
                self.ecrire_noeud(*e, niveau + 1, out);
            }
            out.push_str(&format!("{marge}</{}>\n", n.nom));
        }
    }

    fn ecrire_en_ligne(&self, k: usize, out: &mut String) {
        let n = &self.noeuds[k];
        if n.nom.is_empty() {
            out.push_str(&echapper(&n.texte, false));
        } else if n.nom == "#comment" {
            out.push_str(&format!("<!--{}-->", n.texte));
        } else {
            out.push('<');
            out.push_str(&n.nom);
            for (a, v) in &n.attrs {
                out.push_str(&format!(" {a}=\"{}\"", echapper(v, true)));
            }
            if n.enfants.is_empty() {
                out.push_str("/>");
            } else {
                out.push('>');
                for e in &n.enfants {
                    self.ecrire_en_ligne(*e, out);
                }
                out.push_str(&format!("</{}>", n.nom));
            }
        }
    }

    /// L'élément qui porte cet `id`.
    pub fn par_id(&self, id: &str) -> Option<usize> {
        let mut pile = vec![self.racine];
        while let Some(k) = pile.pop() {
            if self.noeuds[k].attr("id") == Some(id) {
                return Some(k);
            }
            pile.extend(self.noeuds[k].enfants.iter().rev());
        }
        None
    }

    /// Vrai si `n` est `ancetre` ou l'un de ses descendants.
    pub fn sous(&self, mut n: usize, ancetre: usize) -> bool {
        loop {
            if n == ancetre {
                return true;
            }
            match self.noeuds[n].parent {
                Some(p) => n = p,
                None => return false,
            }
        }
    }

    /// Ce qu'on sélectionne en touchant `n` : son ancêtre enfant direct de la racine.
    pub fn selectionnable(&self, mut n: usize) -> usize {
        while let Some(p) = self.noeuds[n].parent {
            if p == self.racine {
                return n;
            }
            n = p;
        }
        n
    }

    /// Le rectangle de la vue : `viewBox`, sinon `width` et `height`.
    pub fn cadre(&self) -> Option<[f32; 4]> {
        let r = &self.noeuds[self.racine];
        if let Some(v) = r.attr("viewBox") {
            let n = nombres(v);
            if n.len() == 4 && n[2] > 0.0 && n[3] > 0.0 {
                return Some([n[0], n[1], n[2], n[3]]);
            }
        }
        let w = r.attr("width").and_then(|v| longueur(v, 0.0))?;
        let h = r.attr("height").and_then(|v| longueur(v, 0.0))?;
        (w > 0.0 && h > 0.0).then_some([0.0, 0.0, w, h])
    }

    // --- Retouches ----------------------------------------------------------------

    /// Donne une valeur à une propriété de présentation (`fill`, `stroke`…) : en
    /// attribut, après l'avoir retirée du `style` qui la masquerait. `None` la retire.
    pub fn definir(&mut self, k: usize, nom: &str, valeur: Option<&str>) {
        let n = &mut self.noeuds[k];
        if let Some(i) = n.attrs.iter().position(|a| a.0 == "style") {
            let reste: Vec<String> = n.attrs[i].1.split(';').filter(|d| !d.trim().is_empty() && d.split(':').next().map(str::trim) != Some(nom)).map(|d| d.trim().to_string()).collect();
            if reste.is_empty() {
                n.attrs.remove(i);
            } else {
                n.attrs[i].1 = reste.join(";");
            }
        }
        match (n.attrs.iter().position(|a| a.0 == nom), valeur) {
            (Some(i), Some(v)) => n.attrs[i].1 = v.to_string(),
            (None, Some(v)) => n.attrs.push((nom.to_string(), v.to_string())),
            (Some(i), None) => {
                n.attrs.remove(i);
            }
            (None, None) => {}
        }
    }

    /// Ajoute un élément à la fin de `parent`.
    pub fn ajouter(&mut self, parent: usize, nom: &str, attrs: Vec<(String, String)>) -> usize {
        let k = self.noeuds.len();
        self.noeuds.push(Noeud { nom: nom.into(), attrs, parent: Some(parent), ..Noeud::default() });
        self.noeuds[parent].enfants.push(k);
        k
    }

    /// Ajoute un texte dans `parent`.
    pub fn ajouter_texte(&mut self, parent: usize, texte: &str) {
        let k = self.noeuds.len();
        self.noeuds.push(Noeud { texte: texte.into(), parent: Some(parent), ..Noeud::default() });
        self.noeuds[parent].enfants.push(k);
    }

    /// Détache `k` de son parent : il disparaît du document.
    pub fn retirer(&mut self, k: usize) {
        if let Some(p) = self.noeuds[k].parent.take() {
            self.noeuds[p].enfants.retain(|e| *e != k);
        }
    }

    /// Copie `k` et tout son contenu, juste après lui. Les `id` de la copie sont
    /// retirés (ils doivent rester uniques).
    pub fn dupliquer(&mut self, k: usize) -> Option<usize> {
        let p = self.noeuds[k].parent?;
        let c = self.copier(k, p);
        let i = self.noeuds[p].enfants.iter().position(|e| *e == k).map_or(0, |i| i + 1);
        self.noeuds[p].enfants.insert(i, c);
        Some(c)
    }

    fn copier(&mut self, k: usize, parent: usize) -> usize {
        let mut n = self.noeuds[k].clone();
        n.attrs.retain(|a| a.0 != "id");
        n.parent = Some(parent);
        let enfants = std::mem::take(&mut n.enfants);
        let c = self.noeuds.len();
        self.noeuds.push(n);
        let copies: Vec<usize> = enfants.into_iter().map(|e| self.copier(e, c)).collect();
        self.noeuds[c].enfants = copies;
        c
    }

    /// Monte (`haut`) ou descend `k` d'un plan parmi ses voisins éléments. Vrai si
    /// c'est fait.
    pub fn changer_plan(&mut self, k: usize, haut: bool) -> bool {
        let Some(p) = self.noeuds[k].parent else { return false };
        let enfants = self.noeuds[p].enfants.clone();
        let Some(i) = enfants.iter().position(|e| *e == k) else { return false };
        // Le voisin élément le plus proche dans ce sens
        let voisin = if haut { enfants[i + 1..].iter().position(|e| self.noeuds[*e].element()).map(|j| i + 1 + j) } else { enfants[..i].iter().rposition(|e| self.noeuds[*e].element()) };
        let Some(j) = voisin else { return false };
        let v = &mut self.noeuds[p].enfants;
        let x = v.remove(i);
        v.insert(j, x);
        true
    }

    /// La transformation qui mène des coordonnées de `k` (sans la sienne) à celles du
    /// document : le produit des `transform` de ses ancêtres sous la racine.
    pub fn ctm_parent(&self, k: usize) -> Matrice {
        let mut chaine = Vec::new();
        let mut n = self.noeuds[k].parent;
        while let Some(p) = n {
            if p == self.racine {
                break;
            }
            chaine.push(p);
            n = self.noeuds[p].parent;
        }
        let mut m = Matrice::IDENTITE;
        for p in chaine.into_iter().rev() {
            m = m.fois(&transformation(self.noeuds[p].attr("transform").unwrap_or("")));
        }
        m
    }

    /// Applique `m`, une transformation dans les coordonnées du document, à `k`
    /// dont la transformation propre était `avant` : `transform` devient
    /// P⁻¹·m·P·avant, P étant celle de ses ancêtres.
    pub fn transformer(&mut self, k: usize, m: &Matrice, avant: &str) {
        let p = self.ctm_parent(k);
        let t = p.inverse().fois(m).fois(&p).fois(&transformation(avant));
        let v = if t.identite() { None } else { Some(t.texte()) };
        let n = &mut self.noeuds[k];
        match (n.attrs.iter().position(|a| a.0 == "transform"), v) {
            (Some(i), Some(v)) => n.attrs[i].1 = v,
            (None, Some(v)) => n.attrs.push(("transform".into(), v)),
            (Some(i), None) => {
                n.attrs.remove(i);
            }
            (None, None) => {}
        }
    }
}

// --- Nombres, longueurs, couleurs ---------------------------------------------

/// Un nombre écrit court : « 12 », « 3.5 », « 0.333 ».
pub fn nombre(v: f32) -> String {
    let v = if v.abs() < 5e-4 { 0.0 } else { v };
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// Lit un nombre au début de `b` : (valeur, octets lus).
fn lire_nombre(b: &[u8]) -> Option<(f32, usize)> {
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let mut chiffres = false;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        chiffres = true;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            chiffres = true;
        }
    }
    if !chiffres {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    std::str::from_utf8(&b[..i]).ok()?.parse().ok().map(|v| (v, i))
}

/// Tous les nombres d'une liste (séparés par des espaces ou des virgules).
pub fn nombres(s: &str) -> Vec<f32> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if let Some((v, n)) = lire_nombre(&b[i..]) {
            out.push(v);
            i += n;
        } else {
            i += 1;
        }
    }
    out
}

/// Une longueur en unités du document : `px`, `mm`, `cm`, `in`, `pt`, `pc`, `em` ;
/// un pourcentage de `reference`.
pub fn longueur(s: &str, reference: f32) -> Option<f32> {
    let s = s.trim();
    let (v, n) = lire_nombre(s.as_bytes())?;
    let k = match s[n..].trim() {
        "" | "px" => 1.0,
        "mm" => 96.0 / 25.4,
        "cm" => 96.0 / 2.54,
        "in" => 96.0,
        "pt" => 4.0 / 3.0,
        "pc" => 16.0,
        "em" => 16.0,
        "ex" => 8.0,
        "%" => reference / 100.0,
        _ => 1.0,
    };
    Some(v * k)
}

/// Une couleur CSS : `#rgb`, `#rrggbb`, `rgb()`, `rgba()`, ou un nom courant.
pub fn couleur(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let v = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
        return match h.len() {
            3 => Some([v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17, 255]),
            4 => Some([v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17, v(3, 1)? * 17]),
            6 => Some([v(0, 2)?, v(2, 2)?, v(4, 2)?, 255]),
            8 => Some([v(0, 2)?, v(2, 2)?, v(4, 2)?, v(6, 2)?]),
            _ => None,
        };
    }
    let l = s.to_ascii_lowercase();
    if l.starts_with("rgb") {
        let dedans = l.split_once('(')?.1.trim_end_matches(')');
        let parts: Vec<&str> = dedans.split([',', ' ', '/']).filter(|p| !p.trim().is_empty()).collect();
        let canal = |p: &str| -> Option<u8> {
            let p = p.trim();
            Some(match p.strip_suffix('%') {
                Some(x) => (x.parse::<f32>().ok()? * 2.55).round().clamp(0.0, 255.0) as u8,
                None => p.parse::<f32>().ok()?.round().clamp(0.0, 255.0) as u8,
            })
        };
        let a = match parts.get(3) {
            Some(p) => match p.trim().strip_suffix('%') {
                Some(x) => (x.parse::<f32>().ok()? * 2.55).round() as u8,
                None => (p.trim().parse::<f32>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8,
            },
            None => 255,
        };
        return Some([canal(parts.first()?)?, canal(parts.get(1)?)?, canal(parts.get(2)?)?, a]);
    }
    let c: [u8; 3] = match l.as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "lime" => [0, 255, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "cyan" | "aqua" => [0, 255, 255],
        "magenta" | "fuchsia" => [255, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "silver" => [192, 192, 192],
        "lightgray" | "lightgrey" => [211, 211, 211],
        "darkgray" | "darkgrey" => [169, 169, 169],
        "maroon" => [128, 0, 0],
        "olive" => [128, 128, 0],
        "navy" => [0, 0, 128],
        "purple" => [128, 0, 128],
        "teal" => [0, 128, 128],
        "orange" => [255, 165, 0],
        "pink" => [255, 192, 203],
        "brown" => [165, 42, 42],
        "gold" => [255, 215, 0],
        "indigo" => [75, 0, 130],
        "violet" => [238, 130, 238],
        "crimson" => [220, 20, 60],
        "coral" => [255, 127, 80],
        "salmon" => [250, 128, 114],
        "tomato" => [255, 99, 71],
        "khaki" => [240, 230, 140],
        "beige" => [245, 245, 220],
        "ivory" => [255, 255, 240],
        "skyblue" => [135, 206, 235],
        "steelblue" => [70, 130, 180],
        "royalblue" => [65, 105, 225],
        "darkblue" => [0, 0, 139],
        "darkred" => [139, 0, 0],
        "darkgreen" => [0, 100, 0],
        "forestgreen" => [34, 139, 34],
        "seagreen" => [46, 139, 87],
        "turquoise" => [64, 224, 208],
        "tan" => [210, 180, 140],
        "chocolate" => [210, 105, 30],
        "sienna" => [160, 82, 45],
        "orchid" => [218, 112, 214],
        "plum" => [221, 160, 221],
        "lavender" => [230, 230, 250],
        "whitesmoke" => [245, 245, 245],
        "gainsboro" => [220, 220, 220],
        "slategray" | "slategrey" => [112, 128, 144],
        "dimgray" | "dimgrey" => [105, 105, 105],
        _ => return None,
    };
    Some([c[0], c[1], c[2], 255])
}

/// Une couleur en `#rrggbb` (l'opacité s'écrit à part).
pub fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

// --- Transformations -----------------------------------------------------------

/// Une transformation affine : (x, y) ↦ (a·x + c·y + e, b·x + d·y + f).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrice(pub [f32; 6]);

impl Matrice {
    pub const IDENTITE: Matrice = Matrice([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translation(x: f32, y: f32) -> Matrice {
        Matrice([1.0, 0.0, 0.0, 1.0, x, y])
    }

    pub fn echelle(x: f32, y: f32) -> Matrice {
        Matrice([x, 0.0, 0.0, y, 0.0, 0.0])
    }

    /// Une rotation de `a` radians autour de (cx, cy).
    pub fn rotation(a: f32, cx: f32, cy: f32) -> Matrice {
        let (s, c) = a.sin_cos();
        Matrice::translation(cx, cy).fois(&Matrice([c, s, -s, c, 0.0, 0.0])).fois(&Matrice::translation(-cx, -cy))
    }

    /// self ∘ m : `m` d'abord, puis `self`.
    pub fn fois(&self, m: &Matrice) -> Matrice {
        let [a, b, c, d, e, f] = self.0;
        let [p, q, r, s, t, u] = m.0;
        Matrice([a * p + c * q, b * p + d * q, a * r + c * s, b * r + d * s, a * t + c * u + e, b * t + d * u + f])
    }

    pub fn appliquer(&self, p: P) -> P {
        let [a, b, c, d, e, f] = self.0;
        (a * p.0 + c * p.1 + e, b * p.0 + d * p.1 + f)
    }

    pub fn inverse(&self) -> Matrice {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return Matrice::IDENTITE;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Matrice([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)])
    }

    /// Le facteur d'agrandissement moyen (pour les épaisseurs).
    pub fn facteur(&self) -> f32 {
        let [a, b, c, d, ..] = self.0;
        (a * d - b * c).abs().sqrt()
    }

    pub fn identite(&self) -> bool {
        self.0.iter().zip(Matrice::IDENTITE.0).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    pub fn texte(&self) -> String {
        let [a, b, c, d, e, f] = self.0;
        if (a - 1.0).abs() < 1e-5 && b.abs() < 1e-5 && c.abs() < 1e-5 && (d - 1.0).abs() < 1e-5 {
            return format!("translate({} {})", nombre(e), nombre(f));
        }
        let n = |v: f32| {
            let s = format!("{:.5}", v);
            let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
            if s == "-0" { "0".to_string() } else { s }
        };
        format!("matrix({} {} {} {} {} {})", n(a), n(b), n(c), n(d), nombre(e), nombre(f))
    }
}

/// Lit un attribut `transform` : matrix, translate, scale, rotate, skewX, skewY.
pub fn transformation(s: &str) -> Matrice {
    let mut m = Matrice::IDENTITE;
    let mut reste = s;
    while let Some(o) = reste.find('(') {
        let nom = reste[..o].trim().trim_start_matches(',').trim();
        let Some(f) = reste[o..].find(')') else { break };
        let v = nombres(&reste[o + 1..o + f]);
        let t = match (nom, v.len()) {
            ("matrix", 6) => Matrice([v[0], v[1], v[2], v[3], v[4], v[5]]),
            ("translate", 1) => Matrice::translation(v[0], 0.0),
            ("translate", 2) => Matrice::translation(v[0], v[1]),
            ("scale", 1) => Matrice::echelle(v[0], v[0]),
            ("scale", 2) => Matrice::echelle(v[0], v[1]),
            ("rotate", 1) => Matrice::rotation(v[0].to_radians(), 0.0, 0.0),
            ("rotate", 3) => Matrice::rotation(v[0].to_radians(), v[1], v[2]),
            ("skewX", 1) => Matrice([1.0, 0.0, v[0].to_radians().tan(), 1.0, 0.0, 0.0]),
            ("skewY", 1) => Matrice([1.0, v[0].to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => Matrice::IDENTITE,
        };
        m = m.fois(&t);
        reste = &reste[o + f + 1..];
    }
    m
}

// --- Chemins -------------------------------------------------------------------

/// Des sous-chemins en lignes brisées, et s'ils sont fermés.
pub type Contours = Vec<(Vec<P>, bool)>;

/// Le nombre de segments pour une courbe de longueur `l` (à l'échelle `s`).
fn segments(l: f32, s: f32) -> usize {
    ((l * s / 3.0).ceil() as usize).clamp(2, 96)
}

/// Réduit un attribut `d` en lignes brisées. `s` : l'agrandissement prévu, pour la
/// finesse des courbes.
pub fn chemin(d: &str, s: f32) -> Contours {
    let b = d.as_bytes();
    let mut i = 0;
    let mut out: Contours = Vec::new();
    let mut cour: Vec<P> = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let mut depart = (0.0, 0.0);
    let mut cmd = b'M';
    // Le dernier point de contrôle (pour S et T) et la commande qui l'a posé
    let mut ctrl: Option<P> = None;
    let mut ctrl_q: Option<P> = None;
    let blanc = |i: &mut usize| {
        while *i < b.len() && (b[*i].is_ascii_whitespace() || b[*i] == b',') {
            *i += 1;
        }
    };
    let num = |i: &mut usize| -> Option<f32> {
        blanc(i);
        let (v, n) = lire_nombre(&b[*i..])?;
        *i += n;
        Some(v)
    };
    let drapeau = |i: &mut usize| -> Option<bool> {
        blanc(i);
        let c = *b.get(*i)?;
        if c == b'0' || c == b'1' {
            *i += 1;
            Some(c == b'1')
        } else {
            None
        }
    };
    let finir = |cour: &mut Vec<P>, out: &mut Contours, ferme: bool| {
        if cour.len() > 1 || (ferme && !cour.is_empty()) {
            out.push((std::mem::take(cour), ferme));
        } else {
            cour.clear();
        }
    };
    loop {
        blanc(&mut i);
        if i >= b.len() {
            break;
        }
        if b[i].is_ascii_alphabetic() {
            cmd = b[i];
            i += 1;
            if cmd == b'Z' || cmd == b'z' {
                finir(&mut cour, &mut out, true);
                x = depart.0;
                y = depart.1;
                ctrl = None;
                ctrl_q = None;
                continue;
            }
        } else if cmd == b'Z' || cmd == b'z' {
            // Des nombres après Z : on ignore
            i += 1;
            continue;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        let k = cmd.to_ascii_uppercase();
        let ok = (|| -> Option<()> {
            match k {
                b'M' => {
                    let (nx, ny) = (num(&mut i)? + ox, num(&mut i)? + oy);
                    finir(&mut cour, &mut out, false);
                    cour.push((nx, ny));
                    (x, y) = (nx, ny);
                    depart = (nx, ny);
                    // Les paires suivantes sont des L
                    cmd = if rel { b'l' } else { b'L' };
                    ctrl = None;
                    ctrl_q = None;
                }
                b'L' => {
                    let (nx, ny) = (num(&mut i)? + ox, num(&mut i)? + oy);
                    if cour.is_empty() {
                        cour.push((x, y));
                    }
                    cour.push((nx, ny));
                    (x, y) = (nx, ny);
                    ctrl = None;
                    ctrl_q = None;
                }
                b'H' => {
                    let nx = num(&mut i)? + ox;
                    if cour.is_empty() {
                        cour.push((x, y));
                    }
                    cour.push((nx, y));
                    x = nx;
                    ctrl = None;
                    ctrl_q = None;
                }
                b'V' => {
                    let ny = num(&mut i)? + if rel { y } else { 0.0 };
                    if cour.is_empty() {
                        cour.push((x, y));
                    }
                    cour.push((x, ny));
                    y = ny;
                    ctrl = None;
                    ctrl_q = None;
                }
                b'C' | b'S' => {
                    let c1 = if k == b'C' {
                        (num(&mut i)? + ox, num(&mut i)? + oy)
                    } else {
                        ctrl.map_or((x, y), |c| (2.0 * x - c.0, 2.0 * y - c.1))
                    };
                    let c2 = (num(&mut i)? + ox, num(&mut i)? + oy);
                    let p = (num(&mut i)? + ox, num(&mut i)? + oy);
                    if cour.is_empty() {
                        cour.push((x, y));
                    }
                    let l = dist((x, y), c1) + dist(c1, c2) + dist(c2, p);
                    let n = segments(l, s);
                    for j in 1..=n {
                        let t = j as f32 / n as f32;
                        let u = 1.0 - t;
                        let (a, bb, c, dd) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                        cour.push((a * x + bb * c1.0 + c * c2.0 + dd * p.0, a * y + bb * c1.1 + c * c2.1 + dd * p.1));
                    }
                    ctrl = Some(c2);
                    ctrl_q = None;
                    (x, y) = p;
                }
                b'Q' | b'T' => {
                    let c1 = if k == b'Q' {
                        (num(&mut i)? + ox, num(&mut i)? + oy)
                    } else {
                        ctrl_q.map_or((x, y), |c| (2.0 * x - c.0, 2.0 * y - c.1))
                    };
                    let p = (num(&mut i)? + ox, num(&mut i)? + oy);
                    if cour.is_empty() {
                        cour.push((x, y));
                    }
                    let n = segments(dist((x, y), c1) + dist(c1, p), s);
                    for j in 1..=n {
                        let t = j as f32 / n as f32;
                        let u = 1.0 - t;
                        cour.push((u * u * x + 2.0 * u * t * c1.0 + t * t * p.0, u * u * y + 2.0 * u * t * c1.1 + t * t * p.1));
                    }
                    ctrl_q = Some(c1);
                    ctrl = None;
                    (x, y) = p;
                }
                b'A' => {
                    let (rx, ry, rot) = (num(&mut i)?, num(&mut i)?, num(&mut i)?);
                    let (grand, sens) = (drapeau(&mut i)?, drapeau(&mut i)?);
                    let p = (num(&mut i)? + ox, num(&mut i)? + oy);
                    if cour.is_empty() {
                        cour.push((x, y));
                    }
                    arc(&mut cour, (x, y), p, rx, ry, rot, grand, sens, s);
                    ctrl = None;
                    ctrl_q = None;
                    (x, y) = p;
                }
                _ => return None,
            }
            Some(())
        })();
        if ok.is_none() {
            // Un chemin mal formé s'arrête à l'erreur, comme dans les navigateurs
            break;
        }
    }
    finir(&mut cour, &mut out, false);
    out
}

fn dist(a: P, b: P) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Le centre d'un arc SVG de `a` à `b` : (centre, rx, ry agrandis s'il le faut,
/// rotation en radians, angle de départ, balayage). `None` pour un arc dégénéré (un
/// segment droit).
#[allow(clippy::type_complexity)]
fn arc_centre(a: P, b: P, rx: f32, ry: f32, rot: f32, grand: bool, sens: bool) -> Option<(P, f32, f32, f32, f32, f32)> {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-6 || ry < 1e-6 || dist(a, b) < 1e-6 {
        return None;
    }
    let phi = rot.to_radians();
    let (sp, cp) = phi.sin_cos();
    let (dx, dy) = ((a.0 - b.0) / 2.0, (a.1 - b.1) / 2.0);
    let (x1, y1) = (cp * dx + sp * dy, -sp * dx + cp * dy);
    // Des rayons trop petits s'agrandissent
    let l = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if l > 1.0 {
        rx *= l.sqrt();
        ry *= l.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = if den > 0.0 { (num / den).sqrt() } else { 0.0 };
    if grand == sens {
        k = -k;
    }
    let (cx1, cy1) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let (cx, cy) = (cp * cx1 - sp * cy1 + (a.0 + b.0) / 2.0, sp * cx1 + cp * cy1 + (a.1 + b.1) / 2.0);
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let d = (ux * vx + uy * vy) / ((ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt()).max(1e-12);
        let a = d.clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 { -a } else { a }
    };
    let t1 = angle(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dt = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let tau = std::f32::consts::TAU;
    if !sens && dt > 0.0 {
        dt -= tau;
    } else if sens && dt < 0.0 {
        dt += tau;
    }
    Some(((cx, cy), rx, ry, phi, t1, dt))
}

/// Un arc elliptique de `a` à `b` (paramètres SVG), ajouté à `cour` sans son départ.
#[allow(clippy::too_many_arguments)]
fn arc(cour: &mut Vec<P>, a: P, b: P, rx: f32, ry: f32, rot: f32, grand: bool, sens: bool, s: f32) {
    let Some(((cx, cy), rx, ry, phi, t1, dt)) = arc_centre(a, b, rx, ry, rot, grand, sens) else {
        cour.push(b);
        return;
    };
    let (sp, cp) = phi.sin_cos();
    let n = segments(dt.abs() * rx.max(ry), s).max(4);
    for j in 1..=n {
        let t = t1 + dt * j as f32 / n as f32;
        let (st, ct) = t.sin_cos();
        let (ex, ey) = (rx * ct, ry * st);
        cour.push((cp * ex - sp * ey + cx, sp * ex + cp * ey + cy));
    }
    // Le bout exact
    if let Some(d) = cour.last_mut() {
        *d = b;
    }
}

/// Un arc SVG en courbes de Bézier cubiques (au plus un quart de tour chacune) :
/// (contrôle 1, contrôle 2, fin). Un arc dégénéré donne une « courbe » droite.
#[allow(clippy::too_many_arguments)]
pub fn arc_cubiques(a: P, b: P, rx: f32, ry: f32, rot: f32, grand: bool, sens: bool) -> Vec<(P, P, P)> {
    let Some(((cx, cy), rx, ry, phi, t1, dt)) = arc_centre(a, b, rx, ry, rot, grand, sens) else {
        return vec![(a, b, b)];
    };
    let (sp, cp) = phi.sin_cos();
    let point = |t: f32| {
        let (st, ct) = t.sin_cos();
        let (ex, ey) = (rx * ct, ry * st);
        (cp * ex - sp * ey + cx, sp * ex + cp * ey + cy)
    };
    let derivee = |t: f32| {
        let (st, ct) = t.sin_cos();
        let (ex, ey) = (-rx * st, ry * ct);
        (cp * ex - sp * ey, sp * ex + cp * ey)
    };
    let n = (dt.abs() / std::f32::consts::FRAC_PI_2 - 1e-3).ceil().max(1.0) as usize;
    let pas = dt / n as f32;
    let alpha = 4.0 / 3.0 * (pas / 4.0).tan();
    (0..n)
        .map(|j| {
            let (ta, tb) = (t1 + pas * j as f32, t1 + pas * (j + 1) as f32);
            let (pa, pb) = (point(ta), if j + 1 == n { b } else { point(tb) });
            let (da, db) = (derivee(ta), derivee(tb));
            ((pa.0 + alpha * da.0, pa.1 + alpha * da.1), (pb.0 - alpha * db.0, pb.1 - alpha * db.1), pb)
        })
        .collect()
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, s: f32) -> Vec<P> {
    let n = segments(std::f32::consts::TAU * rx.max(ry), s).max(12);
    (0..n)
        .map(|j| {
            let t = std::f32::consts::TAU * j as f32 / n as f32;
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
}

// --- Les tracés éditables ----------------------------------------------------------

/// Un morceau d'un tracé, en coordonnées absolues : chaque morceau sauf `Fermer`
/// mène à un sommet. Un sous-chemin commence toujours par `Depart`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Morceau {
    Depart(P),
    Droite(P),
    /// Contrôle 1, contrôle 2, fin.
    Cubique(P, P, P),
    /// Contrôle, fin.
    Quadratique(P, P),
    Arc { rx: f32, ry: f32, rot: f32, grand: bool, sens: bool, fin: P },
    Fermer,
}

impl Morceau {
    /// Le sommet où il mène (aucun pour `Fermer`).
    pub fn fin(&self) -> Option<P> {
        match *self {
            Morceau::Depart(p) | Morceau::Droite(p) | Morceau::Cubique(_, _, p) | Morceau::Quadratique(_, p) | Morceau::Arc { fin: p, .. } => Some(p),
            Morceau::Fermer => None,
        }
    }

    fn fin_mut(&mut self) -> Option<&mut P> {
        match self {
            Morceau::Depart(p) | Morceau::Droite(p) | Morceau::Cubique(_, _, p) | Morceau::Quadratique(_, p) | Morceau::Arc { fin: p, .. } => Some(p),
            Morceau::Fermer => None,
        }
    }
}

/// Ce qu'on peut saisir sur un tracé : le sommet d'un morceau, ou l'un de ses points
/// de contrôle (0 ou 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repere {
    Sommet(usize),
    Controle(usize, u8),
}

/// Lit un attribut `d` en morceaux absolus (H et V deviennent des droites, S et T
/// des courbes pleines). Un chemin mal formé s'arrête à l'erreur.
pub fn lire_trace(d: &str) -> Vec<Morceau> {
    let b = d.as_bytes();
    let mut i = 0;
    let mut t: Vec<Morceau> = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let mut depart = (0.0, 0.0);
    let mut cmd = b'M';
    let blanc = |i: &mut usize| {
        while *i < b.len() && (b[*i].is_ascii_whitespace() || b[*i] == b',') {
            *i += 1;
        }
    };
    let num = |i: &mut usize| -> Option<f32> {
        blanc(i);
        let (v, n) = lire_nombre(&b[*i..])?;
        *i += n;
        Some(v)
    };
    let drapeau = |i: &mut usize| -> Option<bool> {
        blanc(i);
        let c = *b.get(*i)?;
        if c == b'0' || c == b'1' {
            *i += 1;
            Some(c == b'1')
        } else {
            None
        }
    };
    loop {
        blanc(&mut i);
        if i >= b.len() {
            break;
        }
        if b[i].is_ascii_alphabetic() {
            cmd = b[i];
            i += 1;
            if cmd == b'Z' || cmd == b'z' {
                if t.last().is_some_and(|m| *m != Morceau::Fermer) {
                    t.push(Morceau::Fermer);
                }
                (x, y) = depart;
                continue;
            }
        } else if cmd == b'Z' || cmd == b'z' {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
        let k = cmd.to_ascii_uppercase();
        // Un segment sans départ (au début, ou après Z) part du point courant
        if k != b'M' && (t.is_empty() || t.last() == Some(&Morceau::Fermer)) {
            t.push(Morceau::Depart((x, y)));
        }
        // Le contrôle précédent, pour S et T
        let reflet = |t: &[Morceau], cubique: bool| match t.last() {
            Some(Morceau::Cubique(_, c, _)) if cubique => (2.0 * x - c.0, 2.0 * y - c.1),
            Some(Morceau::Quadratique(c, _)) if !cubique => (2.0 * x - c.0, 2.0 * y - c.1),
            _ => (x, y),
        };
        let ok = (|| -> Option<Morceau> {
            Some(match k {
                b'M' => {
                    let p = (num(&mut i)? + ox, num(&mut i)? + oy);
                    depart = p;
                    cmd = if rel { b'l' } else { b'L' };
                    Morceau::Depart(p)
                }
                b'L' => Morceau::Droite((num(&mut i)? + ox, num(&mut i)? + oy)),
                b'H' => Morceau::Droite((num(&mut i)? + ox, y)),
                b'V' => Morceau::Droite((x, num(&mut i)? + if rel { y } else { 0.0 })),
                b'C' => Morceau::Cubique((num(&mut i)? + ox, num(&mut i)? + oy), (num(&mut i)? + ox, num(&mut i)? + oy), (num(&mut i)? + ox, num(&mut i)? + oy)),
                b'S' => {
                    let c1 = reflet(&t, true);
                    Morceau::Cubique(c1, (num(&mut i)? + ox, num(&mut i)? + oy), (num(&mut i)? + ox, num(&mut i)? + oy))
                }
                b'Q' => Morceau::Quadratique((num(&mut i)? + ox, num(&mut i)? + oy), (num(&mut i)? + ox, num(&mut i)? + oy)),
                b'T' => {
                    let c = reflet(&t, false);
                    Morceau::Quadratique(c, (num(&mut i)? + ox, num(&mut i)? + oy))
                }
                b'A' => {
                    let (rx, ry, rot) = (num(&mut i)?, num(&mut i)?, num(&mut i)?);
                    let (grand, sens) = (drapeau(&mut i)?, drapeau(&mut i)?);
                    Morceau::Arc { rx, ry, rot, grand, sens, fin: (num(&mut i)? + ox, num(&mut i)? + oy) }
                }
                _ => return None,
            })
        })();
        let Some(m) = ok else { break };
        // Un départ qui en suit un autre le remplace
        if matches!(m, Morceau::Depart(_)) && matches!(t.last(), Some(Morceau::Depart(_))) {
            t.pop();
        }
        (x, y) = m.fin().unwrap();
        t.push(m);
    }
    // Un départ seul à la fin ne trace rien
    if matches!(t.last(), Some(Morceau::Depart(_))) {
        t.pop();
    }
    t
}

/// Écrit un tracé en attribut `d`.
pub fn ecrire_trace(t: &[Morceau]) -> String {
    let p = |p: P| format!("{} {}", nombre(p.0), nombre(p.1));
    let morceaux: Vec<String> = t
        .iter()
        .map(|m| match *m {
            Morceau::Depart(a) => format!("M{}", p(a)),
            Morceau::Droite(a) => format!("L{}", p(a)),
            Morceau::Cubique(a, b, c) => format!("C{} {} {}", p(a), p(b), p(c)),
            Morceau::Quadratique(a, b) => format!("Q{} {}", p(a), p(b)),
            Morceau::Arc { rx, ry, rot, grand, sens, fin } => format!("A{} {} {} {} {} {}", nombre(rx), nombre(ry), nombre(rot), grand as u8, sens as u8, p(fin)),
            Morceau::Fermer => "Z".into(),
        })
        .collect();
    morceaux.join(" ")
}

/// Le tracé d'un élément qu'on peut éditer : path, polygon, polyline, line, rect,
/// circle, ellipse. Les formes simples deviennent des droites, des arcs ou des
/// courbes (quatre quarts pour une ellipse).
pub fn trace_de(n: &Noeud) -> Option<Vec<Morceau>> {
    let nb = |k: &str| n.attr(k).and_then(|v| longueur(v, 0.0)).unwrap_or(0.0);
    let t = match n.nom.as_str() {
        "path" => lire_trace(n.attr("d")?),
        "polygon" | "polyline" => {
            let v = nombres(n.attr("points")?);
            let mut t: Vec<Morceau> = v.chunks_exact(2).enumerate().map(|(i, c)| if i == 0 { Morceau::Depart((c[0], c[1])) } else { Morceau::Droite((c[0], c[1])) }).collect();
            if t.len() < 2 {
                return None;
            }
            if n.nom == "polygon" {
                t.push(Morceau::Fermer);
            }
            t
        }
        "line" => vec![Morceau::Depart((nb("x1"), nb("y1"))), Morceau::Droite((nb("x2"), nb("y2")))],
        "rect" => {
            let (x, y, w, h) = (nb("x"), nb("y"), nb("width"), nb("height"));
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            let (rx, ry) = match (n.attr("rx").and_then(|v| longueur(v, 0.0)), n.attr("ry").and_then(|v| longueur(v, 0.0))) {
                (Some(a), Some(b)) => (a, b),
                (Some(a), None) | (None, Some(a)) => (a, a),
                (None, None) => (0.0, 0.0),
            };
            let (rx, ry) = (rx.clamp(0.0, w / 2.0), ry.clamp(0.0, h / 2.0));
            if rx <= 0.0 || ry <= 0.0 {
                vec![Morceau::Depart((x, y)), Morceau::Droite((x + w, y)), Morceau::Droite((x + w, y + h)), Morceau::Droite((x, y + h)), Morceau::Fermer]
            } else {
                let a = |fin: P| Morceau::Arc { rx, ry, rot: 0.0, grand: false, sens: true, fin };
                vec![
                    Morceau::Depart((x + rx, y)),
                    Morceau::Droite((x + w - rx, y)),
                    a((x + w, y + ry)),
                    Morceau::Droite((x + w, y + h - ry)),
                    a((x + w - rx, y + h)),
                    Morceau::Droite((x + rx, y + h)),
                    a((x, y + h - ry)),
                    Morceau::Droite((x, y + ry)),
                    a((x + rx, y)),
                    Morceau::Fermer,
                ]
            }
        }
        "circle" | "ellipse" => {
            let (cx, cy) = (nb("cx"), nb("cy"));
            let (rx, ry) = if n.nom == "circle" { (nb("r"), nb("r")) } else { (nb("rx"), nb("ry")) };
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            let k = 0.552_284_8;
            vec![
                Morceau::Depart((cx + rx, cy)),
                Morceau::Cubique((cx + rx, cy + k * ry), (cx + k * rx, cy + ry), (cx, cy + ry)),
                Morceau::Cubique((cx - k * rx, cy + ry), (cx - rx, cy + k * ry), (cx - rx, cy)),
                Morceau::Cubique((cx - rx, cy - k * ry), (cx - k * rx, cy - ry), (cx, cy - ry)),
                Morceau::Cubique((cx + k * rx, cy - ry), (cx + rx, cy - k * ry), (cx + rx, cy)),
                Morceau::Fermer,
            ]
        }
        _ => return None,
    };
    (t.len() >= 2).then_some(t)
}

/// Les attributs de géométrie des formes simples, retirés quand une forme devient un
/// chemin.
const GEOMETRIE: [&str; 14] = ["x", "y", "width", "height", "rx", "ry", "cx", "cy", "r", "points", "x1", "y1", "x2", "y2"];

impl Document {
    /// Donne à `k` le tracé `t`. Un polygone, une polyligne ou une ligne le restent
    /// tant que le tracé s'y prête ; sinon l'élément devient un `path`.
    pub fn poser_trace(&mut self, k: usize, t: &[Morceau]) {
        let droites = t.iter().all(|m| matches!(m, Morceau::Depart(_) | Morceau::Droite(_) | Morceau::Fermer));
        let departs = t.iter().filter(|m| matches!(m, Morceau::Depart(_))).count();
        let fermes = t.iter().filter(|m| **m == Morceau::Fermer).count();
        let points = || t.iter().filter_map(Morceau::fin).map(|p| format!("{},{}", nombre(p.0), nombre(p.1))).collect::<Vec<_>>().join(" ");
        let nom = self.noeuds[k].nom.clone();
        match nom.as_str() {
            "polygon" if droites && departs == 1 && fermes == 1 && t.last() == Some(&Morceau::Fermer) => self.definir(k, "points", Some(&points())),
            "polyline" if droites && departs == 1 && fermes == 0 => self.definir(k, "points", Some(&points())),
            "line" if droites && t.len() == 2 && departs == 1 => {
                let (a, b) = (t[0].fin().unwrap(), t[1].fin().unwrap());
                for (n, v) in [("x1", a.0), ("y1", a.1), ("x2", b.0), ("y2", b.1)] {
                    self.definir(k, n, Some(&nombre(v)));
                }
            }
            _ => {
                if nom != "path" {
                    let n = &mut self.noeuds[k];
                    n.attrs.retain(|a| !GEOMETRIE.contains(&a.0.as_str()));
                    n.nom = "path".into();
                }
                self.definir(k, "d", Some(&ecrire_trace(t)));
            }
        }
    }

    /// La transformation qui mène des coordonnées propres de `k` à celles du document.
    pub fn ctm(&self, k: usize) -> Matrice {
        self.ctm_parent(k).fois(&transformation(self.noeuds[k].attr("transform").unwrap_or("")))
    }
}

fn milieu(a: P, b: P, u: f32) -> P {
    (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u)
}

/// Le sous-chemin qui contient le morceau `i` : [début, fin[.
pub fn sous_chemin(t: &[Morceau], i: usize) -> (usize, usize) {
    let debut = t[..=i.min(t.len() - 1)].iter().rposition(|m| matches!(m, Morceau::Depart(_))).unwrap_or(0);
    let fin = t[debut + 1..].iter().position(|m| matches!(m, Morceau::Depart(_))).map_or(t.len(), |j| debut + 1 + j);
    (debut, fin)
}

/// Le dernier sommet d'un sous-chemin fermé qui retombe sur son départ : il n'est
/// pas montré, et suit le départ.
pub fn jumeau(t: &[Morceau], debut: usize, fin: usize) -> Option<usize> {
    if fin < debut + 3 || t[fin - 1] != Morceau::Fermer {
        return None;
    }
    let j = fin - 2;
    (j > debut && dist(t[j].fin()?, t[debut].fin()?) < 1e-3).then_some(j)
}

/// Où commence le segment `i` : le sommet d'avant (pour `Fermer`, le dernier du
/// sous-chemin).
pub fn depart_de(t: &[Morceau], i: usize) -> P {
    t[..i].iter().rev().find_map(Morceau::fin).unwrap_or((0.0, 0.0))
}

/// Le point d'un repère.
pub fn point_de(t: &[Morceau], r: Repere) -> Option<P> {
    match (r, t.get(r_indice(r))?) {
        (Repere::Sommet(_), m) => m.fin(),
        (Repere::Controle(_, 0), Morceau::Cubique(a, _, _)) | (Repere::Controle(_, 1), Morceau::Cubique(_, a, _)) | (Repere::Controle(_, 0), Morceau::Quadratique(a, _)) => Some(*a),
        _ => None,
    }
}

fn r_indice(r: Repere) -> usize {
    match r {
        Repere::Sommet(i) | Repere::Controle(i, _) => i,
    }
}

/// Les repères à montrer, avec leur point et les sommets auxquels ils se rattachent
/// (pour les tiges des contrôles). Les jumeaux sont omis.
pub fn reperes(t: &[Morceau]) -> Vec<(Repere, P, Vec<P>)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let (debut, fin) = sous_chemin(t, i);
        let jum = jumeau(t, debut, fin);
        for j in debut..fin {
            let a = || depart_de(t, j);
            match t[j] {
                Morceau::Cubique(c1, c2, p) => {
                    v.push((Repere::Controle(j, 0), c1, vec![a()]));
                    v.push((Repere::Controle(j, 1), c2, vec![p]));
                }
                Morceau::Quadratique(c, p) => v.push((Repere::Controle(j, 0), c, vec![a(), p])),
                _ => {}
            }
            if let Some(p) = t[j].fin() {
                if Some(j) != jum {
                    v.push((Repere::Sommet(j), p, Vec::new()));
                }
            }
        }
        i = fin;
    }
    v
}

/// Le point du segment `i` (qui finit au sommet `i`, ou ferme le sous-chemin) au
/// paramètre `u`.
pub fn point_sur(t: &[Morceau], i: usize, u: f32) -> P {
    let a = depart_de(t, i);
    match t[i] {
        Morceau::Depart(p) => p,
        Morceau::Droite(p) => milieu(a, p, u),
        Morceau::Cubique(c1, c2, p) => {
            let v = 1.0 - u;
            let (k0, k1, k2, k3) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
            (k0 * a.0 + k1 * c1.0 + k2 * c2.0 + k3 * p.0, k0 * a.1 + k1 * c1.1 + k2 * c2.1 + k3 * p.1)
        }
        Morceau::Quadratique(c, p) => milieu(milieu(a, c, u), milieu(c, p, u), u),
        Morceau::Arc { rx, ry, rot, grand, sens, fin } => {
            let c = arc_cubiques(a, fin, rx, ry, rot, grand, sens);
            let x = (u * c.len() as f32).min(c.len() as f32 - 1e-4).max(0.0);
            let (j, w) = (x as usize, x.fract());
            let depart = if j == 0 { a } else { c[j - 1].2 };
            let t2 = [Morceau::Depart(depart), Morceau::Cubique(c[j].0, c[j].1, c[j].2)];
            point_sur(&t2, 1, w)
        }
        Morceau::Fermer => milieu(a, t[sous_chemin(t, i).0].fin().unwrap_or(a), u),
    }
}

/// Le segment le plus proche de `p` (en coordonnées du tracé, mesurées après `m`),
/// à moins de `tolerance` : (segment, paramètre).
pub fn segment_proche(t: &[Morceau], m: &Matrice, p: P, tolerance: f32) -> Option<(usize, f32)> {
    let mut mieux: Option<(f32, usize, f32)> = None;
    const N: usize = 32;
    for i in 0..t.len() {
        if matches!(t[i], Morceau::Depart(_)) {
            continue;
        }
        let pts: Vec<P> = (0..=N).map(|k| m.appliquer(point_sur(t, i, k as f32 / N as f32))).collect();
        for k in 0..N {
            let (a, b) = (pts[k], pts[k + 1]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l2 = dx * dx + dy * dy;
            let w = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let d = dist(p, (a.0 + dx * w, a.1 + dy * w));
            if d < tolerance && mieux.is_none_or(|x| d < x.0) {
                mieux = Some((d, i, (k as f32 + w) / N as f32));
            }
        }
    }
    mieux.map(|x| (x.1, x.2))
}

/// Déplace un repère en `p`. Un sommet emmène ses contrôles (et son jumeau).
pub fn deplacer(t: &mut [Morceau], r: Repere, p: P) {
    match r {
        Repere::Sommet(i) => {
            let Some(avant) = t[i].fin() else { return };
            let (dx, dy) = (p.0 - avant.0, p.1 - avant.1);
            let (debut, fin) = sous_chemin(t, i);
            let jum = jumeau(t, debut, fin);
            let mut sommets = vec![i];
            match jum {
                Some(j) if i == debut => sommets.push(j),
                Some(j) if i == j => sommets.push(debut),
                _ => {}
            }
            for s in sommets {
                if let Some(q) = t[s].fin_mut() {
                    *q = (q.0 + dx, q.1 + dy);
                }
                if let Morceau::Cubique(_, c2, _) = &mut t[s] {
                    *c2 = (c2.0 + dx, c2.1 + dy);
                }
                if let Some(Morceau::Cubique(c1, _, _)) = t.get_mut(s + 1) {
                    *c1 = (c1.0 + dx, c1.1 + dy);
                }
            }
        }
        Repere::Controle(i, k) => match (&mut t[i], k) {
            (Morceau::Cubique(c, _, _), 0) | (Morceau::Cubique(_, c, _), 1) | (Morceau::Quadratique(c, _), 0) => *c = p,
            _ => {}
        },
    }
}

/// Coupe le segment `i` au paramètre `u` : un sommet de plus, dont l'indice est
/// renvoyé. La forme ne change pas.
pub fn couper(t: &mut Vec<Morceau>, i: usize, u: f32) -> Option<usize> {
    let a = depart_de(t, i);
    let u = u.clamp(0.01, 0.99);
    match t[i] {
        Morceau::Depart(_) => return None,
        Morceau::Droite(p) => t.splice(i..=i, [Morceau::Droite(milieu(a, p, u)), Morceau::Droite(p)]).for_each(drop),
        Morceau::Cubique(c1, c2, p) => {
            let (ab, bc, cd) = (milieu(a, c1, u), milieu(c1, c2, u), milieu(c2, p, u));
            let (abc, bcd) = (milieu(ab, bc, u), milieu(bc, cd, u));
            let m = milieu(abc, bcd, u);
            t.splice(i..=i, [Morceau::Cubique(ab, abc, m), Morceau::Cubique(bcd, cd, p)]).for_each(drop);
        }
        Morceau::Quadratique(c, p) => {
            let (ac, cp) = (milieu(a, c, u), milieu(c, p, u));
            t.splice(i..=i, [Morceau::Quadratique(ac, milieu(ac, cp, u)), Morceau::Quadratique(cp, p)]).for_each(drop);
        }
        Morceau::Arc { rx, ry, rot, grand, sens, fin } => {
            // En courbes d'abord, puis la bonne est coupée
            let c = arc_cubiques(a, fin, rx, ry, rot, grand, sens);
            let x = (u * c.len() as f32).min(c.len() as f32 - 1e-3);
            let (j, w) = (x as usize, x.fract());
            t.splice(i..=i, c.iter().map(|q| Morceau::Cubique(q.0, q.1, q.2))).for_each(drop);
            if w < 0.02 && j > 0 {
                return Some(i + j - 1);
            }
            return couper(t, i + j, w);
        }
        Morceau::Fermer => {
            let d = t[sous_chemin(t, i).0].fin()?;
            t.insert(i, Morceau::Droite(milieu(a, d, u)));
        }
    }
    Some(i)
}

/// Retire le sommet `i`. Les deux courbes autour se fondent en une. Un sous-chemin
/// réduit à un point disparaît. Faux si le tracé deviendrait vide.
pub fn retirer_sommet(t: &mut Vec<Morceau>, i: usize) -> bool {
    let (debut, fin) = sous_chemin(t, i);
    let jum = jumeau(t, debut, fin);
    let sommets = (debut..fin).filter(|j| t[*j].fin().is_some() && Some(*j) != jum).count();
    if sommets <= 2 {
        if fin - debut == t.len() {
            return false;
        }
        t.drain(debut..fin);
        return true;
    }
    // Le jumeau se retire comme son départ
    let i = if Some(i) == jum { debut } else { i };
    let fondre = |avant: Morceau, apres: Morceau, p: P| match (avant, apres) {
        (Morceau::Cubique(c1, _, _), Morceau::Cubique(_, c2, _)) => Morceau::Cubique(c1, c2, p),
        (_, m) if !matches!(m, Morceau::Depart(_) | Morceau::Fermer) => m,
        _ => Morceau::Droite(p),
    };
    if i == debut {
        let a1 = t[debut + 1].fin().unwrap();
        if let Some(j) = jum {
            // Le dernier segment rejoint directement le deuxième sommet
            t[j] = fondre(t[j], t[debut + 1], a1);
        }
        t[debut] = Morceau::Depart(a1);
        t.remove(debut + 1);
        return true;
    }
    match t.get(i + 1).copied() {
        Some(m @ (Morceau::Cubique(..) | Morceau::Droite(_) | Morceau::Quadratique(..) | Morceau::Arc { .. })) if i + 1 < fin => {
            t[i + 1] = fondre(t[i], m, m.fin().unwrap());
            t.remove(i);
        }
        _ => {
            t.remove(i);
        }
    }
    true
}

/// L'indice du segment qui arrive au sommet `i` : le sien, ou pour un départ fermé,
/// celui de son jumeau ou le `Fermer`.
pub fn arrivee(t: &[Morceau], i: usize) -> Option<usize> {
    if !matches!(t[i], Morceau::Depart(_)) {
        return Some(i);
    }
    let (debut, fin) = sous_chemin(t, i);
    if t[fin - 1] != Morceau::Fermer {
        return None;
    }
    Some(jumeau(t, debut, fin).unwrap_or(fin - 1))
}

/// Vrai si le segment qui arrive à `i` est droit.
pub fn droit(t: &[Morceau], i: usize) -> Option<bool> {
    let s = arrivee(t, i)?;
    Some(match t[s] {
        Morceau::Droite(_) => true,
        Morceau::Fermer => {
            let (debut, _) = sous_chemin(t, s);
            jumeau(t, debut, s + 1).is_none()
        }
        _ => false,
    })
}

/// Le segment qui arrive au sommet `i` devient courbe s'il était droit, droit sinon.
pub fn basculer(t: &mut Vec<Morceau>, i: usize) -> bool {
    let Some(s) = arrivee(t, i) else { return false };
    let a = depart_de(t, s);
    match t[s] {
        Morceau::Droite(p) => t[s] = Morceau::Cubique(milieu(a, p, 1.0 / 3.0), milieu(a, p, 2.0 / 3.0), p),
        Morceau::Fermer => {
            let d = t[sous_chemin(t, s).0].fin().unwrap();
            t.insert(s, Morceau::Cubique(milieu(a, d, 1.0 / 3.0), milieu(a, d, 2.0 / 3.0), d));
        }
        m => {
            let p = m.fin().unwrap();
            let (debut, fin) = sous_chemin(t, s);
            // Le jumeau redevenu droit est inutile : le Fermer trace la droite
            if Some(s) == jumeau(t, debut, fin) {
                t.remove(s);
            } else {
                t[s] = Morceau::Droite(p);
            }
        }
    }
    true
}

/// Ouvre le sous-chemin du morceau `i` s'il est fermé, le ferme sinon.
pub fn ouvrir_fermer(t: &mut Vec<Morceau>, i: usize) {
    let (_, fin) = sous_chemin(t, i);
    if t[fin - 1] == Morceau::Fermer {
        t.remove(fin - 1);
    } else {
        t.insert(fin, Morceau::Fermer);
    }
}

/// Vrai si le sous-chemin du morceau `i` est fermé.
pub fn est_ferme(t: &[Morceau], i: usize) -> bool {
    let (_, fin) = sous_chemin(t, i);
    t[fin - 1] == Morceau::Fermer
}

// --- La simplification des tracés ------------------------------------------------

/// La tolérance « sans perte » : la moitié de la précision à laquelle `nombre`
/// écrit (trois décimales). Un sommet qui s'écarte moins que cela du tracé simplifié
/// ne change rien au fichier qu'on relira.
pub const SANS_PERTE: f32 = 5e-4;

/// Au-delà de ce virage (en degrés), un sommet est un angle vif : les courbes
/// ajustées s'y arrêtent au lieu de l'arrondir.
const ANGLE_VIF: f32 = 60.0;

/// Ce que deviennent les suites de droites (et de courbes) simplifiées.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ajustement {
    /// Des droites : les sommets gardés par Douglas-Peucker.
    Droites,
    /// Des cubiques ajustées par coupes successives (Schneider) : rapide.
    Rapide,
    /// Des cubiques en nombre minimal : les sommets sont choisis ensemble, par
    /// programmation dynamique, parmi des candidats.
    Optimal,
    /// Une B-spline cubique dont les nœuds se placent seuls (insertion là où l'écart
    /// est trop grand, puis retrait de ceux qui ne servent pas) : la courbure ne
    /// saute pas aux sommets. Écrite en cubiques, une par intervalle de nœuds.
    Spline,
}

impl Ajustement {
    pub const TOUS: [Ajustement; 4] = [Ajustement::Droites, Ajustement::Rapide, Ajustement::Optimal, Ajustement::Spline];
}

/// Comment simplifier un tracé.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Simplification {
    /// L'écart permis entre le tracé simplifié et l'original, dans les unités du
    /// tracé. `SANS_PERTE` (ou moins) : aucun écart visible.
    pub tolerance: f32,
    /// Droites ou courbes ; sans perte, toujours des droites.
    pub ajustement: Ajustement,
}

impl Simplification {
    pub const SANS_PERTE: Simplification = Simplification { tolerance: SANS_PERTE, ajustement: Ajustement::Droites };
}

/// Le nombre de sommets d'un tracé.
pub fn sommets(t: &[Morceau]) -> usize {
    t.iter().filter(|m| m.fin().is_some()).count()
}

/// Douglas-Peucker : les indices des points gardés (le premier et le dernier
/// toujours) pour qu'aucun point retiré ne s'écarte de plus de `tol` du tracé gardé.
/// L'écart se mesure au segment, pas à la droite : un aller-retour se garde.
pub fn douglas_peucker(points: &[P], tol: f32) -> Vec<usize> {
    let n = points.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let mut garde = vec![false; n];
    garde[0] = true;
    garde[n - 1] = true;
    let mut pile = vec![(0, n - 1)];
    while let Some((a, b)) = pile.pop() {
        let mut loin = (0.0f32, 0);
        for i in a + 1..b {
            let e = distance_segment(points[i], points[a], points[b]);
            if e > loin.0 {
                loin = (e, i);
            }
        }
        if loin.0 > tol {
            garde[loin.1] = true;
            pile.push((a, loin.1));
            pile.push((loin.1, b));
        }
    }
    (0..n).filter(|i| garde[*i]).collect()
}

/// Simplifie un tracé : retire les morceaux de longueur nulle, redresse les courbes
/// droites, retire les sommets qui ne s'écartent pas de plus de la tolérance (le
/// long des suites de droites), et, avec un ajustement en courbes, remplace ces
/// suites par des courbes de Bézier en gardant les angles vifs. Les arcs et la fermeture restent ; chaque
/// sous-chemin garde au moins un segment.
pub fn simplifier(t: &[Morceau], s: Simplification) -> Vec<Morceau> {
    let tol = s.tolerance.max(0.0);
    let aj = if tol > SANS_PERTE { s.ajustement } else { Ajustement::Droites };
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        let (debut, fin) = sous_chemin(t, i);
        simplifier_sous_chemin(&t[debut..fin], tol, aj, &mut out);
        i = fin;
    }
    out
}

/// Ce qu'une suite de points devient : droites gardées par Douglas-Peucker, ou
/// courbes ajustées.
fn suite(points: &[P], tol: f32, aj: Ajustement, out: &mut Vec<Morceau>) {
    if points.len() < 2 {
        return;
    }
    let gardes = douglas_peucker(points, tol);
    if aj == Ajustement::Droites || gardes.len() <= 2 {
        out.extend(gardes[1..].iter().map(|k| Morceau::Droite(points[*k])));
        return;
    }
    // Les angles vifs (parmi les sommets gardés) coupent la suite en morceaux lisses.
    // Le virage se mesure entre des points à quelques tolérances de part et d'autre,
    // pour ne pas prendre le tremblé pour un angle.
    let portee = tol * 4.0;
    let loin = |k: usize, avant: bool| -> P {
        let mut j = k;
        loop {
            let suivant = if avant { j.checked_sub(1) } else { (j + 1 < points.len()).then_some(j + 1) };
            let Some(s) = suivant else { return points[j] };
            j = s;
            if dist(points[j], points[k]) >= portee {
                return points[j];
            }
        }
    };
    let mut coupures = vec![0];
    for &k in &gardes[1..gardes.len() - 1] {
        let (a, b, c) = (loin(k, true), points[k], loin(k, false));
        let (u, v) = ((b.0 - a.0, b.1 - a.1), (c.0 - b.0, c.1 - b.1));
        let cos = (u.0 * v.0 + u.1 * v.1) / (u.0.hypot(u.1) * v.0.hypot(v.1)).max(1e-12);
        // Deux angles trop proches : le premier suffit
        if cos < ANGLE_VIF.to_radians().cos() && dist(points[*coupures.last().unwrap()], b) >= portee {
            coupures.push(k);
        }
    }
    coupures.push(points.len() - 1);
    coupures.dedup();
    for w in coupures.windows(2) {
        let morceau = &points[w[0]..=w[1]];
        // Ce qui est droit à la tolérance près reste une droite
        if douglas_peucker(morceau, tol).len() == 2 {
            out.push(Morceau::Droite(morceau[morceau.len() - 1]));
        } else {
            // Un léger lissage d'abord (le tremblé du doigt n'est pas à suivre)
            let lisse = lisser(morceau);
            let t1 = tangente(&lisse, tol, false);
            let t2 = tangente(&lisse, tol, true);
            match aj {
                Ajustement::Optimal => ajuster_optimal(&lisse, t1, t2, tol, out),
                Ajustement::Spline => ajuster_spline(&lisse, tol, out),
                _ => ajuster(&lisse, t1, t2, tol, out),
            }
        }
    }
}

fn simplifier_sous_chemin(t: &[Morceau], tol: f32, aj: Ajustement, out: &mut Vec<Morceau>) {
    let courbes = aj != Ajustement::Droites;
    let Some(Morceau::Depart(depart)) = t.first().copied() else {
        out.extend_from_slice(t);
        return;
    };
    let ferme = t.last() == Some(&Morceau::Fermer);
    let debut = out.len();
    out.push(Morceau::Depart(depart));
    let mut courant = depart;
    // La suite de points en cours (elle commence au point courant)
    let mut points = vec![depart];
    let eps = SANS_PERTE.min(tol.max(1e-6));
    for m in t[1..].iter().copied() {
        let m = match m {
            // Une courbe dont les contrôles sont sur la corde, entre ses bouts, est une droite
            Morceau::Cubique(c1, c2, p) if distance_segment(c1, courant, p) <= eps && distance_segment(c2, courant, p) <= eps && rang(courant, p, c1) <= rang(courant, p, c2) => Morceau::Droite(p),
            Morceau::Quadratique(c, p) if distance_segment(c, courant, p) <= eps => Morceau::Droite(p),
            m => m,
        };
        match m {
            Morceau::Droite(p) => {
                if dist(p, courant) > eps {
                    points.push(p);
                    courant = p;
                }
            }
            Morceau::Cubique(c1, c2, p) if courbes => {
                // Mise en points, pour être refondue avec ses voisines
                let n = ((dist(courant, c1) + dist(c1, c2) + dist(c2, p)) / tol.max(1e-3)).sqrt().ceil().clamp(4.0, 64.0) as usize;
                let a = courant;
                points.extend((1..=n).map(|k| point_cubique(a, c1, c2, p, k as f32 / n as f32)));
                courant = p;
            }
            Morceau::Quadratique(c, p) if courbes => {
                let n = ((dist(courant, c) + dist(c, p)) / tol.max(1e-3)).sqrt().ceil().clamp(4.0, 64.0) as usize;
                let a = courant;
                points.extend((1..=n).map(|k| {
                    let u = k as f32 / n as f32;
                    milieu(milieu(a, c, u), milieu(c, p, u), u)
                }));
                courant = p;
            }
            Morceau::Fermer => {}
            m => {
                // Une courbe, un arc : la suite s'arrête, le morceau reste (sauf s'il est nul)
                let p = m.fin().unwrap();
                let nul = match m {
                    Morceau::Cubique(c1, c2, _) => dist(c1, courant) <= eps && dist(c2, courant) <= eps && dist(p, courant) <= eps,
                    Morceau::Quadratique(c, _) => dist(c, courant) <= eps && dist(p, courant) <= eps,
                    _ => dist(p, courant) <= eps,
                };
                suite(&points, tol, aj, out);
                if !nul {
                    out.push(m);
                    courant = p;
                }
                points = vec![courant];
            }
        }
    }
    // Fermé, le dernier sommet qui retombe sur le départ est dit par Z
    if ferme && points.len() > 2 && dist(*points.last().unwrap(), depart) <= eps {
        points.pop();
    }
    suite(&points, tol, aj, out);
    if out.len() == debut + 1 {
        // Rien ne reste : un point (un trait rond) se garde comme un segment nul
        out.push(t[1..].iter().find_map(|m| if *m == Morceau::Fermer { None } else { Some(*m) }).unwrap_or(Morceau::Droite(depart)));
    }
    if ferme {
        out.push(Morceau::Fermer);
    }
}

/// Deux passes du noyau 1-2-1 ; les bouts restent.
fn lisser(points: &[P]) -> Vec<P> {
    let mut p = points.to_vec();
    for _ in 0..2 {
        let q = p.clone();
        for k in 1..q.len().saturating_sub(1) {
            p[k] = ((q[k - 1].0 + 2.0 * q[k].0 + q[k + 1].0) / 4.0, (q[k - 1].1 + 2.0 * q[k].1 + q[k + 1].1) / 4.0);
        }
    }
    p
}

/// La position de `c` le long de `a`–`b` (0 en `a`, 1 en `b`).
fn rang(a: P, b: P, c: P) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    ((c.0 - a.0) * dx + (c.1 - a.1) * dy) / (dx * dx + dy * dy).max(1e-12)
}

fn point_cubique(a: P, b: P, c: P, d: P, u: f32) -> P {
    let v = 1.0 - u;
    let (k0, k1, k2, k3) = (v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u);
    (k0 * a.0 + k1 * b.0 + k2 * c.0 + k3 * d.0, k0 * a.1 + k1 * b.1 + k2 * c.1 + k3 * d.1)
}

fn unitaire(v: P) -> P {
    let l = v.0.hypot(v.1);
    if l < 1e-12 { (0.0, 0.0) } else { (v.0 / l, v.1 / l) }
}

/// La tangente à un bout (vers l'intérieur) : vers le premier point qui s'en écarte
/// de plus de la tolérance, pour ne pas suivre le tremblé du doigt.
fn tangente(points: &[P], tol: f32, fin: bool) -> P {
    let n = points.len();
    let bout = if fin { points[n - 1] } else { points[0] };
    let suivant = |k: usize| if fin { points[n - 1 - k] } else { points[k] };
    let k = (1..n).find(|k| dist(suivant(*k), bout) > tol * 4.0).unwrap_or(1).min(n - 1);
    unitaire((suivant(k).0 - bout.0, suivant(k).1 - bout.1))
}

/// L'ajustement de Schneider (Graphics Gems, 1990) : une cubique dont les tangentes
/// aux bouts sont données, par moindres carrés, reparamétrée par Newton ; coupée en
/// deux au point le plus loin tant que l'écart dépasse la tolérance. Puis une passe
/// de fusion retire les coupures dont les deux voisines tiennent en une courbe.
fn ajuster(points: &[P], t1: P, t2: P, tol: f32, out: &mut Vec<Morceau>) {
    let n = points.len();
    let mut coupures = vec![0, n - 1];
    couper_ajustement(points, 0, n - 1, t1, t2, tol, &mut coupures, 0);
    coupures.sort_unstable();
    coupures.dedup();
    // La tangente à un bout de [a, b] : celle de la suite à ses bouts, celle du
    // point de coupure ailleurs
    let tangentes = |a: usize, b: usize| {
        let ta = if a == 0 { t1 } else { tangente_en(points, a, tol) };
        let tb = if b == n - 1 { t2 } else { let c = tangente_en(points, b, tol); (-c.0, -c.1) };
        (ta, tb)
    };
    let mut fusion = true;
    while fusion && coupures.len() > 2 {
        fusion = false;
        // La coupure dont la fusion s'écarte le moins, si elle tient
        let mut meilleure: Option<(f32, usize)> = None;
        for i in 1..coupures.len() - 1 {
            let (a, b) = (coupures[i - 1], coupures[i + 1]);
            let (ta, tb) = tangentes(a, b);
            let (_, ecart, _) = cubique_au_mieux(&points[a..=b], ta, tb, tol);
            if ecart <= tol && meilleure.is_none_or(|m| ecart < m.0) {
                meilleure = Some((ecart, i));
            }
        }
        if let Some((_, i)) = meilleure {
            coupures.remove(i);
            fusion = true;
        }
    }
    for w in coupures.windows(2) {
        let (ta, tb) = tangentes(w[0], w[1]);
        let (c, _, _) = cubique_au_mieux(&points[w[0]..=w[1]], ta, tb, tol);
        out.push(Morceau::Cubique(c[1], c[2], c[3]));
    }
}

/// La tangente (vers l'avant) au point `k`, entre des points pris à quelques
/// tolérances de part et d'autre (les voisins immédiats tremblent).
fn tangente_en(points: &[P], k: usize, tol: f32) -> P {
    let n = points.len();
    let p = points[k];
    let a = (0..k).rev().find(|j| dist(points[*j], p) >= tol * 4.0).unwrap_or(0);
    let b = (k + 1..n).find(|j| dist(points[*j], p) >= tol * 4.0).unwrap_or(n - 1);
    let t = unitaire((points[b].0 - points[a].0, points[b].1 - points[a].1));
    if t == (0.0, 0.0) && k > 0 { unitaire((p.0 - points[k - 1].0, p.1 - points[k - 1].1)) } else { t }
}

/// Coupe [a, b] au point le plus loin tant que la meilleure cubique s'en écarte de
/// plus de la tolérance ; les coupures s'ajoutent à `coupures`.
#[allow(clippy::too_many_arguments)]
fn couper_ajustement(points: &[P], a: usize, b: usize, t1: P, t2: P, tol: f32, coupures: &mut Vec<usize>, profondeur: u8) {
    if b - a < 2 || profondeur >= 24 {
        return;
    }
    let (_, ecart, loin) = cubique_au_mieux(&points[a..=b], t1, t2, tol);
    if ecart <= tol {
        return;
    }
    // Pas trop près des bouts : un morceau minuscule ne ferait que suivre le tremblé
    let marge = ((b - a) / 4).max(1);
    let k = a + loin.clamp(marge, b - a - marge);
    coupures.push(k);
    let c = tangente_en(points, k, tol);
    couper_ajustement(points, a, k, t1, (-c.0, -c.1), tol, coupures, profondeur + 1);
    couper_ajustement(points, k, b, c, t2, tol, coupures, profondeur + 1);
}

/// La meilleure cubique sur ces points aux tangentes données : (contrôles, écart,
/// indice du point le plus loin).
fn cubique_au_mieux(points: &[P], t1: P, t2: P, tol: f32) -> ([P; 4], f32, usize) {
    let n = points.len();
    let (a, b) = (points[0], points[n - 1]);
    if n == 2 {
        let d = dist(a, b) / 3.0;
        return ([a, (a.0 + t1.0 * d, a.1 + t1.1 * d), (b.0 + t2.0 * d, b.1 + t2.1 * d), b], 0.0, 0);
    }
    // Paramètres à la longueur de corde
    let mut u = vec![0.0f32; n];
    for k in 1..n {
        u[k] = u[k - 1] + dist(points[k], points[k - 1]);
    }
    let total = u[n - 1].max(1e-12);
    u.iter_mut().for_each(|x| *x /= total);
    let mut meilleure: Option<([P; 4], f32, usize)> = None;
    for essai in 0..5 {
        let c = cubique_ajustee(points, &u, t1, t2);
        let (ecart, loin) = ecart_max(points, &u, &c);
        if meilleure.as_ref().is_none_or(|m| ecart < m.1) {
            meilleure = Some((c, ecart, loin));
        }
        if ecart <= tol || essai == 4 {
            break;
        }
        // Newton-Raphson : chaque paramètre glisse vers le point le plus proche
        for k in 1..n - 1 {
            u[k] = newton(&c, points[k], u[k]);
        }
    }
    meilleure.unwrap()
}

/// La segmentation optimale (Plass et Stone, 1983) : parmi des sommets candidats (les
/// points gardés par Douglas-Peucker à un cinquième de la tolérance), le plus petit
/// nombre de cubiques qui tiennent la tolérance, cherché par programmation
/// dynamique. Les tangentes sont celles de la suite en chaque candidat : les cubiques
/// se raccordent sans angle.
fn ajuster_optimal(points: &[P], t1: P, t2: P, tol: f32, out: &mut Vec<Morceau>) {
    let n = points.len();
    let mut candidats = douglas_peucker(points, tol * 0.2);
    if candidats.len() > 400 {
        candidats = douglas_peucker(points, tol * 0.5);
    }
    let m = candidats.len();
    // La tangente vers l'avant en chaque candidat
    let tangentes: Vec<P> = candidats.iter().map(|&k| if k == 0 { t1 } else if k == n - 1 { (-t2.0, -t2.1) } else { tangente_en(points, k, tol) }).collect();
    let essai = |i: usize, j: usize| {
        let (a, b) = (candidats[i], candidats[j]);
        let tb = tangentes[j];
        cubique_au_mieux(&points[a..=b], tangentes[i], (-tb.0, -tb.1), tol)
    };
    // meilleur[j] : (nombre de cubiques jusqu'au candidat j, écart total, précédent)
    let mut meilleur: Vec<(usize, f32, usize)> = vec![(usize::MAX, 0.0, 0); m];
    meilleur[0] = (0, 0.0, 0);
    for j in 1..m {
        let mut echecs = 0;
        for i in (0..j).rev() {
            let (_, ecart, _) = essai(i, j);
            if ecart <= tol {
                echecs = 0;
                let c = (meilleur[i].0 + 1, meilleur[i].1 + ecart, i);
                if c.0 < meilleur[j].0 || (c.0 == meilleur[j].0 && c.1 < meilleur[j].1) {
                    meilleur[j] = c;
                }
            } else {
                // Plus loin, ça ne tiendra pas davantage
                echecs += 1;
                if echecs >= 3 {
                    break;
                }
            }
        }
        if meilleur[j].0 == usize::MAX {
            // Rien ne tient (des tangentes contraires) : la plus courte, quand même
            meilleur[j] = (meilleur[j - 1].0 + 1, meilleur[j - 1].1, j - 1);
        }
    }
    let mut chaine = vec![m - 1];
    while *chaine.last().unwrap() != 0 {
        chaine.push(meilleur[*chaine.last().unwrap()].2);
    }
    chaine.reverse();
    for w in chaine.windows(2) {
        let (c, _, _) = essai(w[0], w[1]);
        out.push(Morceau::Cubique(c[1], c[2], c[3]));
    }
}

// --- Les B-splines à nœuds automatiques ---

/// Les quatre fonctions de base cubiques non nulles en `u`, et l'intervalle `s`
/// (`t[s] <= u < t[s+1]`) : le point vaut la somme des `n[r] * P[s-3+r]`.
fn base_spline(t: &[f64], u: f64) -> (usize, [f64; 4]) {
    let m = t.len() - 4; // nombre de contrôles
    // L'intervalle non vide qui contient u (le dernier pour u = 1)
    let mut s = 3;
    while s + 1 < m && t[s + 1] <= u {
        s += 1;
    }
    let mut n = [0.0f64; 4];
    n[0] = 1.0;
    let (mut gauche, mut droite) = ([0.0f64; 4], [0.0f64; 4]);
    for j in 1..4 {
        gauche[j] = u - t[s + 1 - j];
        droite[j] = t[s + j] - u;
        let mut reste = 0.0;
        for r in 0..j {
            let d = droite[r + 1] + gauche[j - r];
            let q = if d.abs() < 1e-15 { 0.0 } else { n[r] / d };
            n[r] = reste + droite[r + 1] * q;
            reste = gauche[j - r] * q;
        }
        n[j] = reste;
    }
    (s, n)
}

/// Le point du blossom de l'intervalle `s` en (x1, x2, x3) : avec a, a, a puis
/// a, a, b… on obtient les contrôles de Bézier de l'intervalle [a, b].
fn blossom(t: &[f64], c: &[(f64, f64)], s: usize, x: [f64; 3]) -> (f64, f64) {
    let mut d = [c[s - 3], c[s - 2], c[s - 1], c[s]];
    for r in 1..=3 {
        for j in (r..=3).rev() {
            let i = s - 3 + j;
            let den = t[i + 4 - r] - t[i];
            let a = if den.abs() < 1e-15 { 0.0 } else { (x[r - 1] - t[i]) / den };
            d[j] = ((1.0 - a) * d[j - 1].0 + a * d[j].0, (1.0 - a) * d[j - 1].1 + a * d[j].1);
        }
    }
    d[3]
}

/// La B-spline en cubiques de Bézier, une par intervalle non vide, avec ses bornes.
fn spline_en_cubiques(t: &[f64], c: &[(f64, f64)]) -> Vec<(f64, f64, [P; 4])> {
    let mut v = Vec::new();
    for s in 3..c.len() {
        let (a, b) = (t[s], t[s + 1]);
        if b - a < 1e-12 {
            continue;
        }
        let q = |x: [f64; 3]| {
            let p = blossom(t, c, s, x);
            (p.0 as f32, p.1 as f32)
        };
        v.push((a, b, [q([a, a, a]), q([a, a, b]), q([a, b, b]), q([b, b, b])]));
    }
    v
}

/// Les contrôles de la B-spline de nœuds intérieurs `interieurs` la plus proche des
/// points (aux paramètres `u`) par moindres carrés, les bouts passant par le premier
/// et le dernier point. Une légère pénalité sur les différences secondes des
/// contrôles (à la façon des P-splines) tient les intervalles sans points.
fn moindres_carres_spline(points: &[P], u: &[f64], interieurs: &[f64]) -> (Vec<f64>, Vec<(f64, f64)>) {
    let mut t = vec![0.0f64; 4];
    t.extend_from_slice(interieurs);
    t.extend_from_slice(&[1.0; 4]);
    let m = t.len() - 4;
    let n = points.len();
    let (p0, p1) = ((points[0].0 as f64, points[0].1 as f64), (points[n - 1].0 as f64, points[n - 1].1 as f64));
    // Inconnues : les contrôles 1..m-1 (exclus) ; matrice normale bande de demi-largeur 3
    let q = m - 2;
    let mut a = vec![vec![0.0f64; q]; q];
    let mut b = vec![(0.0f64, 0.0f64); q];
    let mut ligne = |coefs: &[(usize, f64)], cible: (f64, f64), poids: f64| {
        let mut c = cible;
        let libres: Vec<(usize, f64)> = coefs
            .iter()
            .filter_map(|&(i, w)| {
                if i == 0 {
                    c = (c.0 - w * p0.0, c.1 - w * p0.1);
                    None
                } else if i == m - 1 {
                    c = (c.0 - w * p1.0, c.1 - w * p1.1);
                    None
                } else {
                    Some((i - 1, w))
                }
            })
            .collect();
        for &(i, wi) in &libres {
            b[i].0 += poids * wi * c.0;
            b[i].1 += poids * wi * c.1;
            for &(j, wj) in &libres {
                a[i][j] += poids * wi * wj;
            }
        }
    };
    for k in 0..n {
        let (s, nb) = base_spline(&t, u[k]);
        let coefs: Vec<(usize, f64)> = (0..4).map(|r| (s - 3 + r, nb[r])).collect();
        ligne(&coefs, (points[k].0 as f64, points[k].1 as f64), 1.0);
    }
    let lambda = 1e-4 * n as f64 / m as f64;
    for i in 1..m - 1 {
        ligne(&[(i - 1, 1.0), (i, -2.0), (i + 1, 1.0)], (0.0, 0.0), lambda);
    }
    // Cholesky dans la bande
    let mut l = vec![vec![0.0f64; q]; q];
    for i in 0..q {
        for j in i.saturating_sub(3)..=i {
            let mut s = a[i][j];
            for k in i.saturating_sub(3)..j {
                s -= l[i][k] * l[j][k];
            }
            l[i][j] = if i == j { s.max(1e-12).sqrt() } else { s / l[j][j] };
        }
    }
    let mut y = vec![(0.0f64, 0.0f64); q];
    for i in 0..q {
        let mut s = b[i];
        for k in i.saturating_sub(3)..i {
            s = (s.0 - l[i][k] * y[k].0, s.1 - l[i][k] * y[k].1);
        }
        y[i] = (s.0 / l[i][i], s.1 / l[i][i]);
    }
    let mut x = vec![(0.0f64, 0.0f64); q];
    for i in (0..q).rev() {
        let mut s = y[i];
        for k in i + 1..(i + 4).min(q) {
            s = (s.0 - l[k][i] * x[k].0, s.1 - l[k][i] * x[k].1);
        }
        x[i] = (s.0 / l[i][i], s.1 / l[i][i]);
    }
    let mut c = vec![p0];
    c.extend(x);
    c.push(p1);
    (t, c)
}

/// L'écart de chaque point à la courbe (vraie distance, sur la courbe mise en
/// points fins autour de son paramètre), et le paramètre du point le plus proche.
fn ecarts_spline(points: &[P], u: &[f64], cubiques: &[(f64, f64, [P; 4])]) -> Vec<(f32, f64)> {
    const FIN: usize = 24;
    let fins: Vec<Vec<(P, f64)>> = cubiques
        .iter()
        .map(|(a, b, c)| (0..=FIN).map(|k| {
            let v = k as f32 / FIN as f32;
            (point_cubique(c[0], c[1], c[2], c[3], v), a + (b - a) * v as f64)
        }).collect())
        .collect();
    // Le plus proche sur une mise en points : (carré de la distance, paramètre)
    let proche = |p: P, f: &[(P, f64)], m: &mut (f32, f64)| {
        for w in f.windows(2) {
            let (a, b) = (w[0].0, w[1].0);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l = dx * dx + dy * dy;
            let v = if l > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l).clamp(0.0, 1.0) } else { 0.0 };
            let (ex, ey) = (p.0 - a.0 - v * dx, p.1 - a.1 - v * dy);
            let e = ex * ex + ey * ey;
            if e < m.0 {
                *m = (e, w[0].1 + (w[1].1 - w[0].1) * v as f64);
            }
        }
    };
    points
        .iter()
        .zip(u)
        .map(|(p, &uk)| {
            let i = cubiques.partition_point(|c| c.1 < uk).min(cubiques.len() - 1);
            let mut m = (f32::MAX, uk);
            proche(*p, &fins[i], &mut m);
            // Les intervalles voisins, seulement près d'une borne
            let (a, b) = (cubiques[i].0, cubiques[i].1);
            if i > 0 && uk - a < (b - a) * 0.25 {
                proche(*p, &fins[i - 1], &mut m);
            }
            if i + 1 < fins.len() && b - uk < (b - a) * 0.25 {
                proche(*p, &fins[i + 1], &mut m);
            }
            (m.0.sqrt(), m.1)
        })
        .collect()
}

/// Ajuste la B-spline de nœuds donnés, en reparamétrant `reprises` fois les points
/// vers leur plus proche ; rend la courbe en cubiques et les écarts.
fn essai_spline(points: &[P], u: &mut [f64], interieurs: &[f64], reprises: usize) -> (Vec<(f64, f64, [P; 4])>, Vec<(f32, f64)>) {
    let mut res = None;
    for passe in 0..=reprises {
        let (t, c) = moindres_carres_spline(points, u, interieurs);
        let cubiques = spline_en_cubiques(&t, &c);
        let ecarts = ecarts_spline(points, u, &cubiques);
        if passe < reprises {
            let n = u.len();
            for k in 1..n - 1 {
                u[k] = ecarts[k].1.clamp(0.0, 1.0);
            }
            // L'ordre des paramètres reste celui des points
            for k in 1..n {
                if u[k] < u[k - 1] {
                    u[k] = u[k - 1];
                }
            }
        }
        res = Some((cubiques, ecarts));
    }
    res.unwrap()
}

/// La B-spline cubique à nœuds automatiques : on part d'une seule cubique, on
/// insère un nœud au point le plus loin de chaque intervalle qui dépasse la
/// tolérance (Dierckx), puis on retire un à un les nœuds dont l'absence tient encore
/// la tolérance (Lyche et Mørken). La courbe garde sa courbure continue aux sommets.
fn ajuster_spline(points: &[P], tol: f32, out: &mut Vec<Morceau>) {
    let n = points.len();
    let mut u0 = vec![0.0f64; n];
    for k in 1..n {
        u0[k] = u0[k - 1] + dist(points[k], points[k - 1]) as f64;
    }
    let total = u0[n - 1].max(1e-12);
    u0.iter_mut().for_each(|x| *x /= total);
    let mut interieurs: Vec<f64> = Vec::new();
    let max_noeuds = (n / 3).max(1);
    let depasse = |cubiques: &[(f64, f64, [P; 4])], ecarts: &[(f32, f64)], u: &[f64]| -> Vec<(usize, f64)> {
        // Par intervalle, le point le plus loin s'il dépasse
        let mut pire: Vec<Option<(f32, f64)>> = vec![None; cubiques.len()];
        for (k, e) in ecarts.iter().enumerate() {
            if e.0 > tol {
                let i = cubiques.partition_point(|c| c.1 < u[k]).min(cubiques.len() - 1);
                if pire[i].is_none_or(|p| e.0 > p.0) {
                    pire[i] = Some((e.0, u[k]));
                }
            }
        }
        pire.iter().enumerate().filter_map(|(i, p)| p.map(|p| (i, p.1))).collect()
    };
    // Insertion
    let mut u = u0.clone();
    for _ in 0..40 {
        let (cubiques, ecarts) = essai_spline(points, &mut u, &interieurs, 2);
        let trop = depasse(&cubiques, &ecarts, &u);
        if trop.is_empty() || interieurs.len() >= max_noeuds {
            break;
        }
        let avant = interieurs.len();
        for (i, v) in trop {
            let (a, b) = (cubiques[i].0, cubiques[i].1);
            // Pas trop près des bornes : au milieu sinon
            let v = if v - a < (b - a) * 0.2 || b - v < (b - a) * 0.2 { (a + b) / 2.0 } else { v };
            if !interieurs.iter().any(|x| (x - v).abs() < 1e-9) {
                interieurs.push(v);
            }
        }
        interieurs.sort_by(f64::total_cmp);
        if interieurs.len() == avant {
            break;
        }
    }
    // Retrait : chaque nœud dont l'absence tient la tolérance s'en va
    let mut k = 0;
    while k < interieurs.len() {
        let mut sans = interieurs.clone();
        sans.remove(k);
        let mut v = u.clone();
        let (cubiques, ecarts) = essai_spline(points, &mut v, &sans, 0);
        if depasse(&cubiques, &ecarts, &v).is_empty() {
            interieurs = sans;
            u = v;
        } else {
            k += 1;
        }
    }
    let (cubiques, _) = essai_spline(points, &mut u, &interieurs, 1);
    out.extend(cubiques.iter().map(|(_, _, c)| Morceau::Cubique(c[1], c[2], c[3])));
}

/// La cubique aux tangentes `t1`, `t2` qui passe au mieux par les points (aux
/// paramètres `u`), par moindres carrés sur la longueur des deux contrôles.
fn cubique_ajustee(points: &[P], u: &[f32], t1: P, t2: P) -> [P; 4] {
    let n = points.len();
    let (a, b) = (points[0], points[n - 1]);
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for k in 0..n {
        let t = u[k];
        let v = 1.0 - t;
        let (b0, b1, b2, b3) = (v * v * v, 3.0 * t * v * v, 3.0 * t * t * v, t * t * t);
        let (a1, a2) = ((t1.0 * b1, t1.1 * b1), (t2.0 * b2, t2.1 * b2));
        c00 += a1.0 * a1.0 + a1.1 * a1.1;
        c01 += a1.0 * a2.0 + a1.1 * a2.1;
        c11 += a2.0 * a2.0 + a2.1 * a2.1;
        let r = (points[k].0 - (a.0 * (b0 + b1) + b.0 * (b2 + b3)), points[k].1 - (a.1 * (b0 + b1) + b.1 * (b2 + b3)));
        x0 += a1.0 * r.0 + a1.1 * r.1;
        x1 += a2.0 * r.0 + a2.1 * r.1;
    }
    let det = c00 * c11 - c01 * c01;
    let (mut la, mut lb) = if det.abs() > 1e-12 { ((x0 * c11 - x1 * c01) / det, (c00 * x1 - c01 * x0) / det) } else { (0.0, 0.0) };
    // Un contrôle à rebours ou collé : l'heuristique du tiers de la corde
    let corde = dist(a, b);
    if la < corde * 1e-3 || lb < corde * 1e-3 || la > corde * 3.0 || lb > corde * 3.0 {
        la = corde / 3.0;
        lb = corde / 3.0;
    }
    [a, (a.0 + t1.0 * la, a.1 + t1.1 * la), (b.0 + t2.0 * lb, b.1 + t2.1 * lb), b]
}

/// L'écart le plus grand entre les points et la cubique (à leurs paramètres), et
/// l'indice du point le plus loin.
fn ecart_max(points: &[P], u: &[f32], c: &[P; 4]) -> (f32, usize) {
    let mut m = (0.0f32, points.len() / 2);
    for k in 1..points.len() - 1 {
        let e = dist(point_cubique(c[0], c[1], c[2], c[3], u[k]), points[k]);
        if e > m.0 {
            m = (e, k);
        }
    }
    m
}

/// Un pas de Newton vers le paramètre de la cubique le plus proche de `p`.
fn newton(c: &[P; 4], p: P, u: f32) -> f32 {
    let q = point_cubique(c[0], c[1], c[2], c[3], u);
    let d1: Vec<P> = (0..3).map(|k| (3.0 * (c[k + 1].0 - c[k].0), 3.0 * (c[k + 1].1 - c[k].1))).collect();
    let d2: Vec<P> = (0..2).map(|k| (2.0 * (d1[k + 1].0 - d1[k].0), 2.0 * (d1[k + 1].1 - d1[k].1))).collect();
    let v = 1.0 - u;
    let q1 = (v * v * d1[0].0 + 2.0 * u * v * d1[1].0 + u * u * d1[2].0, v * v * d1[0].1 + 2.0 * u * v * d1[1].1 + u * u * d1[2].1);
    let q2 = (v * d2[0].0 + u * d2[1].0, v * d2[0].1 + u * d2[1].1);
    let num = (q.0 - p.0) * q1.0 + (q.1 - p.1) * q1.1;
    let den = q1.0 * q1.0 + q1.1 * q1.1 + (q.0 - p.0) * q2.0 + (q.1 - p.1) * q2.1;
    if den.abs() < 1e-12 { u } else { (u - num / den).clamp(0.0, 1.0) }
}

impl Document {
    /// Simplifie les chemins, polygones et polylignes de `k` et de ses descendants
    /// (la tolérance est dans les unités du document : chacun la ramène aux siennes).
    /// Rend le nombre de sommets avant et après.
    pub fn simplifier(&mut self, k: usize, s: Simplification) -> (usize, usize) {
        let (mut avant, mut apres) = (0, 0);
        for n in 0..self.noeuds.len() {
            if !self.sous(n, k) || !matches!(self.noeuds[n].nom.as_str(), "path" | "polygon" | "polyline") {
                continue;
            }
            let Some(t) = trace_de(&self.noeuds[n]) else { continue };
            let f = self.ctm(n).facteur().max(1e-6);
            let u = simplifier(&t, Simplification { tolerance: s.tolerance / f, ..s });
            avant += sommets(&t);
            apres += sommets(&u);
            if u != t {
                self.poser_trace(n, &u);
            }
        }
        (avant, apres)
    }
}

// --- Le rendu en formes ----------------------------------------------------------

/// Un texte à dessiner : sa ligne de base (en coordonnées du document), sa taille
/// et son ancrage (-1 début, 0 milieu, 1 fin).
#[derive(Clone, Debug, PartialEq)]
pub struct TexteSvg {
    pub chaine: String,
    pub x: f32,
    pub y: f32,
    pub taille: f32,
    pub ancre: i8,
}

/// Ce qu'il faut dessiner pour un élément, dans les coordonnées du document.
#[derive(Clone, Debug, PartialEq)]
pub struct Forme {
    /// L'élément d'origine.
    pub noeud: usize,
    pub contours: Contours,
    pub fond: Option<Color>,
    pub pair_impair: bool,
    pub trait_: Option<Color>,
    /// L'épaisseur du trait, en unités du document.
    pub epaisseur: f32,
    pub texte: Option<TexteSvg>,
    /// Le rectangle qui l'enveloppe : x0, y0, x1, y1.
    pub boite: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
enum Peinture {
    Aucune,
    Couleur(Color),
    /// Un dégradé, réduit à une couleur.
    Url(String),
    /// La couleur de `color`.
    Courante,
}

#[derive(Clone, Debug)]
struct Heritage {
    fond: Peinture,
    trait_: Peinture,
    epaisseur: f32,
    opacite_fond: f32,
    opacite_trait: f32,
    pair_impair: bool,
    visible: bool,
    taille: f32,
    ancre: i8,
    couleur: Color,
    /// Le produit des `opacity` des ancêtres.
    opacite: f32,
}

impl Default for Heritage {
    fn default() -> Heritage {
        Heritage { fond: Peinture::Couleur([0, 0, 0, 255]), trait_: Peinture::Aucune, epaisseur: 1.0, opacite_fond: 1.0, opacite_trait: 1.0, pair_impair: false, visible: true, taille: 16.0, ancre: -1, couleur: [0, 0, 0, 255], opacite: 1.0 }
    }
}

/// Une propriété : dans `style`, sinon en attribut.
fn propriete<'a>(n: &'a Noeud, nom: &str) -> Option<&'a str> {
    if let Some(st) = n.attr("style") {
        for d in st.split(';') {
            if let Some((k, v)) = d.split_once(':') {
                if k.trim() == nom {
                    return Some(v.trim().trim_end_matches("!important").trim());
                }
            }
        }
    }
    n.attr(nom).map(str::trim)
}

fn peinture(v: &str) -> Option<Peinture> {
    let v = v.trim();
    if v == "none" || v == "transparent" {
        return Some(Peinture::Aucune);
    }
    if v == "currentColor" {
        return Some(Peinture::Courante);
    }
    if let Some(r) = v.strip_prefix("url(") {
        let id = r.split(')').next()?.trim().trim_matches(['\'', '"']).trim_start_matches('#');
        return Some(Peinture::Url(id.to_string()));
    }
    if v == "inherit" {
        return None;
    }
    couleur(v).map(Peinture::Couleur)
}

/// La couleur moyenne des arrêts d'un dégradé (en suivant `href`).
fn degrade(doc: &Document, id: &str, profondeur: u8) -> Option<Color> {
    let k = doc.par_id(id)?;
    let n = &doc.noeuds[k];
    let arrets: Vec<Color> = n
        .enfants
        .iter()
        .filter(|e| doc.noeuds[**e].nom == "stop")
        .filter_map(|e| {
            let s = &doc.noeuds[*e];
            let mut c = couleur(propriete(s, "stop-color").unwrap_or("black"))?;
            let o = propriete(s, "stop-opacity").and_then(|o| o.parse::<f32>().ok()).unwrap_or(1.0);
            c[3] = (c[3] as f32 * o.clamp(0.0, 1.0)).round() as u8;
            Some(c)
        })
        .collect();
    if arrets.is_empty() {
        let lien = n.attr("href").or(n.attr("xlink:href"))?.trim_start_matches('#');
        return if profondeur < 4 { degrade(doc, lien, profondeur + 1) } else { None };
    }
    let m = |i: usize| (arrets.iter().map(|c| c[i] as u32).sum::<u32>() / arrets.len() as u32) as u8;
    Some([m(0), m(1), m(2), m(3)])
}

/// Les éléments qui ne se dessinent pas eux-mêmes.
const INVISIBLES: [&str; 14] = ["defs", "clipPath", "mask", "linearGradient", "radialGradient", "pattern", "symbol", "style", "title", "desc", "metadata", "marker", "filter", "script"];

/// Réduit le document en formes, dans l'ordre de dessin.
pub fn rendu(doc: &Document) -> Vec<Forme> {
    let mut out = Vec::new();
    let racine = &doc.noeuds[doc.racine];
    let h = heriter(doc, racine, &Heritage::default());
    for e in &racine.enfants {
        parcourir(doc, *e, Matrice::IDENTITE, &h, &mut out, 0);
    }
    out
}

fn heriter(doc: &Document, n: &Noeud, h: &Heritage) -> Heritage {
    let mut h = h.clone();
    let nb = |v: &str| v.trim().trim_end_matches('%').parse::<f32>().ok().map(|x| if v.trim().ends_with('%') { x / 100.0 } else { x });
    if let Some(c) = propriete(n, "color").and_then(couleur) {
        h.couleur = c;
    }
    if let Some(p) = propriete(n, "fill").and_then(peinture) {
        h.fond = p;
    }
    if let Some(p) = propriete(n, "stroke").and_then(peinture) {
        h.trait_ = p;
    }
    if let Some(v) = propriete(n, "stroke-width").and_then(|v| longueur(v, 100.0)) {
        h.epaisseur = v.max(0.0);
    }
    if let Some(v) = propriete(n, "fill-opacity").and_then(nb) {
        h.opacite_fond = v.clamp(0.0, 1.0);
    }
    if let Some(v) = propriete(n, "stroke-opacity").and_then(nb) {
        h.opacite_trait = v.clamp(0.0, 1.0);
    }
    if let Some(v) = propriete(n, "opacity").and_then(nb) {
        h.opacite *= v.clamp(0.0, 1.0);
    }
    if let Some(v) = propriete(n, "fill-rule") {
        h.pair_impair = v == "evenodd";
    }
    if let Some(v) = propriete(n, "visibility") {
        h.visible = v == "visible";
    }
    if let Some(v) = propriete(n, "font-size").and_then(|v| longueur(v, h.taille * 100.0)) {
        h.taille = v;
    }
    if let Some(v) = propriete(n, "text-anchor") {
        h.ancre = match v {
            "middle" => 0,
            "end" => 1,
            _ => -1,
        };
    }
    let _ = doc;
    h
}

fn resoudre(doc: &Document, p: &Peinture, h: &Heritage, opacite: f32) -> Option<Color> {
    let mut c = match p {
        Peinture::Aucune => return None,
        Peinture::Couleur(c) => *c,
        Peinture::Courante => h.couleur,
        Peinture::Url(id) => degrade(doc, id, 0).unwrap_or([0, 0, 0, 255]),
    };
    c[3] = (c[3] as f32 * opacite * h.opacite).round().clamp(0.0, 255.0) as u8;
    (c[3] > 0).then_some(c)
}

fn attr_nb(n: &Noeud, nom: &str, reference: f32) -> f32 {
    n.attr(nom).and_then(|v| longueur(v, reference)).unwrap_or(0.0)
}

fn parcourir(doc: &Document, k: usize, ctm: Matrice, h: &Heritage, out: &mut Vec<Forme>, profondeur: u8) {
    let n = &doc.noeuds[k];
    if !n.element() || INVISIBLES.contains(&n.nom.as_str()) || profondeur > 32 {
        return;
    }
    if propriete(n, "display") == Some("none") {
        return;
    }
    let h = heriter(doc, n, h);
    let mut ctm = ctm.fois(&transformation(n.attr("transform").unwrap_or("")));
    let s = ctm.facteur().max(1e-3);
    let local: Contours = match n.nom.as_str() {
        "g" | "a" | "switch" => {
            for e in &n.enfants {
                parcourir(doc, *e, ctm, &h, out, profondeur + 1);
            }
            return;
        }
        "svg" => {
            ctm = ctm.fois(&Matrice::translation(attr_nb(n, "x", 0.0), attr_nb(n, "y", 0.0)));
            for e in &n.enfants {
                parcourir(doc, *e, ctm, &h, out, profondeur + 1);
            }
            return;
        }
        "use" => {
            let Some(cible) = n.attr("href").or(n.attr("xlink:href")).and_then(|l| doc.par_id(l.trim_start_matches('#'))) else { return };
            if doc.sous(k, cible) {
                return;
            }
            let t = ctm.fois(&Matrice::translation(attr_nb(n, "x", 0.0), attr_nb(n, "y", 0.0)));
            let avant = out.len();
            if doc.noeuds[cible].nom == "symbol" {
                for e in &doc.noeuds[cible].enfants {
                    parcourir(doc, *e, t, &h, out, profondeur + 1);
                }
            } else {
                parcourir(doc, cible, t, &h, out, profondeur + 1);
            }
            // Ce qu'un `use` dessine lui appartient (pour le toucher)
            for f in &mut out[avant..] {
                f.noeud = k;
            }
            return;
        }
        "rect" => {
            let (x, y) = (attr_nb(n, "x", 100.0), attr_nb(n, "y", 100.0));
            let (w, hh) = (attr_nb(n, "width", 100.0), attr_nb(n, "height", 100.0));
            if w <= 0.0 || hh <= 0.0 {
                return;
            }
            let (mut rx, mut ry) = (n.attr("rx").and_then(|v| longueur(v, w)), n.attr("ry").and_then(|v| longueur(v, hh)));
            if rx.is_none() {
                rx = ry;
            }
            if ry.is_none() {
                ry = rx;
            }
            let (rx, ry) = (rx.unwrap_or(0.0).clamp(0.0, w / 2.0), ry.unwrap_or(0.0).clamp(0.0, hh / 2.0));
            if rx <= 0.0 || ry <= 0.0 {
                vec![(vec![(x, y), (x + w, y), (x + w, y + hh), (x, y + hh)], true)]
            } else {
                // Les coins arrondis : des quarts d'ellipse
                let q = std::f32::consts::FRAC_PI_2;
                let m = segments(q * rx.max(ry), s).max(3);
                let mut v = Vec::new();
                for (cx, cy, a0) in [(x + w - rx, y + ry, -q), (x + w - rx, y + hh - ry, 0.0), (x + rx, y + hh - ry, q), (x + rx, y + ry, 2.0 * q)] {
                    for j in 0..=m {
                        let t = a0 + q * j as f32 / m as f32;
                        v.push((cx + rx * t.cos(), cy + ry * t.sin()));
                    }
                }
                vec![(v, true)]
            }
        }
        "circle" => {
            let r = attr_nb(n, "r", 100.0);
            if r <= 0.0 {
                return;
            }
            vec![(ellipse(attr_nb(n, "cx", 100.0), attr_nb(n, "cy", 100.0), r, r, s), true)]
        }
        "ellipse" => {
            let (rx, ry) = (attr_nb(n, "rx", 100.0), attr_nb(n, "ry", 100.0));
            if rx <= 0.0 || ry <= 0.0 {
                return;
            }
            vec![(ellipse(attr_nb(n, "cx", 100.0), attr_nb(n, "cy", 100.0), rx, ry, s), true)]
        }
        "line" => vec![(vec![(attr_nb(n, "x1", 100.0), attr_nb(n, "y1", 100.0)), (attr_nb(n, "x2", 100.0), attr_nb(n, "y2", 100.0))], false)],
        "polyline" | "polygon" => {
            let v = nombres(n.attr("points").unwrap_or(""));
            let pts: Vec<P> = v.chunks_exact(2).map(|c| (c[0], c[1])).collect();
            if pts.len() < 2 {
                return;
            }
            vec![(pts, n.nom == "polygon")]
        }
        "path" => chemin(n.attr("d").unwrap_or(""), s),
        "text" => {
            // Le texte de l'élément et de ses tspan, sur une ligne
            let mut chaine = String::new();
            let mut pile: Vec<usize> = n.enfants.iter().rev().copied().collect();
            while let Some(e) = pile.pop() {
                let m = &doc.noeuds[e];
                if m.nom.is_empty() {
                    chaine.push_str(&m.texte);
                } else if m.element() {
                    pile.extend(m.enfants.iter().rev());
                }
            }
            let chaine = chaine.split_whitespace().collect::<Vec<_>>().join(" ");
            let fond = resoudre(doc, &h.fond, &h, h.opacite_fond);
            if chaine.is_empty() || !h.visible || fond.is_none() {
                return;
            }
            let x = n.attr("x").map(nombres).and_then(|v| v.first().copied()).unwrap_or(0.0);
            let y = n.attr("y").map(nombres).and_then(|v| v.first().copied()).unwrap_or(0.0);
            let (px, py) = ctm.appliquer((x, y));
            let taille = h.taille * s;
            let large = chaine.chars().count() as f32 * taille * 0.55;
            let x0 = match h.ancre {
                0 => px - large / 2.0,
                1 => px - large,
                _ => px,
            };
            out.push(Forme { noeud: k, contours: Vec::new(), fond, pair_impair: false, trait_: None, epaisseur: 0.0, texte: Some(TexteSvg { chaine, x: px, y: py, taille, ancre: h.ancre }), boite: [x0, py - taille * 0.8, x0 + large, py + taille * 0.25] });
            return;
        }
        _ => {
            // Un élément inconnu : son contenu, comme un groupe
            for e in &n.enfants {
                parcourir(doc, *e, ctm, &h, out, profondeur + 1);
            }
            return;
        }
    };
    if !h.visible || local.is_empty() {
        return;
    }
    let contours: Contours = local.into_iter().map(|(v, f)| (v.into_iter().map(|p| ctm.appliquer(p)).collect(), f)).collect();
    let fond = if n.nom == "line" { None } else { resoudre(doc, &h.fond, &h, h.opacite_fond) };
    let trait_ = resoudre(doc, &h.trait_, &h, h.opacite_trait).filter(|_| h.epaisseur > 0.0);
    if fond.is_none() && trait_.is_none() {
        // Invisible, mais on peut encore le toucher : on le garde sans couleur
    }
    let mut boite = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (v, _) in &contours {
        for p in v {
            boite = [boite[0].min(p.0), boite[1].min(p.1), boite[2].max(p.0), boite[3].max(p.1)];
        }
    }
    out.push(Forme { noeud: k, contours, fond, pair_impair: h.pair_impair, trait_, epaisseur: h.epaisseur * s, texte: None, boite });
}

// --- Toucher, boîtes, balayage ------------------------------------------------------

fn distance_segment(p: P, a: P, b: P) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = dx * dx + dy * dy;
    let t = if l > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l).clamp(0.0, 1.0) } else { 0.0 };
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// L'enroulement des contours autour de `p` (fermés implicitement).
fn enroulement(contours: &Contours, p: P) -> i32 {
    let mut w = 0;
    for (v, _) in contours {
        for i in 0..v.len() {
            let (a, b) = (v[i], v[(i + 1) % v.len()]);
            if a.1 <= p.1 {
                if b.1 > p.1 && (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1) > 0.0 {
                    w += 1;
                }
            } else if b.1 <= p.1 && (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1) < 0.0 {
                w -= 1;
            }
        }
    }
    w
}

impl Forme {
    /// Vrai si `p` touche la forme : dans son remplissage, ou à moins de `tolerance`
    /// de son trait (en unités du document). Une forme sans couleur se touche comme
    /// si elle était remplie.
    pub fn touche(&self, p: P, tolerance: f32) -> bool {
        if let Some(t) = &self.texte {
            let _ = t;
            let b = self.boite;
            return p.0 >= b[0] - tolerance && p.0 <= b[2] + tolerance && p.1 >= b[1] - tolerance && p.1 <= b[3] + tolerance;
        }
        let b = self.boite;
        let marge = self.epaisseur / 2.0 + tolerance;
        if p.0 < b[0] - marge || p.0 > b[2] + marge || p.1 < b[1] - marge || p.1 > b[3] + marge {
            return false;
        }
        let rempli = self.fond.is_some() || self.trait_.is_none();
        if rempli && self.contours.iter().any(|c| c.0.len() > 2) {
            let w = enroulement(&self.contours, p);
            if if self.pair_impair { w % 2 != 0 } else { w != 0 } {
                return true;
            }
        }
        for (v, ferme) in &self.contours {
            let n = v.len();
            let segs = if *ferme { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                if distance_segment(p, v[i], v[(i + 1) % n]) <= marge {
                    return true;
                }
            }
        }
        false
    }
}

/// La boîte de tout ce que dessine l'élément `k` (et ses descendants).
pub fn boite_de(doc: &Document, formes: &[Forme], k: usize) -> Option<[f32; 4]> {
    let mut b: Option<[f32; 4]> = None;
    for f in formes.iter().filter(|f| doc.sous(f.noeud, k)) {
        let e = f.epaisseur / 2.0;
        let fb = [f.boite[0] - e, f.boite[1] - e, f.boite[2] + e, f.boite[3] + e];
        b = Some(match b {
            None => fb,
            Some(x) => [x[0].min(fb[0]), x[1].min(fb[1]), x[2].max(fb[2]), x[3].max(fb[3])],
        });
    }
    b
}

/// L'élément sélectionnable sous `p` : le plus haut dessiné.
pub fn sous(doc: &Document, formes: &[Forme], p: P, tolerance: f32) -> Option<usize> {
    formes.iter().rev().find(|f| f.touche(p, tolerance)).map(|f| doc.selectionnable(f.noeud))
}

/// Balaye des contours (en pixels d'écran) entre les lignes `haut` et `bas` :
/// `bande(y, x0, x1)` pour chaque morceau de ligne rempli, selon la règle
/// `nonzero` ou `evenodd`. Les lignes sont échantillonnées en leur milieu.
pub fn balayer(contours: &[Vec<P>], pair_impair: bool, haut: f32, bas: f32, mut bande: impl FnMut(f32, f32, f32)) {
    // Les arêtes non horizontales : (y0, y1, x à y0, pente dx/dy, sens)
    let mut aretes: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
    for v in contours {
        let n = v.len();
        if n < 3 {
            continue;
        }
        for i in 0..n {
            let (a, b) = (v[i], v[(i + 1) % n]);
            if (a.1 - b.1).abs() < 1e-6 {
                continue;
            }
            let (h, l, sens) = if a.1 < b.1 { (a, b, 1) } else { (b, a, -1) };
            aretes.push((h.1, l.1, h.0, (l.0 - h.0) / (l.1 - h.1), sens));
        }
    }
    if aretes.is_empty() {
        return;
    }
    aretes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let y_min = aretes[0].0.max(haut).floor();
    let y_max = aretes.iter().map(|a| a.1).fold(f32::MIN, f32::max).min(bas).ceil();
    let mut actives: Vec<usize> = Vec::new();
    let mut prochaine = 0;
    let mut croisements: Vec<(f32, i32)> = Vec::new();
    let mut y = y_min;
    while y < y_max {
        let yc = y + 0.5;
        while prochaine < aretes.len() && aretes[prochaine].0 <= yc {
            actives.push(prochaine);
            prochaine += 1;
        }
        actives.retain(|&i| aretes[i].1 > yc);
        croisements.clear();
        for &i in &actives {
            let a = aretes[i];
            if a.0 <= yc {
                croisements.push((a.2 + (yc - a.0) * a.3, a.4));
            }
        }
        croisements.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut w = 0;
        for j in 0..croisements.len() {
            let avant = if pair_impair { w % 2 != 0 } else { w != 0 };
            w += croisements[j].1;
            let _ = avant;
            let dedans = if pair_impair { w % 2 != 0 } else { w != 0 };
            if dedans && j + 1 < croisements.len() {
                let (x0, x1) = (croisements[j].0, croisements[j + 1].0);
                if x1 > x0 {
                    bande(y, x0, x1);
                }
            }
        }
        y += 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXEMPLE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<!-- Un dessin -->
<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100">
  <defs><linearGradient id="dg"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs>
  <g transform="translate(10 20)" fill="green" style="stroke: red; stroke-width: 2">
    <rect id="r" x="0" y="0" width="50" height="30"/>
    <circle cx="100" cy="20" r="10" fill="url(#dg)" stroke="none"/>
  </g>
  <path d="M10 90 h30 v-20 z" fill-rule="evenodd"/>
  <text x="150" y="80" font-size="12" text-anchor="middle">Bonjour &amp; <tspan>salut</tspan></text>
</svg>
"##;

    #[test]
    fn le_xml_se_lit_et_se_reecrit() {
        let d = Document::lire(EXEMPLE).unwrap();
        assert!(d.prologue.contains("<?xml") && d.prologue.contains("Un dessin"));
        let r = &d.noeuds[d.racine];
        assert_eq!(r.attr("width"), Some("200"));
        assert_eq!(d.cadre(), Some([0.0, 0.0, 200.0, 100.0]));
        let t = d.ecrire();
        // Réécrit, il se relit à l'identique
        let e = Document::lire(&t).unwrap();
        assert_eq!(e.ecrire(), t);
        assert!(t.contains("<text x=\"150\" y=\"80\" font-size=\"12\" text-anchor=\"middle\">"), "{t}");
        assert!(t.contains("Bonjour &amp; "));
        // Les erreurs disent où
        assert!(Document::lire("<svg><g></svg>").is_ok(), "fermer la racine ferme le reste");
        assert!(Document::lire("<svg><rect x=1/></svg>").unwrap_err().contains("guillemets"));
        assert!(Document::lire("<html/>").unwrap_err().contains("pas <svg>"));
        assert!(Document::lire("<svg>").unwrap_err().contains("n'est pas fermée"));
    }

    #[test]
    fn couleurs_longueurs_transformations() {
        assert_eq!(couleur("#f80"), Some([255, 136, 0, 255]));
        assert_eq!(couleur("rgb(10, 20, 30)"), Some([10, 20, 30, 255]));
        assert_eq!(couleur("rgba(10,20,30,0.5)"), Some([10, 20, 30, 128]));
        assert_eq!(couleur("Navy"), Some([0, 0, 128, 255]));
        assert_eq!(longueur("10mm", 0.0).map(|v| v.round()), Some(38.0));
        assert_eq!(longueur("50%", 200.0), Some(100.0));
        let m = transformation("translate(10, 5) scale(2)");
        assert_eq!(m.appliquer((1.0, 1.0)), (12.0, 7.0));
        let r = transformation("rotate(90 10 10)");
        let p = r.appliquer((20.0, 10.0));
        assert!((p.0 - 10.0).abs() < 1e-4 && (p.1 - 20.0).abs() < 1e-4);
        let i = m.inverse();
        assert_eq!(i.appliquer((12.0, 7.0)), (1.0, 1.0));
        assert_eq!(Matrice::translation(3.0, 4.5).texte(), "translate(3 4.5)");
        assert_eq!(nombre(2.50001), "2.5");
    }

    #[test]
    fn les_chemins_se_reduisent() {
        let c = chemin("M0 0 L10 0 10 10 Z m 20 0 h5 v5", 1.0);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0], (vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true));
        // Après Z, le point courant revient au départ du sous-chemin
        assert_eq!(c[1].0, vec![(20.0, 0.0), (25.0, 0.0), (25.0, 5.0)]);
        // Courbes : le bout est exact
        let c = chemin("M0 0C0 10 10 10 10 0S20-10 20 0Q25 5 30 0T40 0", 1.0);
        let v = &c[0].0;
        assert_eq!(*v.last().unwrap(), (40.0, 0.0));
        assert!(v.len() > 10);
        // Arcs, avec des drapeaux collés
        let c = chemin("M0 0a10 10 0 1010 0", 1.0);
        let v = &c[0].0;
        assert_eq!(*v.last().unwrap(), (10.0, 0.0));
        let bas = v.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        let haut = v.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        assert!(bas > 5.0 || haut < -5.0, "le grand arc fait un détour");
        // Un chemin mal formé s'arrête à l'erreur
        assert_eq!(chemin("M0 0 L10 0 L x", 1.0)[0].0.len(), 2);
    }

    #[test]
    fn le_rendu_herite_styles_et_transformations() {
        let d = Document::lire(EXEMPLE).unwrap();
        let f = rendu(&d);
        assert_eq!(f.len(), 4);
        let rect = &f[0];
        assert_eq!(rect.fond, Some([0, 128, 0, 255]));
        assert_eq!(rect.trait_, Some([255, 0, 0, 255]));
        assert_eq!(rect.epaisseur, 2.0);
        assert_eq!(rect.boite, [10.0, 20.0, 60.0, 50.0]);
        // Le dégradé prend la moyenne de ses arrêts ; stroke="none" l'emporte
        assert_eq!(f[1].fond, Some([127, 0, 127, 255]));
        assert_eq!(f[1].trait_, None);
        assert!(f[2].pair_impair);
        let t = f[3].texte.as_ref().unwrap();
        assert_eq!((t.chaine.as_str(), t.ancre), ("Bonjour & salut", 0));
        // Toucher : le rectangle, puis rien
        assert_eq!(sous(&d, &f, (30.0, 30.0), 1.0), d.par_id("r").map(|r| d.selectionnable(r)));
        assert_eq!(sous(&d, &f, (190.0, 10.0), 1.0), None);
        let g = d.noeuds[d.racine].enfants[1];
        assert_eq!(boite_de(&d, &f, g).map(|b| b[0]), Some(9.0));
    }

    #[test]
    fn le_balayage_respecte_la_regle() {
        // Un carré de 10 avec un trou de 4 au milieu, dans le même sens
        let ext = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let int = vec![(3.0, 3.0), (7.0, 3.0), (7.0, 7.0), (3.0, 7.0)];
        let surface = |pair: bool| {
            let mut s = 0.0;
            balayer(&[ext.clone(), int.clone()], pair, -100.0, 100.0, |_, a, b| s += b - a);
            s
        };
        assert_eq!(surface(false), 100.0);
        assert_eq!(surface(true), 84.0);
        // Coupé par la vue
        let mut lignes = 0;
        balayer(&[ext.clone()], false, 2.0, 5.0, |_, _, _| lignes += 1);
        assert_eq!(lignes, 3);
    }

    #[test]
    fn les_retouches() {
        let mut d = Document::lire(EXEMPLE).unwrap();
        let r = d.par_id("r").unwrap();
        // Une propriété du style passe en attribut
        let g = d.noeuds[r].parent.unwrap();
        d.definir(g, "stroke", Some("blue"));
        assert_eq!(d.noeuds[g].attr("style"), Some("stroke-width: 2"));
        assert_eq!(d.noeuds[g].attr("stroke"), Some("blue"));
        // Déplacer le rectangle de 5 dans le document, sous son groupe translaté
        let avant = d.noeuds[r].attr("transform").unwrap_or("").to_string();
        d.transformer(r, &Matrice::translation(5.0, 0.0), &avant);
        assert_eq!(d.noeuds[r].attr("transform"), Some("translate(5 0)"));
        assert_eq!(rendu(&d)[0].boite[0], 15.0);
        // Dupliquer : juste après, sans id
        let c = d.dupliquer(r).unwrap();
        assert_eq!(d.noeuds[g].enfants.iter().position(|e| *e == c), d.noeuds[g].enfants.iter().position(|e| *e == r).map(|i| i + 1));
        assert_eq!(d.noeuds[c].attr("id"), None);
        // Plans : descendre la copie la remet sous l'original
        assert!(d.changer_plan(c, false));
        assert!(!d.changer_plan(c, false));
        // Retirer
        d.retirer(c);
        assert!(!d.ecrire().contains("transform=\"translate(5 0)\"/>\n    <rect"));
        let n = d.ajouter(d.racine, "circle", vec![("r".into(), "3".into())]);
        assert_eq!(d.selectionnable(n), n);
    }

    #[test]
    fn les_traces_se_lisent_et_s_ecrivent() {
        let t = lire_trace("m10 20 h30 v10 s5 5 10 0 t10 0 a5 5 0 0 1 10 0 z l5 5");
        assert_eq!(t[0], Morceau::Depart((10.0, 20.0)));
        assert_eq!(t[1], Morceau::Droite((40.0, 20.0)));
        assert_eq!(t[2], Morceau::Droite((40.0, 30.0)));
        // S sans C avant : le premier contrôle est le point courant
        assert_eq!(t[3], Morceau::Cubique((40.0, 30.0), (45.0, 35.0), (50.0, 30.0)));
        assert_eq!(t[4], Morceau::Quadratique((50.0, 30.0), (60.0, 30.0)));
        assert!(matches!(t[5], Morceau::Arc { fin: (70.0, 30.0), sens: true, .. }));
        assert_eq!(t[6], Morceau::Fermer);
        // Après Z, le segment repart du départ
        assert_eq!(&t[7..], &[Morceau::Depart((10.0, 20.0)), Morceau::Droite((15.0, 25.0))]);
        let d = ecrire_trace(&t);
        assert_eq!(lire_trace(&d), t);
        assert!(d.starts_with("M10 20 L40 20 L40 30 C40 30 45 35 50 30 Q50 30 60 30 A5 5 0 0 1 70 30 Z M10 20"));
        // Le rendu est le même avant et après réécriture
        let a = chemin("m10 20 h30 v10 s5 5 10 0 t10 0 a5 5 0 0 1 10 0 z", 1.0);
        let b = chemin(&ecrire_trace(&lire_trace("m10 20 h30 v10 s5 5 10 0 t10 0 a5 5 0 0 1 10 0 z")), 1.0);
        assert_eq!(a.len(), b.len());
        for (x, y) in a[0].0.iter().zip(&b[0].0) {
            assert!(dist(*x, *y) < 1e-3);
        }
    }

    #[test]
    fn les_formes_deviennent_des_traces() {
        let d = Document::lire(r#"<svg><rect x="0" y="0" width="10" height="20"/><circle cx="5" cy="5" r="5"/><polygon points="0,0 10,0 5,8"/><line x1="1" y1="2" x2="3" y2="4"/><rect width="10" height="10" rx="2"/></svg>"#).unwrap();
        let e: Vec<usize> = d.noeuds[d.racine].enfants.clone();
        assert_eq!(trace_de(&d.noeuds[e[0]]).unwrap().len(), 5);
        let c = trace_de(&d.noeuds[e[1]]).unwrap();
        assert_eq!((c.len(), c[0], c[4].fin()), (6, Morceau::Depart((10.0, 5.0)), Some((10.0, 5.0))));
        // Le jumeau (le dernier sommet du cercle) ne se montre pas
        assert_eq!(reperes(&c).iter().filter(|r| matches!(r.0, Repere::Sommet(_))).count(), 4);
        assert_eq!(trace_de(&d.noeuds[e[2]]).unwrap().last(), Some(&Morceau::Fermer));
        assert_eq!(trace_de(&d.noeuds[e[3]]).unwrap()[1], Morceau::Droite((3.0, 4.0)));
        assert_eq!(trace_de(&d.noeuds[e[4]]).unwrap().len(), 10);
        // Un polygone reste polygone ; une courbe en fait un chemin
        let mut d = d;
        let mut t = trace_de(&d.noeuds[e[2]]).unwrap();
        deplacer(&mut t, Repere::Sommet(2), (5.0, 10.0));
        d.poser_trace(e[2], &t);
        assert_eq!(d.noeuds[e[2]].attr("points"), Some("0,0 10,0 5,10"));
        basculer(&mut t, 1);
        d.poser_trace(e[2], &t);
        assert_eq!(d.noeuds[e[2]].nom, "path");
        assert_eq!(d.noeuds[e[2]].attr("points"), None);
        assert!(d.noeuds[e[2]].attr("d").unwrap().contains('C'));
        // Le rectangle devenu chemin perd sa géométrie
        let mut t = trace_de(&d.noeuds[e[0]]).unwrap();
        deplacer(&mut t, Repere::Sommet(2), (12.0, 22.0));
        d.poser_trace(e[0], &t);
        assert_eq!(d.noeuds[e[0]].attrs, vec![("d".to_string(), "M0 0 L10 0 L12 22 L0 20 Z".to_string())]);
    }

    /// L'écart le plus grand entre des points et un tracé (mis en points fins).
    fn ecart_au_trace(points: &[P], t: &[Morceau]) -> f32 {
        let c = chemin(&ecrire_trace(t), 100.0);
        points.iter().map(|p| c.iter().flat_map(|l| l.0.windows(2)).map(|w| distance_segment(*p, w[0], w[1])).fold(f32::MAX, f32::min)).fold(0.0, f32::max)
    }

    #[test]
    fn simplifier_sans_perte() {
        let s = |d: &str| ecrire_trace(&simplifier(&lire_trace(d), Simplification::SANS_PERTE));
        // Les sommets alignés et les doublons s'en vont, un aller-retour reste
        assert_eq!(s("M0 0 L5 0 L10 0 L10 0 L10 10"), "M0 0 L10 0 L10 10");
        assert_eq!(s("M0 0 L10 0 L5 0"), "M0 0 L10 0 L5 0");
        // Un point (trait rond) garde son segment nul
        assert_eq!(s("M3 3 L3 3"), "M3 3 L3 3");
        // Une courbe droite devient une droite, une vraie courbe reste
        assert_eq!(s("M0 0 C2 0 8 0 10 0 L20 0"), "M0 0 L20 0");
        assert_eq!(s("M0 0 C0 5 10 5 10 0"), "M0 0 C0 5 10 5 10 0");
        // Fermé : le retour au départ est dit par Z ; les sous-chemins restent séparés
        assert_eq!(s("M0 0 L10 0 L10 10 L0 0 Z M20 20 L25 20 L30 20"), "M0 0 L10 0 L10 10 Z M20 20 L30 20");
        // Un arc reste
        assert_eq!(s("M0 0 L1 0 L2 0 A5 5 0 0 1 12 0"), "M0 0 L2 0 A5 5 0 0 1 12 0");
    }

    #[test]
    fn simplifier_avec_perte() {
        // Un cercle tremblé de 400 points
        let points: Vec<P> = (0..=400)
            .map(|k| {
                let a = k as f32 / 400.0 * std::f32::consts::TAU;
                let r = 50.0 + 0.2 * (k as f32 * 1.7).sin();
                (r * a.cos(), r * a.sin())
            })
            .collect();
        let t: Vec<Morceau> = points.iter().enumerate().map(|(k, p)| if k == 0 { Morceau::Depart(*p) } else { Morceau::Droite(*p) }).collect();
        let droites = simplifier(&t, Simplification { tolerance: 0.5, ajustement: Ajustement::Droites });
        assert!(sommets(&droites) < 60, "{}", sommets(&droites));
        assert!(droites.iter().skip(1).all(|m| matches!(m, Morceau::Droite(_))));
        assert!(ecart_au_trace(&points, &droites) <= 0.51);
        let lisse = simplifier(&t, Simplification { tolerance: 0.5, ajustement: Ajustement::Rapide });
        assert!(sommets(&lisse) <= 16, "{}", sommets(&lisse));
        assert!(lisse.iter().skip(1).all(|m| matches!(m, Morceau::Cubique(..))));
        assert!(ecart_au_trace(&points, &lisse) <= 0.6, "{}", ecart_au_trace(&points, &lisse));
        // Un carré finement échantillonné garde ses angles et ses côtés droits
        let mut carre = vec![Morceau::Depart((0.0, 0.0))];
        for (a, b) in [((0.0, 0.0), (10.0, 0.0)), ((10.0, 0.0), (10.0, 10.0)), ((10.0, 10.0), (0.0, 10.0))] {
            carre.extend((1..=20).map(|k| Morceau::Droite(milieu(a, b, k as f32 / 20.0))));
        }
        for aj in [Ajustement::Rapide, Ajustement::Optimal, Ajustement::Spline] {
            assert_eq!(ecrire_trace(&simplifier(&carre, Simplification { tolerance: 0.3, ajustement: aj })), "M0 0 L10 0 L10 10 L0 10", "{aj:?}");
        }
        // Une suite de courbes se refond
        let c = lire_trace("M0 0 C0 5 5 10 10 10 C15 10 20 5 20 0 C20 -5 25 -10 30 -10");
        let r = simplifier(&c, Simplification { tolerance: 0.2, ajustement: Ajustement::Rapide });
        assert!(sommets(&r) <= sommets(&c));
        let fins: Vec<P> = (0..=60).map(|k| point_sur(&c, 1 + (k * 3 / 61), (k * 3 % 61) as f32 / 61.0)).collect();
        assert!(ecart_au_trace(&fins, &r) <= 0.3);
    }

    #[test]
    fn optimal_et_spline() {
        // Une tache tremblée de 241 points (comme un tracé au doigt), fermée
        let mut pts = Vec::new();
        for k in 0..=240 {
            let a = k as f32 / 240.0 * std::f32::consts::TAU;
            let r = 70.0 + 12.0 * (3.0 * a).sin() + 0.6 * (k as f32 * 2.3).sin();
            pts.push((200.0 + r * a.cos(), 150.0 + r * a.sin()));
        }
        let t: Vec<Morceau> = pts.iter().enumerate().map(|(k, p)| if k == 0 { Morceau::Depart(*p) } else { Morceau::Droite(*p) }).collect();
        let s = |aj| simplifier(&t, Simplification { tolerance: 1.0, ajustement: aj });
        let (rapide, optimal, spline) = (s(Ajustement::Rapide), s(Ajustement::Optimal), s(Ajustement::Spline));
        // Fidèles (le lissage enlève le tremblé de ±0,6), et pas plus de sommets que la méthode rapide
        for (nom, r) in [("rapide", &rapide), ("optimal", &optimal), ("spline", &spline)] {
            let e = ecart_au_trace(&pts, r);
            assert!(e <= 1.7, "{nom} : écart {e}");
            assert!(r[1..].iter().all(|m| matches!(m, Morceau::Cubique(..))), "{nom}");
        }
        assert!(sommets(&optimal) <= sommets(&rapide), "{} > {}", sommets(&optimal), sommets(&rapide));
        assert!(sommets(&spline) <= 16, "{}", sommets(&spline));
        // La spline : aux sommets intérieurs, tangente et courbure continues
        for w in spline[1..].windows(2) {
            let (Morceau::Cubique(_, b2, p), Morceau::Cubique(c1, c2, _)) = (w[0], w[1]) else { panic!() };
            let (u, v) = (unitaire((p.0 - b2.0, p.1 - b2.1)), unitaire((c1.0 - p.0, c1.1 - p.1)));
            assert!(u.0 * v.0 + u.1 * v.1 > 0.9998, "tangente {u:?} {v:?}");
            let _ = c2;
        }
        // Une droite bruitée reste une droite, quelle que soit la méthode
        let d: Vec<Morceau> = (0..50).map(|k| { let p = (k as f32, 0.05 * (k as f32 * 1.3).sin()); if k == 0 { Morceau::Depart(p) } else { Morceau::Droite(p) } }).collect();
        for aj in Ajustement::TOUS {
            assert_eq!(sommets(&simplifier(&d, Simplification { tolerance: 0.5, ajustement: aj })), 2, "{aj:?}");
        }
    }

    #[test]
    fn simplifier_un_document() {
        let mut d = Document::lire(r#"<svg><g transform="scale(10)"><polyline points="0 0 1 0.05 2 0 3 0"/></g><rect width="5" height="5"/><path d="M0 0 L1 0.02 L2 0"/></svg>"#).unwrap();
        let r = d.racine;
        // 0,2 de tolérance : l'écart de 0,02 retiré dehors, gardé dans le groupe agrandi
        assert_eq!(d.simplifier(r, Simplification { tolerance: 0.1, ajustement: Ajustement::Droites }), (7, 6));
        let e = d.ecrire();
        assert!(e.contains("points=\"0 0 1 0.05 2 0 3 0\""), "{e}");
        assert!(e.contains("d=\"M0 0 L2 0\""), "{e}");
        assert!(e.contains("<rect"), "{e}");
    }

    #[test]
    fn retoucher_les_sommets() {
        // Déplacer le départ d'un cercle emmène son jumeau et les contrôles voisins
        let n = Noeud { nom: "circle".into(), attrs: vec![("cx".into(), "0".into()), ("cy".into(), "0".into()), ("r".into(), "10".into())], ..Noeud::default() };
        let mut c = trace_de(&n).unwrap();
        deplacer(&mut c, Repere::Sommet(0), (12.0, 0.0));
        assert_eq!(c[4].fin(), Some((12.0, 0.0)));
        assert!(matches!(c[1], Morceau::Cubique((x, _), _, _) if (x - 12.0).abs() < 1e-4));
        assert!(matches!(c[4], Morceau::Cubique(_, (x, _), _) if (x - 12.0).abs() < 1e-4));
        // Couper une courbe garde la forme
        let avant = point_sur(&c, 1, 0.75);
        let k = couper(&mut c, 1, 0.5).unwrap();
        assert_eq!(k, 1);
        assert_eq!(c.len(), 7);
        assert!(dist(point_sur(&c, 2, 0.5), avant) < 1e-3);
        // La retirer refond les deux courbes
        assert!(retirer_sommet(&mut c, 1));
        assert_eq!(c.len(), 6);
        assert!(matches!(c[1], Morceau::Cubique(_, _, p) if dist(p, (0.0, 10.0)) < 1e-3));
        // Retirer le départ : le deuxième sommet devient départ, le jumeau le rejoint
        assert!(retirer_sommet(&mut c, 0));
        assert_eq!(c[0].fin(), Some((0.0, 10.0)));
        let (d, f) = sous_chemin(&c, 0);
        assert!(jumeau(&c, d, f).is_some());
        assert_eq!(reperes(&c).iter().filter(|r| matches!(r.0, Repere::Sommet(_))).count(), 3);
        // Un polygone : couper le Fermer, basculer, ouvrir
        let mut t = lire_trace("M0 0 L10 0 L10 10 Z");
        assert_eq!(segment_proche(&t, &Matrice::IDENTITE, (5.0, 5.2), 1.0).map(|s| s.0), Some(3));
        assert_eq!(couper(&mut t, 3, 0.5), Some(3));
        assert_eq!(t[3], Morceau::Droite((5.0, 5.0)));
        assert_eq!(droit(&t, 0), Some(true));
        basculer(&mut t, 0);
        assert!(matches!(t[4], Morceau::Cubique(_, _, (0.0, 0.0))));
        assert_eq!(droit(&t, 0), Some(false));
        basculer(&mut t, 0);
        assert_eq!(ecrire_trace(&t), "M0 0 L10 0 L10 10 L5 5 Z");
        ouvrir_fermer(&mut t, 2);
        assert!(!est_ferme(&t, 0));
        assert_eq!(droit(&t, 0), None);
        // Trop peu de sommets : refusé s'il ne reste rien
        let mut l = lire_trace("M0 0 L1 1");
        assert!(!retirer_sommet(&mut l, 1));
        let mut l = lire_trace("M0 0 L1 1 M5 5 L6 6 L7 5");
        assert!(retirer_sommet(&mut l, 1));
        assert_eq!(ecrire_trace(&l), "M5 5 L6 6 L7 5");
        // Un arc coupé devient courbes
        let mut a = lire_trace("M0 0 A10 10 0 0 1 20 0");
        let m = point_sur(&a, 1, 0.5);
        assert!(dist(m, (10.0, -10.0)) < 0.05, "{m:?}");
        couper(&mut a, 1, 0.5).unwrap();
        assert!(a.iter().skip(1).all(|m| matches!(m, Morceau::Cubique(..))));
        assert!(a.iter().any(|m| m.fin().is_some_and(|p| dist(p, (10.0, -10.0)) < 0.05)));
    }

}
