//! Inkrementelles Speichern: Originalbytes bleiben unverändert, Änderungen werden angehängt
//! (ISO 32000-1, Abschnitt 7.5.6). Dadurch bleiben z. B. vorhandene Signaturen gültig.

use crate::annotation::{AnnotId, Annotation, appearance, export};
use crate::date::pdf_date_now;
use crate::document::{Document, inherited};
use crate::error::{CoreError, Result};
use crate::form::{self, FieldKind, FieldValue, FormField, appearance::WidgetAppearance};
use crate::pdf_string;
use lopdf::{Dictionary, IncrementalDocument, Object, ObjectId, dictionary};
use std::collections::{HashMap, HashSet};

/// Ergebnis eines Speichervorgangs.
pub(crate) struct SaveOutput {
    pub bytes: Vec<u8>,
    /// Neu geschriebene Annotationen: (ID, Objektnummer, /NM, /M).
    pub written: Vec<(AnnotId, ObjectId, String, String)>,
    /// Pro Seitenindex: Positionen in /Annots, die FolioCraft selbst zeichnet
    /// (der Renderer blendet genau diese aus).
    pub hidden: HashMap<u32, Vec<usize>>,
}

/// /Annots einer Seite – auch für Seiten, die erst eingefügt und noch nicht gespeichert sind.
fn page_annots(doc: &Document, page: ObjectId) -> Vec<Object> {
    let Some(Object::Dictionary(d)) = doc.object(page) else {
        return Vec::new();
    };
    match d.get(b"Annots") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(id)) => match doc.object(*id) {
            Some(Object::Array(a)) => a.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Schreibt Änderungen am Seitenbaum: Reihenfolge, gelöschte Seiten, Rotation.
fn write_page_tree(doc: &Document, inc: &mut IncrementalDocument) -> Result<()> {
    let pdf = doc.pdf();
    let pages = doc.pages();
    let current: Vec<(ObjectId, u16)> = pages.iter().map(|p| (p.object_id, p.rotation)).collect();
    let saved = doc.saved_pages();
    if current == saved {
        return Ok(());
    }

    let same_order =
        current.len() == saved.len() && current.iter().zip(saved).all(|(a, b)| a.0 == b.0);
    if same_order {
        // Nur gedreht: betroffene Seiten mit neuem /Rotate neu schreiben.
        for (&(id, rot), &(_, old_rot)) in current.iter().zip(saved) {
            if rot != old_rot {
                inc.opt_clone_object_to_new_document(id)?;
                inc.new_document
                    .get_object_mut(id)
                    .and_then(Object::as_dict_mut)?
                    .set("Rotate", rot as i64);
            }
        }
        return Ok(());
    }

    // Reihenfolge/Anzahl geändert: neuer flacher Seitenbaum direkt unter dem Wurzelknoten.
    // Zwischenknoten fallen weg, deshalb bekommt jede Seite die Attribute, die sie bisher
    // von dort geerbt hat, direkt eingetragen.
    let root_id = pdf
        .catalog()?
        .get(b"Pages")?
        .as_reference()
        .map_err(|_| CoreError::Other("Katalog ohne gültiges /Pages".into()))?;
    for p in pages {
        let inherit: Vec<(&[u8], Object)> = [&b"MediaBox"[..], b"CropBox", b"Resources"]
            .into_iter()
            .filter_map(|key| Some((key, inherited(pdf, p.object_id, key)?.clone())))
            .collect();
        inc.opt_clone_object_to_new_document(p.object_id)?;
        let dict = inc
            .new_document
            .get_object_mut(p.object_id)
            .and_then(Object::as_dict_mut)?;
        for (key, value) in inherit {
            if !dict.has(key) {
                dict.set(key, value);
            }
        }
        dict.set("Parent", root_id);
        dict.set("Rotate", p.rotation as i64);
    }
    inc.opt_clone_object_to_new_document(root_id)?;
    let root = inc
        .new_document
        .get_object_mut(root_id)
        .and_then(Object::as_dict_mut)?;
    root.set(
        "Kids",
        Object::Array(pages.iter().map(|p| p.object_id.into()).collect()),
    );
    root.set("Count", pages.len() as i64);
    Ok(())
}

/// Eindeutiger Name (/NM) für neue Annotationen.
fn unique_name(id: AnnotId) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("foliocraft-{nanos:x}-{}", id.0)
}

pub(crate) fn incremental(doc: &Document) -> Result<SaveOutput> {
    let pdf = doc.pdf();
    let mut inc = IncrementalDocument::create_from(doc.bytes().to_vec(), pdf.clone());
    let now = pdf_date_now();
    let mut written = Vec::new();
    let mut hidden = HashMap::new();
    let store = &doc.model().annotations;

    // Neue Objekte (eingefügte Seiten usw.) zuerst – der Seitenbaum verweist darauf.
    for (&id, obj) in &doc.model().extra_objects {
        inc.new_document.set_object(id, obj.clone());
    }
    // Neue Nummern (z. B. für Appearance Streams) erst hinter allen vergebenen.
    let reserved = doc.next_free_object_id().saturating_sub(1);
    inc.new_document.max_id = inc.new_document.max_id.max(reserved);

    write_page_tree(doc, &mut inc)?;
    write_forms(doc, &mut inc)?;
    let widgets: HashSet<ObjectId> = doc.model().forms.managed_widget_ids().collect();
    let is_widget = |o: &Object| o.as_reference().is_ok_and(|id| widgets.contains(&id));

    // Popups von importierten Annotationen, die gelöscht wurden, fliegen mit raus.
    // (Global bestimmt: Seitenindizes können sich seit dem Laden geändert haben.)
    let kept_all: HashSet<ObjectId> = store
        .iter()
        .filter_map(|a| a.source.map(|s| s.object_id))
        .collect();
    let dropped_popups: HashSet<ObjectId> = doc
        .imported()
        .filter_map(|a| a.source)
        .filter(|s| !kept_all.contains(&s.object_id))
        .filter_map(|s| s.popup)
        .collect();

    for (page_idx, info) in doc.pages().iter().enumerate() {
        let original = page_annots(doc, info.object_id);
        let managed: Vec<&Annotation> = store.on_page(page_idx).collect();
        let kept: HashSet<ObjectId> = managed
            .iter()
            .filter_map(|a| a.source.map(|s| s.object_id))
            .collect();

        let mut changed = false;
        let mut new_list: Vec<Object> = Vec::with_capacity(original.len() + managed.len());

        // 1) Fremde Einträge in Originalreihenfolge übernehmen.
        for entry in &original {
            if let Object::Reference(id) = entry {
                if doc.is_imported(*id) {
                    // Verwaltete Annotation: wird unten neu eingereiht (oder ist gelöscht).
                    changed |= !kept.contains(id);
                    continue;
                }
                if dropped_popups.contains(id) {
                    changed = true;
                    continue;
                }
            }
            new_list.push(entry.clone());
        }

        // 2) Verwaltete Annotationen anhängen.
        let foreign_count = new_list.len();
        for a in managed {
            if let Some(src) = a.source
                && doc.imported_snapshot(src.object_id) == Some(a)
            {
                new_list.push(src.object_id.into()); // unverändert -> alter Verweis
                continue;
            }
            changed = true;
            let mut a = a.clone();
            if a.name.is_empty() {
                a.name = unique_name(a.id);
            }
            a.modified = now.clone();

            let base = a.source.and_then(|s| pdf.get_dictionary(s.object_id).ok());
            let mut dict = export::to_dict(&a, info.object_id, base);
            let ap_id = inc
                .new_document
                .add_object(appearance::to_stream(appearance::build(&a)));
            dict.set("AP", dictionary! { "N" => ap_id });
            let obj_id = match a.source {
                // Geändert: unter derselben Objektnummer neu schreiben (ersetzt das alte Objekt).
                Some(src) => {
                    inc.new_document.set_object(src.object_id, dict);
                    src.object_id
                }
                None => inc.new_document.add_object(dict),
            };
            new_list.push(obj_id.into());
            written.push((a.id, obj_id, a.name, a.modified));
        }

        // Welche /Annots-Positionen zeichnet die App selbst? Wird die Seite neu geschrieben,
        // stehen die verwalteten hinten; sonst gilt die Originalreihenfolge.
        // Widgets ausfüllbarer Felder sind fremde Einträge, werden aber ebenfalls ausgeblendet.
        let managed_positions: Vec<usize> = if changed {
            new_list[..foreign_count]
                .iter()
                .enumerate()
                .filter(|(_, o)| is_widget(o))
                .map(|(i, _)| i)
                .chain(foreign_count..new_list.len())
                .collect()
        } else {
            original
                .iter()
                .enumerate()
                .filter(|(_, o)| {
                    is_widget(o) || o.as_reference().is_ok_and(|id| doc.is_imported(id))
                })
                .map(|(i, _)| i)
                .collect()
        };
        if !managed_positions.is_empty() {
            hidden.insert(page_idx as u32, managed_positions);
        }

        // 3) Seite nur anfassen, wenn sich ihre Annotationen geändert haben.
        if changed {
            inc.opt_clone_object_to_new_document(info.object_id)?;
            let page = inc
                .new_document
                .get_object_mut(info.object_id)
                .and_then(Object::as_dict_mut)?;
            page.set("Annots", Object::Array(new_list));
        }
    }

    let mut bytes = Vec::new();
    // Schreiben in einen Vec kann praktisch nicht scheitern; der Fehlertyp ist io::Error.
    inc.save_to(&mut bytes)
        .map_err(|e| crate::error::CoreError::Other(format!("PDF schreiben: {e}")))?;
    Ok(SaveOutput {
        bytes,
        written,
        hidden,
    })
}

/// Schreibt geänderte Formularfelder: Wert ins Feld (/V), Erscheinungsbild in jedes Widget.
fn write_forms(doc: &Document, inc: &mut IncrementalDocument) -> Result<()> {
    let pages: HashSet<ObjectId> = doc.pages().iter().map(|p| p.object_id).collect();
    for field in doc.model().forms.iter() {
        // Neue Felder (ohne Herkunft) kommen mit dem Formular-Editor (3.3).
        let Some(src) = field.source else { continue };
        if doc.imported_field(src) == Some(field) || !field.kind.is_fillable() {
            continue;
        }
        inc.opt_clone_object_to_new_document(src)?;
        let dict = inc
            .new_document
            .get_object_mut(src)
            .and_then(Object::as_dict_mut)?;
        set_value(dict, field);

        for w in &field.widgets {
            // Widgets auf gelöschten Seiten bleiben, wie sie sind.
            let Some(wid) = w.object_id.filter(|_| pages.contains(&w.page)) else {
                continue;
            };
            let existing_states = existing_states(doc, wid);
            match form::appearance::build(field, w) {
                Some(WidgetAppearance::Single(stream)) => {
                    let ap = inc.new_document.add_object(stream);
                    inc.opt_clone_object_to_new_document(wid)?;
                    let wd = inc
                        .new_document
                        .get_object_mut(wid)
                        .and_then(Object::as_dict_mut)?;
                    wd.set("AP", dictionary! { "N" => ap });
                }
                Some(WidgetAppearance::States { on_state, on, off }) => {
                    // Vorhandene Erscheinungsbilder behalten, nur den Zustand (/AS) umschalten.
                    let ap = if existing_states.contains(&on_state) {
                        None
                    } else {
                        let on_id = inc.new_document.add_object(on);
                        let off_id = inc.new_document.add_object(off);
                        let mut n = Dictionary::new();
                        n.set(on_state.as_bytes(), on_id);
                        n.set("Off", off_id);
                        Some(dictionary! { "N" => n })
                    };
                    let state = if field.widget_is_on(w) {
                        on_state.as_str()
                    } else {
                        "Off"
                    };
                    inc.opt_clone_object_to_new_document(wid)?;
                    let wd = inc
                        .new_document
                        .get_object_mut(wid)
                        .and_then(Object::as_dict_mut)?;
                    wd.set("AS", Object::Name(state.as_bytes().to_vec()));
                    if let Some(ap) = ap {
                        wd.set("AP", ap);
                    }
                }
                None => {}
            }
        }
    }
    Ok(())
}

/// Namen der Zustände in /AP /N eines Widgets (leer, wenn es keine gibt).
fn existing_states(doc: &Document, widget: ObjectId) -> Vec<String> {
    let pdf = doc.pdf();
    let deref = |o: &Object| pdf.dereference(o).map(|(_, o)| o.clone()).ok();
    let Ok(d) = pdf.get_dictionary(widget) else {
        return Vec::new();
    };
    d.get(b"AP")
        .ok()
        .and_then(deref)
        .and_then(|ap| ap.as_dict().ok()?.get(b"N").ok().and_then(deref))
        .and_then(|n| {
            n.as_dict().ok().map(|n| {
                n.iter()
                    .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Trägt den Wert des Felds als /V (und bei Listenfeldern /I) ein.
fn set_value(dict: &mut Dictionary, field: &FormField) {
    match &field.value {
        FieldValue::None => {}
        FieldValue::Text(t) => dict.set("V", pdf_string::encode(t)),
        FieldValue::Toggle(state) => {
            let name = state.as_deref().unwrap_or("Off");
            dict.set("V", Object::Name(name.as_bytes().to_vec()));
        }
        FieldValue::Choice(values) => {
            match values.as_slice() {
                [] => {
                    dict.remove(b"V");
                }
                [one] => dict.set("V", pdf_string::encode(one)),
                many => dict.set(
                    "V",
                    Object::Array(many.iter().map(|v| pdf_string::encode(v)).collect()),
                ),
            }
            // /I: Indizes der gewählten Einträge (hilft Readern bei doppelten Exportwerten).
            if matches!(field.kind, FieldKind::List { .. }) {
                let mut idx: Vec<i64> = field
                    .options
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| values.contains(&o.export))
                    .map(|(i, _)| i as i64)
                    .collect();
                idx.sort_unstable();
                dict.set(
                    "I",
                    Object::Array(idx.into_iter().map(Object::from).collect()),
                );
            } else {
                dict.remove(b"I");
            }
        }
    }
}
