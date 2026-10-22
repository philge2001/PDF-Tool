//! Inkrementelles Speichern: Originalbytes bleiben unverändert, Änderungen werden angehängt
//! (ISO 32000-1, Abschnitt 7.5.6). Dadurch bleiben z. B. vorhandene Signaturen gültig.

use crate::annotation::{AnnotId, Annotation, appearance, export, import};
use crate::date::pdf_date_now;
use crate::document::Document;
use crate::error::Result;
use lopdf::{IncrementalDocument, Object, ObjectId, dictionary};
use std::collections::HashSet;

/// Ergebnis eines Speichervorgangs.
pub(crate) struct SaveOutput {
    pub bytes: Vec<u8>,
    /// Neu geschriebene Annotationen: (ID, Objektnummer, /NM, /M).
    pub written: Vec<(AnnotId, ObjectId, String, String)>,
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
    let store = &doc.model().annotations;

    for (page_idx, info) in doc.pages().iter().enumerate() {
        let original = import::page_annots(pdf, info.object_id);
        let managed: Vec<&Annotation> = store.on_page(page_idx).collect();
        let kept: HashSet<ObjectId> = managed
            .iter()
            .filter_map(|a| a.source.map(|s| s.object_id))
            .collect();
        // Popups von importierten Annotationen, die gelöscht wurden, fliegen mit raus.
        let dropped_popups: HashSet<ObjectId> = doc
            .imported()
            .filter(|a| a.page == page_idx)
            .filter_map(|a| a.source)
            .filter(|s| !kept.contains(&s.object_id))
            .filter_map(|s| s.popup)
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
    Ok(SaveOutput { bytes, written })
}
