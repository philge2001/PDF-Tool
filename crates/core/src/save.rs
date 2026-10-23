//! Inkrementelles Speichern: Originalbytes bleiben unverändert, Änderungen werden angehängt
//! (ISO 32000-1, Abschnitt 7.5.6). Dadurch bleiben z. B. vorhandene Signaturen gültig.

use crate::annotation::{AnnotId, Annotation, appearance, export};
use crate::date::pdf_date_now;
use crate::document::{Document, inherited};
use crate::error::{CoreError, Result};
use crate::form::FieldId;
use crate::form_save;
use lopdf::{IncrementalDocument, Object, ObjectId, dictionary};
use std::collections::{HashMap, HashSet};

/// Ergebnis eines Speichervorgangs.
pub(crate) struct SaveOutput {
    pub bytes: Vec<u8>,
    /// Neu geschriebene Annotationen: (ID, Objektnummer, /NM, /M).
    pub written: Vec<(AnnotId, ObjectId, String, String)>,
    /// Pro Seitenindex: Positionen in /Annots, die FolioCraft selbst zeichnet
    /// (der Renderer blendet genau diese aus).
    pub hidden: HashMap<u32, Vec<usize>>,
    /// Neu angelegte Formularobjekte: (Feld, Objektnummer des Felds, Nummern der Widgets).
    pub written_fields: Vec<(FieldId, ObjectId, Vec<Option<ObjectId>>)>,
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

/// Schreibt bearbeitete Seiteninhalte: neuer Content Stream, Ressourcen als direktes
/// Dictionary (bisherige plus eingebettete Ersatzschriften und neue Bilder).
fn write_page_contents(doc: &Document, inc: &mut IncrementalDocument) -> Result<()> {
    let pdf = doc.pdf();
    for info in doc.pages() {
        let page = info.object_id;
        if !doc.content_changed(page) {
            continue;
        }
        let Some(pc) = doc.model().page_content.get(&page) else {
            continue;
        };
        let mut fonts = Vec::new();
        for (name, sub) in &pc.fonts {
            let id = sub.write(&mut |o| inc.new_document.add_object(o))?;
            fonts.push((name.clone(), id));
        }
        let mut stream = lopdf::Stream::new(lopdf::Dictionary::new(), pc.content.clone());
        let _ = stream.compress();
        let content_id = inc.new_document.add_object(stream);

        // Ressourcen auflösen und kopieren (sie können mit anderen Seiten geteilt sein).
        let resolve = |o: &Object| -> lopdf::Dictionary {
            pdf.dereference(o)
                .ok()
                .and_then(|(_, o)| o.as_dict().ok().cloned())
                .unwrap_or_default()
        };
        let mut resources = inherited(pdf, page, b"Resources")
            .map(resolve)
            .unwrap_or_default();
        let mut images = Vec::new();
        for (name, img) in &pc.images {
            let mut stream = img.image.clone();
            if let Some(mask) = &img.smask {
                let mask_id = inc.new_document.add_object(mask.clone());
                stream.dict.set("SMask", mask_id);
            }
            images.push((name.clone(), inc.new_document.add_object(stream)));
        }
        for (category, entries) in [(&b"Font"[..], fonts), (b"XObject", images)] {
            if entries.is_empty() {
                continue;
            }
            let mut dict = resources.get(category).map(resolve).unwrap_or_default();
            for (name, id) in entries {
                dict.set(name, id);
            }
            resources.set(category, dict);
        }
        inc.opt_clone_object_to_new_document(page)?;
        let dict = inc
            .new_document
            .get_object_mut(page)
            .and_then(Object::as_dict_mut)?;
        dict.set("Contents", content_id);
        dict.set("Resources", resources);
    }
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
    write_page_contents(doc, &mut inc)?;
    crate::tags::write(doc, &mut inc)?;
    let forms = form_save::write(doc, &mut inc)?;
    // Widgets ausfüllbarer Felder – auch die gerade erst angelegten.
    let widgets: HashSet<ObjectId> = doc
        .model()
        .forms
        .managed_widget_ids()
        .chain(forms.added_widgets.values().flatten().copied())
        .collect();
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
                if dropped_popups.contains(id) || forms.removed_widgets.contains(id) {
                    changed = true;
                    continue;
                }
            }
            new_list.push(entry.clone());
        }
        // Neue Widgets von Formularfeldern gehören zu den "fremden" Einträgen.
        if let Some(added) = forms.added_widgets.get(&info.object_id) {
            changed = true;
            new_list.extend(added.iter().map(|&id| Object::from(id)));
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
            let stream = appearance::to_stream(appearance::build(&a), &mut |o| {
                inc.new_document.add_object(o)
            })?;
            let ap_id = inc.new_document.add_object(stream);
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
        written_fields: forms.written,
    })
}
