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

/// Un arc elliptique de `a` à `b` (paramètres SVG), ajouté à `cour` sans son départ.
#[allow(clippy::too_many_arguments)]
fn arc(cour: &mut Vec<P>, a: P, b: P, rx: f32, ry: f32, rot: f32, grand: bool, sens: bool, s: f32) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-6 || ry < 1e-6 || dist(a, b) < 1e-6 {
        cour.push(b);
        return;
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

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, s: f32) -> Vec<P> {
    let n = segments(std::f32::consts::TAU * rx.max(ry), s).max(12);
    (0..n)
        .map(|j| {
            let t = std::f32::consts::TAU * j as f32 / n as f32;
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
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
}
