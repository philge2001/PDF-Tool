//! Formularfelder speichern (Teil des inkrementellen Speicherns in `save`).
//!
//! - Gelöschte Felder verschwinden aus /AcroForm /Fields bzw. aus den /Kids ihres
//!   Elternfelds; ihre Widgets aus den /Annots der Seiten.
//! - Geänderte Felder werden unter derselben Objektnummer neu geschrieben. Dabei ändern
//!   wir nur die Einträge, die sich wirklich geändert haben – unbekannte Einträge (z. B.
//!   Aktionen) und die Originalschrift in /DA bleiben so erhalten.
//! - Neue Felder bekommen neue Objekte und werden in /AcroForm eingetragen.

use crate::annotation::appearance::helvetica_dict;
use crate::annotation::import::deref;
use crate::document::Document;
use crate::error::{CoreError, Result};
use crate::form::import::{
    FF_COMBO, FF_EDIT, FF_MULTILINE, FF_MULTISELECT, FF_PASSWORD, FF_PUSHBUTTON, FF_RADIO,
    FF_READ_ONLY, FF_REQUIRED,
};
use crate::form::{
    Align, FieldId, FieldKind, FieldValue, FormField, Widget, appearance::WidgetAppearance,
};
use crate::geometry::Color;
use crate::pdf_string;
use lopdf::{Dictionary, IncrementalDocument, Object, ObjectId, dictionary};
use std::collections::{HashMap, HashSet};

/// Optionsfelder: Ein Klick auf die gewählte Option schaltet sie nicht wieder aus.
const FF_NO_TOGGLE_TO_OFF: i64 = 1 << 14;
/// Alle Flag-Bits, die sich aus dem Modell ergeben (der Rest bleibt, wie er war).
const FF_KNOWN: i64 = FF_READ_ONLY
    | FF_REQUIRED
    | FF_MULTILINE
    | FF_PASSWORD
    | FF_NO_TOGGLE_TO_OFF
    | FF_RADIO
    | FF_PUSHBUTTON
    | FF_COMBO
    | FF_EDIT
    | FF_MULTISELECT;

/// Was `save` für die Seiten und die App wissen muss.
#[derive(Default)]
pub(crate) struct FormChanges {
    /// Widgets, die aus den /Annots ihrer Seite entfernt werden.
    pub removed_widgets: HashSet<ObjectId>,
    /// Neue Widgets pro Seite (Objektnummer der Seite).
    pub added_widgets: HashMap<ObjectId, Vec<ObjectId>>,
    /// Felder mit neu vergebenen Objektnummern: (Feld, Feldobjekt, Widgets in Modellreihenfolge).
    pub written: Vec<(FieldId, ObjectId, Vec<Option<ObjectId>>)>,
}

pub(crate) fn write(doc: &Document, inc: &mut IncrementalDocument) -> Result<FormChanges> {
    let pdf = doc.pdf();
    let pages: HashSet<ObjectId> = doc.pages().iter().map(|p| p.object_id).collect();
    let forms = &doc.model().forms;
    let mut ch = FormChanges::default();

    // 1) Gelöschte Felder.
    let current: HashSet<ObjectId> = forms.iter().filter_map(|f| f.source).collect();
    let mut top_removed = HashSet::new();
    for old in doc.imported_fields() {
        let Some(src) = old.source.filter(|s| !current.contains(s)) else {
            continue;
        };
        ch.removed_widgets
            .extend(old.widgets.iter().filter_map(|w| w.object_id));
        match parent_of(pdf, src) {
            Some(parent) => edit_kids(inc, parent, |kids| kids.retain(|k| !is_ref(k, src)))?,
            None => {
                top_removed.insert(src);
            }
        }
    }

    // 2) Neue und geänderte Felder.
    let mut top_added = Vec::new();
    for field in forms.iter().filter(|f| f.kind.is_fillable()) {
        match field.source {
            None => {
                let (id, widgets) = write_new(doc, inc, field, &pages, &mut ch)?;
                top_added.push(id);
                ch.written.push((field.id, id, widgets));
            }
            Some(src) => {
                let old = doc.imported_field(src);
                if old != Some(field) {
                    write_existing(doc, inc, field, src, old, &pages, &mut ch)?;
                }
            }
        }
    }

    if !top_added.is_empty() || !top_removed.is_empty() {
        update_acroform(doc.pdf(), inc, &top_added, &top_removed, None)?;
    }
    Ok(ch)
}

fn write_existing(
    doc: &Document,
    inc: &mut IncrementalDocument,
    field: &FormField,
    src: ObjectId,
    old: Option<&FormField>,
    pages: &HashSet<ObjectId>,
    ch: &mut FormChanges,
) -> Result<()> {
    inc.opt_clone_object_to_new_document(src)?;
    write_field_props(dict_mut(inc, src)?, field, old);

    let old_widgets: &[Widget] = old.map_or(&[], |o| &o.widgets);
    let keep: HashSet<ObjectId> = field.widgets.iter().filter_map(|w| w.object_id).collect();
    for id in old_widgets.iter().filter_map(|w| w.object_id) {
        if !keep.contains(&id) {
            ch.removed_widgets.insert(id);
            edit_kids(inc, src, |kids| kids.retain(|k| !is_ref(k, id)))?;
        }
    }

    let mut ids = Vec::new();
    let mut any_new = false;
    for w in &field.widgets {
        match w.object_id {
            Some(wid) => {
                let old_w = old_widgets.iter().find(|o| o.object_id == Some(wid));
                // Widgets auf gelöschten Seiten bleiben, wie sie sind.
                if pages.contains(&w.page) {
                    write_widget(doc, inc, field, w, wid, old_w)?;
                }
                ids.push(Some(wid));
            }
            None if pages.contains(&w.page) => {
                let wid = inc.new_document.new_object_id();
                inc.new_document.set_object(
                    wid,
                    dictionary! { "Type" => "Annot", "Subtype" => "Widget", "Parent" => src },
                );
                write_widget(doc, inc, field, w, wid, None)?;
                edit_kids(inc, src, |kids| kids.push(wid.into()))?;
                ch.added_widgets.entry(w.page).or_default().push(wid);
                ids.push(Some(wid));
                any_new = true;
            }
            None => ids.push(None),
        }
    }
    if any_new {
        ch.written.push((field.id, src, ids));
    }
    Ok(())
}

fn write_new(
    doc: &Document,
    inc: &mut IncrementalDocument,
    field: &FormField,
    pages: &HashSet<ObjectId>,
    ch: &mut FormChanges,
) -> Result<(ObjectId, Vec<Option<ObjectId>>)> {
    let fid = inc.new_document.new_object_id();
    let mut fdict = Dictionary::new();
    write_field_props(&mut fdict, field, None);

    // Optionsfelder (und Felder mit mehreren Widgets): Feld mit Kindern. Sonst stehen
    // Feld und Widget in einem Objekt – so machen es die meisten Programme.
    let separate = field.kind == FieldKind::Radio || field.widgets.len() > 1;
    if !separate {
        let w = &field.widgets[0];
        fdict.set("Type", "Annot");
        fdict.set("Subtype", "Widget");
        inc.new_document.set_object(fid, fdict);
        if !pages.contains(&w.page) {
            return Ok((fid, vec![None]));
        }
        write_widget(doc, inc, field, w, fid, None)?;
        ch.added_widgets.entry(w.page).or_default().push(fid);
        return Ok((fid, vec![Some(fid)]));
    }

    let mut ids = Vec::new();
    let mut kids = Vec::new();
    inc.new_document.set_object(fid, fdict);
    for w in &field.widgets {
        if !pages.contains(&w.page) {
            ids.push(None);
            continue;
        }
        let wid = inc.new_document.new_object_id();
        inc.new_document.set_object(
            wid,
            dictionary! { "Type" => "Annot", "Subtype" => "Widget", "Parent" => fid },
        );
        write_widget(doc, inc, field, w, wid, None)?;
        ch.added_widgets.entry(w.page).or_default().push(wid);
        kids.push(Object::from(wid));
        ids.push(Some(wid));
    }
    dict_mut(inc, fid)?.set("Kids", kids);
    Ok((fid, ids))
}

/// Schreibt die Feld-Einträge, die sich gegenüber `old` geändert haben (alle, wenn neu).
fn write_field_props(dict: &mut Dictionary, field: &FormField, old: Option<&FormField>) {
    // `is_none_or`: true, wenn es kein altes Feld gibt ODER die Bedingung zutrifft.
    let changed = |differs: &dyn Fn(&FormField) -> bool| old.is_none_or(differs);

    if changed(&|o| {
        o.kind != field.kind || o.read_only != field.read_only || o.required != field.required
    }) {
        let ft = match field.kind {
            FieldKind::Text { .. } => "Tx",
            FieldKind::CheckBox | FieldKind::Radio | FieldKind::PushButton => "Btn",
            FieldKind::Combo { .. } | FieldKind::List { .. } => "Ch",
            FieldKind::Signature => "Sig",
        };
        dict.set("FT", ft);
        let base = dict.get(b"Ff").and_then(Object::as_i64).unwrap_or(0);
        dict.set("Ff", (base & !FF_KNOWN) | flags(field));
        match field.kind {
            FieldKind::Text {
                max_len: Some(m), ..
            } => dict.set("MaxLen", m as i64),
            _ => {
                dict.remove(b"MaxLen");
            }
        }
    }
    if changed(&|o| o.name != field.name) {
        dict.set("T", pdf_string::encode(field.short_name()));
    }
    if changed(&|o| o.tooltip != field.tooltip) {
        if field.tooltip.is_empty() {
            dict.remove(b"TU");
        } else {
            dict.set("TU", pdf_string::encode(&field.tooltip));
        }
    }
    if changed(&|o| o.value != field.value) {
        set_value(dict, "V", field, &field.value);
    }
    if changed(&|o| o.default != field.default) {
        set_value(dict, "DV", field, &field.default);
    }
    if changed(&|o| o.font_size != field.font_size || o.text_color != field.text_color) {
        let c = field.text_color;
        dict.set(
            "DA",
            Object::string_literal(format!(
                "/Helv {} Tf {} {} {} rg",
                field.font_size, c.r, c.g, c.b
            )),
        );
    }
    if changed(&|o| o.align != field.align) {
        let q = match field.align {
            Align::Left => 0,
            Align::Center => 1,
            Align::Right => 2,
        };
        dict.set("Q", q);
    }
    if changed(&|o| o.options != field.options) {
        if field.options.is_empty() {
            dict.remove(b"Opt");
        } else {
            let opt: Vec<Object> = field
                .options
                .iter()
                .map(|o| {
                    if o.export == o.display {
                        pdf_string::encode(&o.display)
                    } else {
                        Object::Array(vec![
                            pdf_string::encode(&o.export),
                            pdf_string::encode(&o.display),
                        ])
                    }
                })
                .collect();
            dict.set("Opt", opt);
        }
    }
}

/// /Ff-Bits, die sich aus dem Modell ergeben.
fn flags(field: &FormField) -> i64 {
    let mut ff = 0;
    if field.read_only {
        ff |= FF_READ_ONLY;
    }
    if field.required {
        ff |= FF_REQUIRED;
    }
    ff | match field.kind {
        FieldKind::Text {
            multiline,
            password,
            ..
        } => (if multiline { FF_MULTILINE } else { 0 }) | (if password { FF_PASSWORD } else { 0 }),
        FieldKind::Radio => FF_RADIO | FF_NO_TOGGLE_TO_OFF,
        FieldKind::PushButton => FF_PUSHBUTTON,
        FieldKind::Combo { editable } => FF_COMBO | if editable { FF_EDIT } else { 0 },
        FieldKind::List { multi } => {
            if multi {
                FF_MULTISELECT
            } else {
                0
            }
        }
        FieldKind::CheckBox | FieldKind::Signature => 0,
    }
}

/// Trägt einen Wert unter `key` (/V oder /DV) ein; bei /V von Listenfeldern auch /I.
fn set_value(dict: &mut Dictionary, key: &str, field: &FormField, value: &FieldValue) {
    match value {
        FieldValue::None => {
            dict.remove(key.as_bytes());
        }
        FieldValue::Text(t) => dict.set(key, pdf_string::encode(t)),
        FieldValue::Toggle(state) => {
            let name = state.as_deref().unwrap_or("Off");
            dict.set(key, Object::Name(name.as_bytes().to_vec()));
        }
        FieldValue::Choice(values) => {
            match values.as_slice() {
                [] => {
                    dict.remove(key.as_bytes());
                }
                [one] => dict.set(key, pdf_string::encode(one)),
                many => dict.set(
                    key,
                    Object::Array(many.iter().map(|v| pdf_string::encode(v)).collect()),
                ),
            }
            if key != "V" {
                return;
            }
            // /I: Indizes der gewählten Einträge (hilft Readern bei doppelten Exportwerten).
            if matches!(field.kind, FieldKind::List { .. }) {
                let idx: Vec<Object> = field
                    .options
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| values.contains(&o.export))
                    .map(|(i, _)| Object::from(i as i64))
                    .collect();
                dict.set("I", idx);
            } else {
                dict.remove(b"I");
            }
        }
    }
}

/// Schreibt Geometrie, Farben, Zustand und Erscheinungsbild eines Widgets.
fn write_widget(
    doc: &Document,
    inc: &mut IncrementalDocument,
    field: &FormField,
    w: &Widget,
    wid: ObjectId,
    old: Option<&Widget>,
) -> Result<()> {
    // Erst die Appearance-Streams als eigene Objekte anlegen, dann das Widget ändern
    // (beides braucht `inc` veränderlich – nacheinander ist das kein Problem).
    let mut ap: Option<Dictionary> = None;
    let mut state: Option<String> = None;
    match crate::form::appearance::build(field, w) {
        Some(WidgetAppearance::Single(stream)) => {
            let id = inc.new_document.add_object(stream);
            ap = Some(dictionary! { "N" => id });
        }
        Some(WidgetAppearance::States { on_state, on, off }) => {
            // Vorhandene Zustandsbilder behalten, solange sich am Aussehen nichts geändert
            // hat – dann wird nur /AS umgeschaltet.
            let same_look = old.is_some_and(|o| {
                o.rect == w.rect
                    && o.background == w.background
                    && o.border == w.border
                    && o.border_width == w.border_width
            });
            if !(same_look && existing_states(doc, wid).contains(&on_state)) {
                let on_id = inc.new_document.add_object(on);
                let off_id = inc.new_document.add_object(off);
                let mut n = Dictionary::new();
                n.set(on_state.as_bytes(), on_id);
                n.set("Off", off_id);
                ap = Some(dictionary! { "N" => n });
            }
            state = Some(if field.widget_is_on(w) {
                on_state
            } else {
                "Off".into()
            });
        }
        None => {}
    }

    inc.opt_clone_object_to_new_document(wid)?;
    let d = dict_mut(inc, wid)?;
    let changed = |differs: &dyn Fn(&Widget) -> bool| old.is_none_or(differs);
    if old.is_none() {
        d.set("P", w.page);
    }
    if changed(&|o| o.rect != w.rect) {
        let r = w.rect;
        d.set(
            "Rect",
            vec![r.x0.into(), r.y0.into(), r.x1.into(), r.y1.into()],
        );
    }
    if changed(&|o| o.hidden != w.hidden) {
        let f = d.get(b"F").and_then(Object::as_i64).unwrap_or(4);
        // Bit 3 (4) = drucken, Bit 2 (2) = ausgeblendet.
        d.set("F", if w.hidden { (f | 2) & !4 } else { (f & !2) | 4 });
    }
    if changed(&|o| o.background != w.background || o.border != w.border) {
        let mut mk = match d.get(b"MK") {
            Ok(Object::Dictionary(mk)) => mk.clone(),
            _ => Dictionary::new(),
        };
        set_color(&mut mk, "BG", w.background);
        set_color(&mut mk, "BC", w.border);
        d.set("MK", mk);
    }
    if changed(&|o| o.border_width != w.border_width) {
        d.set("BS", dictionary! { "W" => w.border_width, "S" => "S" });
    }
    if let Some(ap) = ap {
        d.set("AP", ap);
    }
    if let Some(state) = state {
        d.set("AS", Object::Name(state.into_bytes()));
    }
    Ok(())
}

fn set_color(mk: &mut Dictionary, key: &str, c: Option<Color>) {
    match c {
        Some(c) => mk.set(key, vec![c.r.into(), c.g.into(), c.b.into()]),
        None => {
            mk.remove(key.as_bytes());
        }
    }
}

/// Namen der Zustände in /AP /N eines Widgets in der Datei (leer, wenn es keine gibt).
fn existing_states(doc: &Document, widget: ObjectId) -> Vec<String> {
    let pdf = doc.pdf();
    let Ok(d) = pdf.get_dictionary(widget) else {
        return Vec::new();
    };
    d.get(b"AP")
        .ok()
        .and_then(|ap| deref(pdf, ap).as_dict().ok())
        .and_then(|ap| ap.get(b"N").ok())
        .and_then(|n| deref(pdf, n).as_dict().ok())
        .map(|n| {
            n.iter()
                .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn is_ref(o: &Object, id: ObjectId) -> bool {
    o.as_reference().is_ok_and(|r| r == id)
}

pub(crate) fn dict_mut(inc: &mut IncrementalDocument, id: ObjectId) -> Result<&mut Dictionary> {
    Ok(inc
        .new_document
        .get_object_mut(id)
        .and_then(Object::as_dict_mut)?)
}

fn parent_of(pdf: &lopdf::Document, id: ObjectId) -> Option<ObjectId> {
    pdf.get_dictionary(id)
        .ok()?
        .get(b"Parent")
        .ok()?
        .as_reference()
        .ok()
}

/// Ändert die /Kids eines Felds (steht /Kids als eigenes Objekt, wird es direkt eingetragen).
pub(crate) fn edit_kids(
    inc: &mut IncrementalDocument,
    field: ObjectId,
    f: impl FnOnce(&mut Vec<Object>),
) -> Result<()> {
    let mut kids = resolved_array(inc, field, b"Kids");
    f(&mut kids);
    inc.opt_clone_object_to_new_document(field)?;
    dict_mut(inc, field)?.set("Kids", kids);
    Ok(())
}

/// Ein Array-Eintrag eines Objekts, Verweise aufgelöst (aus dem neuen oder alten Stand).
pub(crate) fn resolved_array(inc: &IncrementalDocument, id: ObjectId, key: &[u8]) -> Vec<Object> {
    let get = |id: ObjectId| {
        inc.new_document
            .get_object(id)
            .ok()
            .or_else(|| inc.get_prev_documents().get_object(id).ok())
    };
    let Some(Object::Dictionary(d)) = get(id) else {
        return Vec::new();
    };
    match d.get(key) {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => match get(*r) {
            Some(Object::Array(a)) => a.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Trägt neue Felder in /AcroForm /Fields ein (legt /AcroForm bei Bedarf an) und
/// entfernt gelöschte. Sorgt außerdem für die Schrift /Helv in /DR, die /DA benutzt.
/// `sig_flags`: bei Signaturen 3 (SignaturesExist | AppendOnly) – sagt Readern, dass sie
/// nur noch inkrementell speichern dürfen.
pub(crate) fn update_acroform(
    pdf: &lopdf::Document,
    inc: &mut IncrementalDocument,
    added: &[ObjectId],
    removed: &HashSet<ObjectId>,
    sig_flags: Option<i64>,
) -> Result<()> {
    let root = pdf
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| CoreError::Other("Trailer ohne /Root".into()))?;
    let af_id = match pdf.catalog()?.get(b"AcroForm") {
        Ok(Object::Reference(id)) => *id,
        other => {
            // Fehlt oder steht direkt im Katalog: als eigenes Objekt neu anlegen.
            let existing = match other {
                Ok(Object::Dictionary(d)) => d.clone(),
                _ => Dictionary::new(),
            };
            let id = inc.new_document.add_object(existing);
            inc.opt_clone_object_to_new_document(root)?;
            dict_mut(inc, root)?.set("AcroForm", id);
            id
        }
    };
    let mut fields = resolved_array(inc, af_id, b"Fields");
    fields.retain(|f| f.as_reference().map_or(true, |r| !removed.contains(&r)));
    fields.extend(added.iter().map(|&id| Object::from(id)));

    // /DR /Font auflösen (kann aus Verweisen bestehen) und /Helv ergänzen.
    let af_old = inc
        .new_document
        .get_object(af_id)
        .ok()
        .or_else(|| pdf.get_object(af_id).ok())
        .and_then(|o| o.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    let mut dr = af_old
        .get(b"DR")
        .ok()
        .and_then(|o| deref(pdf, o).as_dict().ok())
        .cloned()
        .unwrap_or_default();
    let mut fonts = dr
        .get(b"Font")
        .ok()
        .and_then(|o| deref(pdf, o).as_dict().ok())
        .cloned()
        .unwrap_or_default();
    if !fonts.has(b"Helv") {
        fonts.set("Helv", helvetica_dict());
    }
    dr.set("Font", fonts);

    inc.opt_clone_object_to_new_document(af_id)?;
    let af = dict_mut(inc, af_id)?;
    af.set("Fields", fields);
    af.set("DR", dr);
    if !af.has(b"DA") {
        af.set("DA", Object::string_literal("/Helv 0 Tf 0 g"));
    }
    if let Some(flags) = sig_flags {
        af.set("SigFlags", flags);
    }
    Ok(())
}
