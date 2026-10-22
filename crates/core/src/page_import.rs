//! Seiten einfügen: aus einer anderen PDF kopieren oder leere Seiten anlegen.
//!
//! Eine Seite hängt an vielen Objekten (Inhalt, Schriften, Bilder …). Diese werden per
//! Tiefenkopie mit neuen Objektnummern in unser Dokument übernommen. Bis zum Speichern
//! liegen sie im Modell (`Model::extra_objects`) und gehören zum `InsertPages`-Command –
//! Rückgängig entfernt sie wieder.

use crate::annotation::Annotation;
use crate::command::{Command, Model};
use crate::document::{Document, PageId, PageInfo, inherited};
use crate::error::{CoreError, Result};
use crate::geometry::Rect;
use lopdf::{Dictionary, Object, ObjectId, Stream, dictionary};
use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Fügt vorbereitete Seiten (samt ihrer Objekte und Annotationen) an einer Position ein.
#[derive(Debug)]
pub struct InsertPages {
    at: usize,
    pages: Vec<PageInfo>,
    objects: BTreeMap<ObjectId, Object>,
    /// Annotationen der neuen Seiten; `page` ist relativ (0 = erste eingefügte Seite).
    annotations: Vec<Annotation>,
    label: String,
}

impl Command for InsertPages {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn apply(&mut self, model: &mut Model) -> Result<()> {
        let at = self.at.min(model.pages.len());
        let count = self.pages.len();
        // Vorhandene Annotationen ab der Einfügestelle rücken nach hinten.
        model
            .annotations
            .remap_pages(|p| if p >= at { p + count } else { p });
        for (k, p) in self.pages.iter().enumerate() {
            model.pages.insert(at + k, p.clone());
        }
        for (&id, obj) in &self.objects {
            model.extra_objects.insert(id, obj.clone());
        }
        for a in &self.annotations {
            let mut a = a.clone();
            a.page += at;
            model.annotations.push(a);
        }
        self.at = at;
        Ok(())
    }

    fn undo(&mut self, model: &mut Model) -> Result<()> {
        let count = self.pages.len();
        let ids: HashSet<_> = self.annotations.iter().map(|a| a.id).collect();
        for id in ids {
            model.annotations.remove(id);
        }
        model.pages.drain(self.at..self.at + count);
        model
            .annotations
            .remap_pages(|p| if p >= self.at + count { p - count } else { p });
        for id in self.objects.keys() {
            model.extra_objects.remove(id);
        }
        Ok(())
    }

    fn label(&self) -> String {
        self.label.clone()
    }
}

/// Vergibt fortlaufend neue Objektnummern und kopiert Objekte aus einer Quell-PDF.
/// Wird auch von `page_export` genutzt.
pub(crate) struct Copier<'a> {
    pub source: &'a lopdf::Document,
    pub next_id: u32,
    /// alt -> neu (verhindert Mehrfachkopien und Endlosschleifen bei Zyklen)
    pub mapped: HashMap<ObjectId, ObjectId>,
    pub out: BTreeMap<ObjectId, Object>,
    /// Seiten der Quelle, die NICHT mitkopiert werden (Verweise darauf werden zu null).
    pub foreign_pages: HashSet<ObjectId>,
}

impl Copier<'_> {
    pub fn fresh_id(&mut self) -> ObjectId {
        let id = (self.next_id, 0);
        self.next_id += 1;
        id
    }

    /// Kopiert ein Objekt rekursiv und gibt die Kopie (mit umgeschriebenen Verweisen) zurück.
    pub fn copy(&mut self, obj: &Object) -> Object {
        match obj {
            Object::Reference(id) => match self.copy_ref(*id) {
                Some(new) => Object::Reference(new),
                None => Object::Null,
            },
            Object::Array(items) => Object::Array(items.iter().map(|o| self.copy(o)).collect()),
            Object::Dictionary(d) => Object::Dictionary(self.copy_dict(d, &[])),
            Object::Stream(s) => {
                let mut stream = Stream::new(self.copy_dict(&s.dict, &[]), s.content.clone());
                // Inhalt unverändert übernehmen (bereits kodiert), nicht erneut komprimieren.
                stream.allows_compression = false;
                Object::Stream(stream)
            }
            other => other.clone(),
        }
    }

    pub fn copy_dict(&mut self, d: &Dictionary, skip: &[&[u8]]) -> Dictionary {
        let mut out = Dictionary::new();
        for (k, v) in d.iter() {
            if !skip.contains(&k.as_slice()) {
                out.set(k.clone(), self.copy(v));
            }
        }
        out
    }

    fn copy_ref(&mut self, id: ObjectId) -> Option<ObjectId> {
        if let Some(&new) = self.mapped.get(&id) {
            return Some(new);
        }
        if self.foreign_pages.contains(&id) {
            return None;
        }
        let obj = self.source.get_object(id).ok()?.clone();
        let new = self.fresh_id();
        // Erst eintragen, dann rekursiv kopieren – so enden Zyklen (A -> B -> A) sauber.
        self.mapped.insert(id, new);
        let copied = self.copy(&obj);
        self.out.insert(new, copied);
        Some(new)
    }
}

impl Document {
    /// Bereitet das Einfügen von Seiten aus `source` vor. `pages` sind Indizes in `source`
    /// (in der gewünschten Reihenfolge), `at` die Einfügeposition in diesem Dokument.
    pub fn prepare_insert(
        &mut self,
        source: &Document,
        pages: &[usize],
        at: usize,
    ) -> Result<InsertPages> {
        if pages.is_empty() {
            return Err(CoreError::Other("Keine Seiten ausgewählt".into()));
        }
        let spdf = source.pdf();
        let wanted: HashSet<ObjectId> = pages
            .iter()
            .map(|&i| source.page(i).map(|p| p.object_id))
            .collect::<Result<_>>()?;
        let mut copier = Copier {
            source: spdf,
            next_id: self.next_free_object_id(),
            mapped: HashMap::new(),
            out: BTreeMap::new(),
            foreign_pages: source
                .pages()
                .iter()
                .map(|p| p.object_id)
                .filter(|id| !wanted.contains(id))
                .collect(),
        };

        // Neue Nummern für die Seiten vorab reservieren, damit Verweise zwischen den
        // kopierten Seiten (z. B. Links) auf die Kopien zeigen.
        let mut new_page_ids = Vec::new();
        for &i in pages {
            let old = source.page(i)?.object_id;
            let new = copier.mapped.get(&old).copied().unwrap_or_else(|| {
                let id = copier.fresh_id();
                copier.mapped.insert(old, id);
                id
            });
            new_page_ids.push(new);
        }

        let mut infos = Vec::new();
        let mut annotations = Vec::new();
        for (k, (&i, &new_id)) in pages.iter().zip(&new_page_ids).enumerate() {
            let info = source.page(i)?;
            let src_dict = spdf.get_dictionary(info.object_id)?;
            // Verwaltete Annotationen nicht als Objekt kopieren, sondern ins Modell übernehmen
            // (sofort bearbeitbar). Ihre Popups fallen mit weg.
            let managed: HashSet<ObjectId> = source
                .annotations()
                .on_page(i)
                .filter_map(|a| a.source.map(|s| s.object_id))
                .collect();
            let popups: HashSet<ObjectId> = source
                .annotations()
                .on_page(i)
                .filter_map(|a| a.source.and_then(|s| s.popup))
                .collect();

            let mut dict = copier.copy_dict(src_dict, &[b"Parent", b"Annots"]);
            // Geerbte Attribute direkt eintragen (die Quell-Zwischenknoten kommen nicht mit).
            for key in [&b"MediaBox"[..], b"CropBox", b"Resources"] {
                if !dict.has(key)
                    && let Some(v) = inherited(spdf, info.object_id, key)
                {
                    let v = copier.copy(v);
                    dict.set(key, v);
                }
            }
            dict.set("Rotate", info.rotation as i64);
            // Fremde Annotationen (Links, Formularfelder …) mitkopieren.
            let kept: Vec<Object> = crate::annotation::import::page_annots(spdf, info.object_id)
                .iter()
                .filter(|o| match o.as_reference() {
                    Ok(id) => !managed.contains(&id) && !popups.contains(&id),
                    Err(_) => true, // direkt eingebettete Annotation (selten) – mitnehmen
                })
                .map(|o| copier.copy(o))
                .filter(|o| !matches!(o, Object::Null))
                .collect();
            if !kept.is_empty() {
                dict.set("Annots", Object::Array(kept));
            }
            copier.out.insert(new_id, Object::Dictionary(dict));

            infos.push(PageInfo {
                id: PageId(0), // wird unten aus dem Modell vergeben
                object_id: new_id,
                crop_box: info.crop_box,
                rotation: info.rotation,
            });
            for a in source.annotations().on_page(i) {
                let mut a = a.clone();
                a.id = self.allocate_annotation_id();
                a.page = k;
                a.source = None; // wird beim Speichern neu geschrieben
                annotations.push(a);
            }
        }
        for info in &mut infos {
            info.id = self.allocate_page_id();
        }
        self.reserve_object_ids(copier.next_id);

        let n = infos.len();
        let label = if n == 1 {
            "Seite einfügen".into()
        } else {
            format!("{n} Seiten einfügen")
        };
        Ok(InsertPages {
            at,
            pages: infos,
            objects: copier.out,
            annotations,
            label,
        })
    }

    /// Bereitet eine leere Seite der Größe `size` (Punkt) an Position `at` vor.
    pub fn prepare_blank_page(&mut self, at: usize, size: (f32, f32)) -> InsertPages {
        let content_id = (self.next_free_object_id(), 0);
        let page_id = (content_id.0 + 1, 0);
        self.reserve_object_ids(page_id.0 + 1);
        let mut objects = BTreeMap::new();
        objects.insert(
            content_id,
            Object::Stream(Stream::new(Dictionary::new(), Vec::new())),
        );
        objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "MediaBox" => vec![0.into(), 0.into(), size.0.into(), size.1.into()],
                "Resources" => Dictionary::new(),
                "Contents" => content_id,
            }),
        );
        let info = PageInfo {
            id: self.allocate_page_id(),
            object_id: page_id,
            crop_box: Rect::new(0.0, 0.0, size.0, size.1),
            rotation: 0,
        };
        InsertPages {
            at,
            pages: vec![info],
            objects,
            annotations: Vec::new(),
            label: "Leere Seite einfügen".into(),
        }
    }
}

/// Liest Seitenbereiche wie "1-3, 5, 8-" (1-basiert, wie in der Oberfläche) und liefert
/// 0-basierte Indizes in der angegebenen Reihenfolge. "8-" heißt "ab 8 bis zum Ende".
pub fn parse_page_ranges(input: &str, page_count: usize) -> Result<Vec<usize>> {
    let err = |msg: String| CoreError::Other(msg);
    let mut out = Vec::new();
    for part in input
        .split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let num = |s: &str| -> Result<usize> {
            let n: usize = s
                .trim()
                .parse()
                .map_err(|_| err(format!("„{s}“ ist keine Seitenzahl")))?;
            if n == 0 || n > page_count {
                return Err(err(format!("Seite {n} gibt es nicht (1–{page_count})")));
            }
            Ok(n - 1)
        };
        match part.split_once('-') {
            Some((a, b)) => {
                let from = if a.trim().is_empty() { 0 } else { num(a)? };
                let to = if b.trim().is_empty() {
                    page_count - 1
                } else {
                    num(b)?
                };
                if from > to {
                    return Err(err(format!("Bereich „{part}“ ist rückwärts")));
                }
                out.extend(from..=to);
            }
            None => out.push(num(part)?),
        }
    }
    if out.is_empty() {
        return Err(err("Keine Seiten angegeben".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges() {
        assert_eq!(parse_page_ranges("1-3, 5", 10).unwrap(), vec![0, 1, 2, 4]);
        assert_eq!(parse_page_ranges("8-", 10).unwrap(), vec![7, 8, 9]);
        assert_eq!(parse_page_ranges("-2", 10).unwrap(), vec![0, 1]);
        assert_eq!(parse_page_ranges("3;1", 10).unwrap(), vec![2, 0]);
        assert!(parse_page_ranges("0", 10).is_err());
        assert!(parse_page_ranges("11", 10).is_err());
        assert!(parse_page_ranges("5-2", 10).is_err());
        assert!(parse_page_ranges("abc", 10).is_err());
        assert!(parse_page_ranges(" ", 10).is_err());
    }
}
