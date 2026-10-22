//! Dokument bereinigen: versteckte oder vergessene Informationen entfernen.
//!
//! Zuerst [`inspect`] (was ist drin?), dann [`sanitize_pdf`] mit den gewählten Optionen.
//! Danach wird die Datei komplett neu geschrieben – damit verschwinden auch alte Revisionen
//! und Objekte, auf die nichts mehr verweist.

use crate::content::{page_content_bytes, page_resources};
use crate::error::Result;
use crate::redact::rewrite;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream};
use std::collections::HashSet;

/// Was entfernt werden soll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SanitizeOptions {
    pub metadata: bool,
    pub javascript: bool,
    pub embedded_files: bool,
    pub hidden_layers: bool,
    pub comments: bool,
    pub form_data: bool,
}

impl Default for SanitizeOptions {
    fn default() -> Self {
        Self {
            metadata: true,
            javascript: true,
            embedded_files: true,
            hidden_layers: true,
            comments: false,
            form_data: false,
        }
    }
}

/// Was im Dokument gefunden wurde (Anzahlen).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Findings {
    pub metadata: usize,
    pub javascript: usize,
    pub embedded_files: usize,
    pub hidden_layers: usize,
    pub comments: usize,
    pub form_values: usize,
}

/// Untersucht das Dokument, ohne etwas zu ändern.
pub fn inspect(bytes: &[u8]) -> Result<Findings> {
    let pdf = lopdf::Document::load_mem(bytes)?;
    let mut f = Findings::default();
    let catalog = pdf.catalog().ok();
    if pdf.trailer.get(b"Info").is_ok() {
        f.metadata += 1;
    }
    for obj in pdf.objects.values() {
        let dict = match obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &s.dict,
            _ => continue,
        };
        if dict.has(b"Metadata") || dict.has(b"PieceInfo") {
            f.metadata += 1;
        }
        if is_js_action(&pdf, dict) {
            f.javascript += 1;
        }
        if dict.get(b"Type").and_then(Object::as_name).ok() == Some(b"EmbeddedFile") {
            f.embedded_files += 1;
        }
        if dict.has(b"FT") && dict.has(b"V") {
            f.form_values += 1;
        }
    }
    if let Some(c) = catalog
        && let Ok(names) = deref_dict(&pdf, c.get(b"Names").ok())
        && names.has(b"JavaScript")
    {
        f.javascript += 1;
    }
    // Kommentare: alle Annotationen der Seiten außer Formularfeldern, Links und Popups.
    for page in pdf.get_pages().into_values() {
        let annots = pdf
            .get_dictionary(page)
            .ok()
            .and_then(|d| d.get(b"Annots").ok())
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok());
        for a in annots.into_iter().flatten() {
            let sub = a
                .as_reference()
                .ok()
                .and_then(|id| pdf.get_dictionary(id).ok())
                .and_then(|d| d.get(b"Subtype").ok())
                .and_then(|o| o.as_name().ok())
                .unwrap_or_default();
            if !matches!(sub, b"Widget" | b"Link" | b"Popup") {
                f.comments += 1;
            }
        }
    }
    f.hidden_layers = hidden_ocgs(&pdf).len();
    Ok(f)
}

/// Bereinigt und schreibt die Datei komplett neu. Liefert die Bytes und eine Beschreibung.
pub fn sanitize_pdf(bytes: &[u8], opts: SanitizeOptions) -> Result<(Vec<u8>, Vec<String>)> {
    let mut pdf = lopdf::Document::load_mem(bytes)?;
    let mut done = Vec::new();
    let found = inspect(bytes)?;

    if opts.hidden_layers {
        let n = remove_hidden_layers(&mut pdf)?;
        if n > 0 {
            done.push(format!("{n} ausgeblendete Ebene(n) samt Inhalt entfernt"));
        }
    }
    if opts.metadata {
        pdf.trailer.remove(b"Info");
        for obj in pdf.objects.values_mut() {
            if let Some(d) = dict_mut(obj) {
                d.remove(b"Metadata");
                d.remove(b"PieceInfo");
            }
        }
        if found.metadata > 0 {
            done.push("Metadaten (Dokumentinfo, XMP, Programmdaten) entfernt".into());
        }
    }
    if opts.javascript {
        let root = root_id(&pdf);
        if let Some(root) = root
            && let Ok(c) = pdf.get_object_mut(root).and_then(Object::as_dict_mut)
        {
            c.remove(b"OpenAction");
            c.remove(b"AA");
        }
        remove_name_tree(&mut pdf, b"JavaScript");
        let ids: Vec<ObjectId> = pdf.objects.keys().copied().collect();
        for id in ids {
            let is_js = pdf
                .objects
                .get(&id)
                .and_then(|o| match o {
                    Object::Dictionary(d) => Some(is_js_action(&pdf, d)),
                    _ => None,
                })
                .unwrap_or(false);
            if let Some(d) = pdf.objects.get_mut(&id).and_then(dict_mut) {
                // Zusatzaktionen (Seiten, Felder, Annotationen) und JavaScript-Aktionen.
                d.remove(b"AA");
                if is_js {
                    // Eine JavaScript-Aktion ohne /JS tut nichts mehr.
                    d.remove(b"JS");
                }
                if d.get(b"A")
                    .ok()
                    .is_some_and(|a| a.as_dict().is_ok_and(is_js_name))
                {
                    d.remove(b"A");
                }
            }
        }
        if found.javascript > 0 {
            done.push("JavaScript und automatische Aktionen entfernt".into());
        }
    }
    if opts.embedded_files {
        remove_name_tree(&mut pdf, b"EmbeddedFiles");
        let n = remove_annots(&mut pdf, |sub| sub == b"FileAttachment");
        if found.embedded_files > 0 || n > 0 {
            done.push("Eingebettete Dateien entfernt".into());
        }
    }
    if opts.comments {
        let n = remove_annots(&mut pdf, |sub| !matches!(sub, b"Widget" | b"Link"));
        if n > 0 {
            done.push(format!("{n} Kommentar(e)/Annotation(en) entfernt"));
        }
    }
    if opts.form_data {
        let n = clear_form_data(&mut pdf);
        if n > 0 {
            done.push(format!("{n} Formularwert(e) gelöscht"));
        }
    }
    let out = rewrite(pdf)?;
    done.push("Alte Revisionen und unbenutzte Objekte entfernt".into());
    Ok((out, done))
}

// ---------- Hilfen ----------

fn dict_mut(o: &mut Object) -> Option<&mut Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&mut s.dict),
        _ => None,
    }
}

fn deref_dict<'a>(pdf: &'a lopdf::Document, o: Option<&'a Object>) -> Result<&'a Dictionary> {
    let o = o.ok_or(lopdf::Error::DictKey("".into()))?;
    Ok(pdf.dereference(o)?.1.as_dict()?)
}

fn root_id(pdf: &lopdf::Document) -> Option<ObjectId> {
    pdf.trailer.get(b"Root").and_then(Object::as_reference).ok()
}

fn is_js_name(d: &Dictionary) -> bool {
    d.get(b"S").and_then(Object::as_name).ok() == Some(b"JavaScript")
}

/// Aktion mit /S /JavaScript (oder ein Dictionary mit /JS).
fn is_js_action(_pdf: &lopdf::Document, d: &Dictionary) -> bool {
    is_js_name(d) || d.has(b"JS")
}

/// Entfernt einen Eintrag aus dem /Names-Dictionary des Katalogs.
fn remove_name_tree(pdf: &mut lopdf::Document, key: &[u8]) {
    let Some(root) = root_id(pdf) else { return };
    let names_ref = pdf
        .get_dictionary(root)
        .ok()
        .and_then(|c| c.get(b"Names").ok())
        .and_then(|n| n.as_reference().ok());
    let target = match names_ref {
        Some(id) => pdf.get_object_mut(id).ok().and_then(dict_mut),
        None => pdf
            .get_object_mut(root)
            .ok()
            .and_then(dict_mut)
            .and_then(|c| c.get_mut(b"Names").ok())
            .and_then(dict_mut),
    };
    if let Some(n) = target {
        n.remove(key);
    }
}

/// Entfernt Annotationen, deren /Subtype `pred` erfüllt (samt Popups). Liefert die Anzahl.
fn remove_annots(pdf: &mut lopdf::Document, pred: impl Fn(&[u8]) -> bool) -> usize {
    let mut count = 0;
    for page in pdf.get_pages().into_values().collect::<Vec<_>>() {
        let annots: Vec<Object> = match pdf
            .get_dictionary(page)
            .ok()
            .and_then(|d| d.get(b"Annots").ok())
        {
            Some(o) => match pdf.dereference(o) {
                Ok((_, Object::Array(a))) => a.clone(),
                _ => continue,
            },
            None => continue,
        };
        let mut gone = HashSet::new();
        for a in &annots {
            let Ok(id) = a.as_reference() else { continue };
            let Ok(d) = pdf.get_dictionary(id) else {
                continue;
            };
            let sub = d
                .get(b"Subtype")
                .and_then(Object::as_name)
                .unwrap_or_default();
            if sub != b"Popup" && pred(sub) {
                gone.insert(id);
                count += 1;
                if let Ok(p) = d.get(b"Popup").and_then(Object::as_reference) {
                    gone.insert(p);
                }
            }
        }
        if gone.is_empty() {
            continue;
        }
        let kept: Vec<Object> = annots
            .into_iter()
            .filter(|a| a.as_reference().map_or(true, |id| !gone.contains(&id)))
            .collect();
        if let Ok(d) = pdf.get_object_mut(page).and_then(Object::as_dict_mut) {
            d.set("Annots", kept);
        }
    }
    count
}

/// Löscht Feldwerte (/V, /DV), XFA-Daten und die sichtbaren Werte (Erscheinungsbilder) der
/// Text-/Auswahlfelder; Schalter werden ausgeschaltet.
fn clear_form_data(pdf: &mut lopdf::Document) -> usize {
    let mut count = 0;
    if let Some(root) = root_id(pdf) {
        let acro = pdf
            .get_dictionary(root)
            .ok()
            .and_then(|c| c.get(b"AcroForm").ok())
            .and_then(|a| a.as_reference().ok());
        let target = match acro {
            Some(id) => pdf.get_object_mut(id).ok().and_then(dict_mut),
            None => pdf
                .get_object_mut(root)
                .ok()
                .and_then(dict_mut)
                .and_then(|c| c.get_mut(b"AcroForm").ok())
                .and_then(dict_mut),
        };
        if let Some(af) = target {
            if af.remove(b"XFA").is_some() {
                count += 1;
            }
            // Reader sollen leere Felder selbst neu zeichnen.
            af.set("NeedAppearances", true);
        }
    }
    for obj in pdf.objects.values_mut() {
        let Some(d) = dict_mut(obj) else { continue };
        let is_field = d.has(b"FT") || d.has(b"T");
        let is_widget = d.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget");
        if is_field && d.remove(b"V").is_some() {
            count += 1;
        }
        if is_field {
            d.remove(b"DV");
            d.remove(b"I");
        }
        if is_widget {
            if d.has(b"AS") {
                d.set("AS", "Off");
            } else {
                // Text-/Auswahlfelder: Erscheinungsbild zeigt den alten Wert.
                d.remove(b"AP");
            }
        }
    }
    count
}

/// Ebenen (Optional Content Groups), die standardmäßig ausgeblendet sind.
fn hidden_ocgs(pdf: &lopdf::Document) -> HashSet<ObjectId> {
    let Ok(cat) = pdf.catalog() else {
        return HashSet::new();
    };
    let Ok(props) = deref_dict(pdf, cat.get(b"OCProperties").ok()) else {
        return HashSet::new();
    };
    let all: Vec<ObjectId> = props
        .get(b"OCGs")
        .ok()
        .and_then(|o| o.as_array().ok())
        .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default();
    let Ok(d) = deref_dict(pdf, props.get(b"D").ok()) else {
        return HashSet::new();
    };
    let list = |key: &[u8]| -> HashSet<ObjectId> {
        d.get(key)
            .ok()
            .and_then(|o| o.as_array().ok())
            .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default()
    };
    if d.get(b"BaseState").and_then(Object::as_name).ok() == Some(b"OFF") {
        let on = list(b"ON");
        all.into_iter().filter(|id| !on.contains(id)).collect()
    } else {
        list(b"OFF")
    }
}

/// Ist ein /OC-Eintrag (OCG oder OCMD) ausgeblendet?
fn oc_hidden(pdf: &lopdf::Document, oc: &Object, hidden: &HashSet<ObjectId>) -> bool {
    let Ok(id) = oc.as_reference() else {
        return false;
    };
    if hidden.contains(&id) {
        return true;
    }
    // OCMD: sichtbar, wenn eine der Gruppen sichtbar ist (Standard /AnyOn) – wir werten nur
    // diesen häufigsten Fall aus: alle Gruppen versteckt = versteckt.
    let Ok(d) = pdf.get_dictionary(id) else {
        return false;
    };
    if d.get(b"Type").and_then(Object::as_name).ok() != Some(b"OCMD") {
        return false;
    }
    let groups: Vec<ObjectId> = match d.get(b"OCGs") {
        Ok(Object::Array(a)) => a.iter().filter_map(|o| o.as_reference().ok()).collect(),
        Ok(Object::Reference(r)) => vec![*r],
        _ => Vec::new(),
    };
    !groups.is_empty() && groups.iter().all(|g| hidden.contains(g))
}

/// Entfernt Inhalte ausgeblendeter Ebenen aus allen Seiten, dazu die Ebenen-Definition.
fn remove_hidden_layers(pdf: &mut lopdf::Document) -> Result<usize> {
    let hidden = hidden_ocgs(pdf);
    if !hidden.is_empty() {
        for page in pdf.get_pages().into_values().collect::<Vec<_>>() {
            let res = page_resources(pdf, page).cloned().unwrap_or_default();
            let bytes = page_content_bytes(pdf, page)?;
            let ops = Content::decode(&bytes)?.operations;
            let (new_ops, changed) = strip_hidden(pdf, ops, &res, &hidden);
            if changed {
                let mut s = Stream::new(
                    Dictionary::new(),
                    Content {
                        operations: new_ops,
                    }
                    .encode()?,
                );
                let _ = s.compress();
                let id = pdf.add_object(s);
                if let Ok(d) = pdf.get_object_mut(page).and_then(Object::as_dict_mut) {
                    d.set("Contents", id);
                }
            }
        }
        // Annotationen auf versteckten Ebenen.
        for page in pdf.get_pages().into_values().collect::<Vec<_>>() {
            let annots: Vec<Object> = match pdf
                .get_dictionary(page)
                .ok()
                .and_then(|d| d.get(b"Annots").ok())
            {
                Some(o) => match pdf.dereference(o) {
                    Ok((_, Object::Array(a))) => a.clone(),
                    _ => continue,
                },
                None => continue,
            };
            let kept: Vec<Object> = annots
                .into_iter()
                .filter(|a| {
                    let oc = a
                        .as_reference()
                        .ok()
                        .and_then(|id| pdf.get_dictionary(id).ok())
                        .and_then(|d| d.get(b"OC").ok());
                    !oc.is_some_and(|oc| oc_hidden(pdf, oc, &hidden))
                })
                .collect();
            if let Ok(d) = pdf.get_object_mut(page).and_then(Object::as_dict_mut) {
                d.set("Annots", kept);
            }
        }
    }
    // Ebenen-Definition entfernen: alles, was übrig ist, ist sichtbar.
    if let Some(root) = root_id(pdf)
        && let Ok(c) = pdf.get_object_mut(root).and_then(Object::as_dict_mut)
    {
        c.remove(b"OCProperties");
    }
    Ok(hidden.len())
}

/// Entfernt `/OC /Name BDC … EMC`-Abschnitte und `Do` von XObjects auf versteckten Ebenen.
fn strip_hidden(
    pdf: &lopdf::Document,
    ops: Vec<Operation>,
    resources: &Dictionary,
    hidden: &HashSet<ObjectId>,
) -> (Vec<Operation>, bool) {
    let props = resources
        .get(b"Properties")
        .ok()
        .and_then(|o| pdf.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok());
    let xobjects = resources
        .get(b"XObject")
        .ok()
        .and_then(|o| pdf.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok());
    let mut out = Vec::with_capacity(ops.len());
    let mut changed = false;
    // Tiefe der Markierungsabschnitte; `skip_from` = Tiefe, ab der übersprungen wird.
    let mut depth = 0usize;
    let mut skip_from: Option<usize> = None;
    for op in ops {
        match op.operator.as_str() {
            "BDC" | "BMC" => {
                depth += 1;
                if skip_from.is_none() && op.operator == "BDC" {
                    let is_oc = op.operands.first().and_then(|o| o.as_name().ok()) == Some(b"OC");
                    let target = op.operands.get(1).and_then(|o| match o {
                        Object::Name(n) => props.and_then(|p| p.get(n).ok()),
                        other => Some(other),
                    });
                    if is_oc && target.is_some_and(|t| oc_hidden(pdf, t, hidden)) {
                        skip_from = Some(depth);
                        changed = true;
                        continue;
                    }
                }
            }
            "EMC" => {
                let closing = depth;
                depth = depth.saturating_sub(1);
                if skip_from == Some(closing) {
                    skip_from = None;
                    continue;
                }
            }
            "Do" if skip_from.is_none() => {
                let oc = op
                    .operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .and_then(|n| xobjects?.get(n).ok())
                    .and_then(|o| pdf.dereference(o).ok())
                    .and_then(|(_, o)| o.as_stream().ok())
                    .and_then(|s| s.dict.get(b"OC").ok());
                if oc.is_some_and(|oc| oc_hidden(pdf, oc, hidden)) {
                    changed = true;
                    continue;
                }
            }
            _ => {}
        }
        if skip_from.is_none() {
            out.push(op);
        }
    }
    (out, changed)
}
