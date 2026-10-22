//! Seiten als neue, eigenständige PDF herausschreiben (Extrahieren, Teilen).
//!
//! Anders als beim Speichern wird hier eine komplett neue Datei erzeugt: Sie enthält nur
//! die gewünschten Seiten und die Objekte, die diese wirklich brauchen.

use crate::document::{Document, inherited};
use crate::error::{CoreError, Result};
use crate::page_import::Copier;
use lopdf::{Object, ObjectId, dictionary};
use std::collections::{BTreeMap, HashMap, HashSet};

impl Document {
    /// Der aktuelle Stand (inkl. ungespeicherter Änderungen) als lopdf-Dokument –
    /// Quelle für [`extract_pages`]. Kann in einen Hintergrund-Thread verschoben werden.
    pub fn snapshot_pdf(&self) -> Result<lopdf::Document> {
        let snap = self.render_snapshot()?;
        Ok(lopdf::Document::load_mem(&snap.bytes)?)
    }
}

/// Erzeugt eine neue PDF aus den Seiten `pages` (Indizes in `source`, in dieser Reihenfolge).
pub fn extract_pages(source: &lopdf::Document, pages: &[usize]) -> Result<Vec<u8>> {
    let all: Vec<ObjectId> = source.get_pages().into_values().collect();
    let wanted: Vec<ObjectId> = pages
        .iter()
        .map(|&i| all.get(i).copied().ok_or(CoreError::PageOutOfRange(i)))
        .collect::<Result<_>>()?;
    if wanted.is_empty() {
        return Err(CoreError::Other("Keine Seiten ausgewählt".into()));
    }
    let wanted_set: HashSet<ObjectId> = wanted.iter().copied().collect();

    // Objekt 1 = Katalog, 2 = Seitenbaum; ab 3 die kopierten Objekte.
    let catalog_id: ObjectId = (1, 0);
    let root_id: ObjectId = (2, 0);
    let mut copier = Copier {
        source,
        next_id: 3,
        mapped: HashMap::new(),
        out: BTreeMap::new(),
        foreign_pages: all
            .iter()
            .copied()
            .filter(|id| !wanted_set.contains(id))
            .collect(),
    };

    // Seitennummern vorab vergeben (Links zwischen extrahierten Seiten bleiben erhalten).
    // Wird dieselbe Seite mehrfach gewünscht, entsteht pro Vorkommen eine eigene Kopie.
    let mut kids = Vec::new();
    for &old in &wanted {
        let new = copier.fresh_id();
        copier.mapped.entry(old).or_insert(new);
        kids.push((old, new));
    }
    for &(old, new) in &kids {
        let src = source.get_dictionary(old)?;
        let mut dict = copier.copy_dict(src, &[b"Parent"]);
        for key in [&b"MediaBox"[..], b"CropBox", b"Resources", b"Rotate"] {
            if !dict.has(key)
                && let Some(v) = inherited(source, old, key)
            {
                let v = copier.copy(v);
                dict.set(key, v);
            }
        }
        dict.set("Parent", root_id);
        copier.out.insert(new, Object::Dictionary(dict));
    }

    let acroform = copy_acroform(source, &mut copier, &kids);

    let mut doc = lopdf::Document::with_version(source.version.clone());
    doc.objects = std::mem::take(&mut copier.out);
    doc.objects.insert(
        root_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids.iter().map(|&(_, new)| Object::from(new)).collect::<Vec<_>>(),
            "Count" => kids.len() as i64,
        }),
    );
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => root_id };
    if let Some(af) = acroform {
        catalog.set("AcroForm", af);
    }
    doc.objects.insert(catalog_id, Object::Dictionary(catalog));
    doc.max_id = copier.next_id - 1;
    doc.trailer.set("Root", catalog_id);
    let info = doc.add_object(dictionary! {
        "Producer" => Object::string_literal("FolioCraft"),
        "CreationDate" => Object::string_literal(crate::date::pdf_date_now()),
    });
    doc.trailer.set("Info", info);
    // Objekte, die nur über weggelassene Seiten erreichbar waren (z. B. Widgets eines
    // Optionsfelds auf anderen Seiten), fliegen raus.
    doc.prune_objects();
    doc.compress();

    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|e| CoreError::Other(format!("PDF schreiben: {e}")))?;
    Ok(bytes)
}

/// Übernimmt /AcroForm mit den Feldern, deren Widgets auf den kopierten Seiten liegen.
/// Die Widgets wurden mit den Seiten kopiert (/Annots), ihre Felder über /Parent mit.
/// Hier werden die Feldbäume auf die mitgekommenen Widgets beschnitten.
fn copy_acroform(
    source: &lopdf::Document,
    copier: &mut Copier,
    pages: &[(ObjectId, ObjectId)],
) -> Option<Object> {
    let acro = crate::form::import::acroform(source)?;
    let mut kept_widgets = HashSet::new();
    for &(_, new) in pages {
        if let Some(Object::Dictionary(d)) = copier.out.get(&new)
            && let Ok(Object::Array(annots)) = d.get(b"Annots")
        {
            kept_widgets.extend(annots.iter().filter_map(|a| a.as_reference().ok()));
        }
    }
    let top: Vec<ObjectId> = acro
        .get(b"Fields")
        .ok()
        .and_then(|f| source.dereference(f).ok())
        .and_then(|(_, f)| f.as_array().ok())
        .into_iter()
        .flatten()
        .filter_map(|f| f.as_reference().ok())
        .filter_map(|old| copier.mapped.get(&old).copied())
        .filter(|&new| prune_field(&mut copier.out, new, &kept_widgets, 0))
        .collect();
    if top.is_empty() {
        return None;
    }
    let mut af = dictionary! { "Fields" => top.into_iter().map(Object::from).collect::<Vec<_>>() };
    for key in [&b"DA"[..], b"DR", b"Q"] {
        if let Ok(v) = acro.get(key) {
            let v = copier.copy(v);
            af.set(key, v);
        }
    }
    Some(Object::Dictionary(af))
}

/// Entfernt aus einem kopierten Feldbaum alle Widgets, die nicht auf einer kopierten Seite
/// liegen. `true` = das Feld bleibt (hat noch mindestens ein Widget).
fn prune_field(
    out: &mut BTreeMap<ObjectId, Object>,
    id: ObjectId,
    kept: &HashSet<ObjectId>,
    depth: usize,
) -> bool {
    if depth > 32 {
        return false;
    }
    let kids: Option<Vec<Object>> = match out.get(&id) {
        Some(Object::Dictionary(d)) => match d.get(b"Kids") {
            Ok(Object::Array(k)) => Some(k.clone()),
            _ => None,
        },
        _ => return false,
    };
    let Some(kids) = kids else {
        return kept.contains(&id); // Blatt: Feld mit eigenem Widget
    };
    let remaining: Vec<Object> = kids
        .into_iter()
        .filter(|k| {
            k.as_reference()
                .is_ok_and(|kid| prune_field(out, kid, kept, depth + 1))
        })
        .collect();
    let keep = !remaining.is_empty();
    if let Some(Object::Dictionary(d)) = out.get_mut(&id) {
        d.set("Kids", remaining);
    }
    keep
}

/// Teilt `page_count` Seiten in Gruppen zu je `n` Seiten.
pub fn split_every(page_count: usize, n: usize) -> Vec<Vec<usize>> {
    let n = n.max(1);
    (0..page_count)
        .collect::<Vec<_>>()
        .chunks(n)
        .map(<[usize]>::to_vec)
        .collect()
}

/// Teilt nach Bereichen, getrennt durch Semikolon: "1-3; 4-6; 7-" -> drei Gruppen.
pub fn split_by_ranges(spec: &str, page_count: usize) -> Result<Vec<Vec<usize>>> {
    let groups: Vec<Vec<usize>> = spec
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| crate::page_import::parse_page_ranges(p, page_count))
        .collect::<Result<_>>()?;
    if groups.is_empty() {
        return Err(CoreError::Other("Keine Bereiche angegeben".into()));
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_helpers() {
        assert_eq!(split_every(5, 2), vec![vec![0, 1], vec![2, 3], vec![4]]);
        assert_eq!(split_every(2, 0), vec![vec![0], vec![1]]);
        assert_eq!(
            split_by_ranges("1-2; 3; 4-", 5).unwrap(),
            vec![vec![0, 1], vec![2], vec![3, 4]]
        );
        assert!(split_by_ranges("1-2; 9", 5).is_err());
    }
}
