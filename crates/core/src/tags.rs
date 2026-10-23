//! Tagged PDF (Phase 9): Strukturbaum lesen, Dokumenttitel/-sprache und Alternativtexte ändern.
//!
//! Ein getaggtes PDF enthält neben dem Aussehen eine logische Struktur (`/StructTreeRoot`):
//! Elemente wie Überschrift (`H1`), Absatz (`P`) oder Abbildung (`Figure`). Sie verweisen über
//! Markierungs-Nummern (MCID) auf Inhalte einer Seite. Screenreader lesen diese Struktur in
//! Baumreihenfolge vor; Bilder brauchen einen Alternativtext (`/Alt`).
//!
//! Lesen ist rein: [`Document::structure_tree`]. Ändern läuft über Commands (Undo): Titel und
//! Sprache in `Model::properties`, Alternativtexte in `Model::alt_texts`. Beim Speichern
//! schreibt `save` die Änderungen (Katalog /Lang, /Info /Title, /ViewerPreferences
//! /DisplayDocTitle, /Alt am Strukturelement).

use crate::command::{Command, Model};
use crate::document::Document;
use crate::error::Result;
use crate::pdf_string;
use lopdf::{Dictionary, Object, ObjectId};
use std::any::Any;
use std::collections::HashMap;

/// Titel und Sprache des Dokuments (leer = nicht gesetzt).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocProperties {
    pub title: String,
    /// Sprachkennung nach BCP 47, z. B. "de-DE".
    pub lang: String,
}

/// Ein Strukturelement.
#[derive(Debug, Clone, PartialEq)]
pub struct StructNode {
    /// Objektnummer (für Änderungen wie den Alternativtext).
    pub id: Option<ObjectId>,
    /// Rolle nach der RoleMap aufgelöst (z. B. eigene Rolle "Ueberschrift" -> "H1").
    pub role: String,
    /// Rolle wie in der Datei.
    pub raw_role: String,
    pub alt: Option<String>,
    pub actual_text: Option<String>,
    pub lang: Option<String>,
    /// Markierte Inhalte: (Seitenindex, MCID).
    pub content: Vec<(usize, i64)>,
    pub children: Vec<StructNode>,
}

impl StructNode {
    /// Alle Knoten in Lesereihenfolge (Tiefensuche), mit Tiefe.
    pub fn walk<'a>(&'a self, depth: usize, out: &mut Vec<(usize, &'a StructNode)>) {
        out.push((depth, self));
        for c in &self.children {
            c.walk(depth + 1, out);
        }
    }

    pub fn is_figure(&self) -> bool {
        self.role == "Figure"
    }
}

/// Der Strukturbaum eines Dokuments.
#[derive(Debug, Clone, PartialEq)]
pub struct StructTree {
    pub roots: Vec<StructNode>,
}

impl StructTree {
    pub fn nodes(&self) -> Vec<(usize, &StructNode)> {
        let mut out = Vec::new();
        for r in &self.roots {
            r.walk(0, &mut out);
        }
        out
    }
}

struct Reader<'a> {
    pdf: &'a lopdf::Document,
    role_map: HashMap<Vec<u8>, Vec<u8>>,
    /// Seitenobjekt -> Index.
    pages: HashMap<ObjectId, usize>,
    alt_overrides: &'a std::collections::BTreeMap<ObjectId, Option<String>>,
}

impl Reader<'_> {
    fn deref<'b>(&'b self, o: &'b Object) -> &'b Object {
        self.pdf.dereference(o).map(|(_, o)| o).unwrap_or(o)
    }

    /// Rolle über die RoleMap auf eine Standardrolle zurückführen (begrenzt gegen Schleifen).
    fn standard_role(&self, raw: &[u8]) -> Vec<u8> {
        let mut role = raw.to_vec();
        for _ in 0..8 {
            match self.role_map.get(&role) {
                Some(next) if *next != role => role = next.clone(),
                _ => break,
            }
        }
        role
    }

    fn node(
        &self,
        id: Option<ObjectId>,
        d: &Dictionary,
        page: Option<usize>,
        depth: usize,
    ) -> StructNode {
        let raw = d
            .get(b"S")
            .and_then(Object::as_name)
            .unwrap_or_default()
            .to_vec();
        let page = d
            .get(b"Pg")
            .and_then(Object::as_reference)
            .ok()
            .and_then(|p| self.pages.get(&p).copied())
            .or(page);
        let text = |k: &[u8]| {
            d.get(k).ok().and_then(|o| match self.deref(o) {
                Object::String(b, _) => Some(pdf_string::decode(b)),
                _ => None,
            })
        };
        let alt = match id.and_then(|id| self.alt_overrides.get(&id)) {
            Some(over) => over.clone(),
            None => text(b"Alt"),
        };
        let mut node = StructNode {
            id,
            role: String::from_utf8_lossy(&self.standard_role(&raw)).into_owned(),
            raw_role: String::from_utf8_lossy(&raw).into_owned(),
            alt,
            actual_text: text(b"ActualText"),
            lang: text(b"Lang"),
            content: Vec::new(),
            children: Vec::new(),
        };
        if depth < 64
            && let Ok(k) = d.get(b"K")
        {
            self.kids(k, page, depth, &mut node);
        }
        node
    }

    /// /K: Zahl (MCID), Dictionary (Struktur-Element, MCR oder OBJR), Verweis oder Array davon.
    fn kids(&self, k: &Object, page: Option<usize>, depth: usize, node: &mut StructNode) {
        match k {
            Object::Integer(mcid) => {
                if let Some(p) = page {
                    node.content.push((p, *mcid));
                }
            }
            Object::Array(a) => {
                for k in a {
                    self.kids(k, page, depth, node);
                }
            }
            Object::Reference(id) => {
                if let Ok(d) = self.pdf.get_dictionary(*id) {
                    self.kid_dict(Some(*id), d, page, depth, node);
                }
            }
            Object::Dictionary(d) => self.kid_dict(None, d, page, depth, node),
            _ => {}
        }
    }

    fn kid_dict(
        &self,
        id: Option<ObjectId>,
        d: &Dictionary,
        page: Option<usize>,
        depth: usize,
        node: &mut StructNode,
    ) {
        match d.get(b"Type").and_then(Object::as_name).ok() {
            // Marked-content reference: MCID auf (evtl. anderer) Seite.
            Some(b"MCR") => {
                let page = d
                    .get(b"Pg")
                    .and_then(Object::as_reference)
                    .ok()
                    .and_then(|p| self.pages.get(&p).copied())
                    .or(page);
                if let (Some(p), Ok(mcid)) = (page, d.get(b"MCID").and_then(Object::as_i64)) {
                    node.content.push((p, mcid));
                }
            }
            // Verweis auf ein Objekt (z. B. Annotation) – nicht als Inhalt geführt.
            Some(b"OBJR") => {}
            _ => node.children.push(self.node(id, d, page, depth + 1)),
        }
    }
}

impl Document {
    /// Hat das Dokument einen Strukturbaum (und ist als getaggt markiert)?
    pub fn is_tagged(&self) -> bool {
        let Ok(cat) = self.pdf().catalog() else {
            return false;
        };
        let marked = cat
            .get(b"MarkInfo")
            .ok()
            .and_then(|o| self.pdf().dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|d| d.get(b"Marked").and_then(Object::as_bool).ok())
            .unwrap_or(false);
        marked && cat.has(b"StructTreeRoot")
    }

    /// Liest den Strukturbaum (`None`, wenn das Dokument nicht getaggt ist).
    pub fn structure_tree(&self) -> Option<StructTree> {
        let pdf = self.pdf();
        let cat = pdf.catalog().ok()?;
        let root = pdf
            .dereference(cat.get(b"StructTreeRoot").ok()?)
            .ok()?
            .1
            .as_dict()
            .ok()?;
        let role_map = root
            .get(b"RoleMap")
            .ok()
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_name().ok()?.to_vec())))
                    .collect()
            })
            .unwrap_or_default();
        let reader = Reader {
            pdf,
            role_map,
            pages: self
                .pages()
                .iter()
                .enumerate()
                .map(|(i, p)| (p.object_id, i))
                .collect(),
            alt_overrides: &self.model().alt_texts,
        };
        let mut top = StructNode {
            id: None,
            role: String::new(),
            raw_role: String::new(),
            alt: None,
            actual_text: None,
            lang: None,
            content: Vec::new(),
            children: Vec::new(),
        };
        if let Ok(k) = root.get(b"K") {
            reader.kids(k, None, 0, &mut top);
        }
        Some(StructTree {
            roots: top.children,
        })
    }

    /// Markierte Inhalte einer Seite (Text und Rechteck je MCID).
    pub fn marked_content(
        &self,
        page_idx: usize,
    ) -> Result<std::collections::BTreeMap<i64, crate::content::MarkedContent>> {
        let pc = self.page_content(page_idx)?;
        let page = self.page(page_idx)?.object_id;
        Ok(self.interpret_content(page, &pc)?.marked)
    }

    /// Titel und Sprache, wie sie gerade gelten (inkl. ungespeicherter Änderungen).
    pub fn properties(&self) -> &DocProperties {
        &self.model().properties
    }
}

/// Titel und Sprache aus der Datei lesen.
pub(crate) fn read_properties(pdf: &lopdf::Document) -> DocProperties {
    let title = pdf
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| pdf.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
        .map(|d| pdf_string::get(d, b"Title"))
        .unwrap_or_default();
    let lang = pdf
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Lang").ok())
        .and_then(|o| match pdf.dereference(o).ok()?.1 {
            Object::String(b, _) => Some(pdf_string::decode(b)),
            _ => None,
        })
        .unwrap_or_default();
    DocProperties { title, lang }
}

// ---------- Commands ----------

/// Titel und Sprache ändern.
#[derive(Debug)]
pub struct SetProperties {
    before: DocProperties,
    after: DocProperties,
}

impl SetProperties {
    pub fn new(before: DocProperties, after: DocProperties) -> Self {
        Self { before, after }
    }
}

impl Command for SetProperties {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        model.properties = self.after.clone();
        Ok(())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        model.properties = self.before.clone();
        Ok(())
    }
    fn label(&self) -> String {
        "Titel/Sprache ändern".into()
    }
}

/// Alternativtext eines Strukturelements setzen oder entfernen.
#[derive(Debug)]
pub struct SetAltText {
    elem: ObjectId,
    /// Vorheriger Eintrag in `Model::alt_texts` (äußeres `None` = keiner).
    before: Option<Option<String>>,
    after: Option<String>,
}

impl SetAltText {
    pub fn new(elem: ObjectId, alt: Option<String>) -> Self {
        Self {
            elem,
            before: None,
            after: alt.filter(|a| !a.trim().is_empty()),
        }
    }
}

impl Command for SetAltText {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        self.before = model.alt_texts.insert(self.elem, self.after.clone());
        Ok(())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        match self.before.take() {
            Some(b) => model.alt_texts.insert(self.elem, b),
            None => model.alt_texts.remove(&self.elem),
        };
        Ok(())
    }
    fn label(&self) -> String {
        "Alternativtext ändern".into()
    }
}

/// Schreibt Titel/Sprache und Alternativtexte (Teil von `save::incremental`).
pub(crate) fn write(doc: &Document, inc: &mut lopdf::IncrementalDocument) -> Result<()> {
    let pdf = doc.pdf();
    let props = &doc.model().properties;
    if *props != doc.saved_properties() {
        // Katalog: /Lang und /ViewerPreferences /DisplayDocTitle.
        let cat_id = pdf
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|_| crate::error::CoreError::Other("Katalog fehlt".into()))?;
        let prefs = pdf
            .catalog()?
            .get(b"ViewerPreferences")
            .ok()
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok().cloned())
            .unwrap_or_default();
        inc.opt_clone_object_to_new_document(cat_id)?;
        let cat = inc
            .new_document
            .get_object_mut(cat_id)
            .and_then(Object::as_dict_mut)?;
        if props.lang.is_empty() {
            cat.remove(b"Lang");
        } else {
            cat.set("Lang", pdf_string::encode(&props.lang));
        }
        let mut prefs = prefs;
        prefs.set("DisplayDocTitle", !props.title.is_empty());
        cat.set("ViewerPreferences", prefs);
        // /Info: Titel (neues oder geändertes Info-Dictionary).
        let info_ref = pdf.trailer.get(b"Info").and_then(Object::as_reference).ok();
        let mut info = info_ref
            .and_then(|id| pdf.get_dictionary(id).ok().cloned())
            .unwrap_or_default();
        if props.title.is_empty() {
            info.remove(b"Title");
        } else {
            info.set("Title", pdf_string::encode(&props.title));
        }
        info.set(
            "ModDate",
            Object::string_literal(crate::date::pdf_date_now()),
        );
        match info_ref {
            Some(id) => {
                inc.new_document.set_object(id, info);
            }
            None => {
                let id = inc.new_document.add_object(info);
                inc.new_document.trailer.set("Info", id);
            }
        }
    }
    for (&elem, alt) in &doc.model().alt_texts {
        if doc.saved_alt(elem) == Some(alt) {
            continue;
        }
        if inc.opt_clone_object_to_new_document(elem).is_err() {
            continue;
        }
        if let Ok(d) = inc
            .new_document
            .get_object_mut(elem)
            .and_then(Object::as_dict_mut)
        {
            match alt {
                Some(a) => d.set("Alt", pdf_string::encode(a)),
                None => {
                    d.remove(b"Alt");
                }
            }
        }
    }
    Ok(())
}
