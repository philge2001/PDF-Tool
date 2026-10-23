//! Lesezeichen (PDF-"Outlines"): das Inhaltsverzeichnis, das PDF-Programme in einer Seitenleiste
//! zeigen. Jedes Lesezeichen hat einen Titel und meist ein Ziel (Seite und Position), optional
//! Unterpunkte, eine Farbe und Fett-/Kursivschrift.
//!
//! Ziele stehen direkt im Eintrag (`/Dest`), in einer Aktion (`/A` mit `/S /GoTo`) oder als
//! Name, der über den Katalog (`/Dests` bzw. den Namensbaum `/Names /Dests`) aufgelöst wird.

use crate::document::Document;
use crate::pdf_string;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::{HashMap, HashSet};

/// Ziel eines Lesezeichens.
#[derive(Debug, Clone, PartialEq)]
pub struct Destination {
    /// Seitenindex (aktuelle Reihenfolge).
    pub page: usize,
    /// Obere Kante des Ausschnitts im PDF-Raum (falls angegeben).
    pub top: Option<f32>,
    pub left: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    pub dest: Option<Destination>,
    /// Weblink (Aktion /URI) statt eines Ziels im Dokument.
    pub uri: Option<String>,
    /// Beim Öffnen aufgeklappt (positives /Count)?
    pub open: bool,
    /// Textfarbe (RGB 0..1), falls nicht schwarz.
    pub color: Option<[f32; 3]>,
    pub bold: bool,
    pub italic: bool,
    pub children: Vec<OutlineItem>,
}

/// Höchstens so viele Einträge (Schutz vor kaputten oder böswilligen Dateien).
const MAX_ITEMS: usize = 20_000;

struct Reader<'a> {
    pdf: &'a lopdf::Document,
    pages: HashMap<ObjectId, usize>,
    seen: HashSet<ObjectId>,
}

impl Document {
    /// Liest die Lesezeichen (leer, wenn es keine gibt).
    pub fn outline(&self) -> Vec<OutlineItem> {
        let pdf = self.pdf();
        let Some(root) = pdf
            .catalog()
            .ok()
            .and_then(|c| c.get(b"Outlines").ok())
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
        else {
            return Vec::new();
        };
        let mut r = Reader {
            pdf,
            pages: self
                .pages()
                .iter()
                .enumerate()
                .map(|(i, p)| (p.object_id, i))
                .collect(),
            seen: HashSet::new(),
        };
        r.siblings(root.get(b"First").ok(), 0)
    }

    /// Gewünschte Ansicht beim Öffnen (/PageMode im Katalog), z. B. "UseOutlines" für
    /// "Lesezeichen zeigen"; leer, wenn nicht angegeben.
    pub fn page_mode(&self) -> String {
        self.pdf()
            .catalog()
            .ok()
            .and_then(|c| c.get(b"PageMode").ok())
            .and_then(|o| o.as_name().ok())
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default()
    }

    /// Hat das Dokument Lesezeichen?
    pub fn has_outline(&self) -> bool {
        !self.outline().is_empty()
    }
}

impl Reader<'_> {
    /// Liest eine Kette von Geschwistern (`/First` → `/Next` → …).
    fn siblings(&mut self, first: Option<&Object>, depth: usize) -> Vec<OutlineItem> {
        let mut out = Vec::new();
        let mut next = first.and_then(|o| o.as_reference().ok());
        while let Some(id) = next {
            // Zyklen und übergroße Bäume abfangen.
            if depth > 64 || self.seen.len() >= MAX_ITEMS || !self.seen.insert(id) {
                break;
            }
            let Ok(dict) = self.pdf.get_dictionary(id) else {
                break;
            };
            out.push(self.item(dict, depth));
            next = dict.get(b"Next").ok().and_then(|o| o.as_reference().ok());
        }
        out
    }

    fn item(&mut self, d: &Dictionary, depth: usize) -> OutlineItem {
        let mut dest = d.get(b"Dest").ok().and_then(|o| self.dest(o, 0));
        let mut uri = None;
        if let Some(action) = d
            .get(b"A")
            .ok()
            .and_then(|o| self.pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
        {
            match action.get(b"S").and_then(Object::as_name) {
                Ok(b"GoTo") => {
                    dest = dest.or_else(|| action.get(b"D").ok().and_then(|o| self.dest(o, 0)));
                }
                Ok(b"URI") => {
                    let u = pdf_string::get(action, b"URI");
                    uri = (!u.is_empty()).then_some(u);
                }
                _ => {}
            }
        }
        let color = d
            .get(b"C")
            .ok()
            .and_then(|o| o.as_array().ok())
            .filter(|a| a.len() == 3)
            .map(|a| {
                let f = |i: usize| a[i].as_float().unwrap_or(0.0).clamp(0.0, 1.0);
                [f(0), f(1), f(2)]
            })
            .filter(|c| c.iter().any(|v| *v > 0.0));
        let flags = d.get(b"F").and_then(Object::as_i64).unwrap_or(0);
        OutlineItem {
            title: pdf_string::get(d, b"Title"),
            dest,
            uri,
            open: d.get(b"Count").and_then(Object::as_i64).unwrap_or(0) > 0,
            color,
            bold: flags & 2 != 0,
            italic: flags & 1 != 0,
            children: self.siblings(d.get(b"First").ok(), depth + 1),
        }
    }

    /// Löst ein Ziel auf: Array, Name/String (benanntes Ziel) oder Dictionary mit /D.
    fn dest(&self, o: &Object, hops: usize) -> Option<Destination> {
        if hops > 8 {
            return None;
        }
        let (_, o) = self.pdf.dereference(o).ok()?;
        match o {
            Object::Array(a) => self.explicit(a),
            Object::Dictionary(d) => self.dest(d.get(b"D").ok()?, hops + 1),
            Object::Name(n) => self.dest(self.named(n)?, hops + 1),
            Object::String(s, _) => self.dest(self.named(s)?, hops + 1),
            _ => None,
        }
    }

    /// `[Seite /XYZ links oben zoom]`, `[Seite /FitH oben]`, `[Seite /FitR l u r o]` …
    fn explicit(&self, a: &[Object]) -> Option<Destination> {
        let page = *self.pages.get(&a.first()?.as_reference().ok()?)?;
        let num = |i: usize| a.get(i).and_then(|o| o.as_float().ok());
        let kind = a.get(1).and_then(|o| o.as_name().ok()).unwrap_or(b"Fit");
        let (left, top) = match kind {
            b"XYZ" => (num(2), num(3)),
            b"FitH" | b"FitBH" => (None, num(2)),
            b"FitV" | b"FitBV" => (num(2), None),
            b"FitR" => (num(2), num(5)),
            _ => (None, None),
        };
        Some(Destination { page, top, left })
    }

    /// Folgt einem Verweis. Eigene Methode statt Closure, damit Rust die Lebensdauern
    /// verknüpfen kann (Ergebnis lebt so lange wie Dokument bzw. Eingabe).
    fn deref<'a>(&'a self, o: &'a Object) -> Option<&'a Object> {
        self.pdf.dereference(o).ok().map(|(_, o)| o)
    }

    /// Benanntes Ziel: zuerst `/Dests` im Katalog (alte Form), dann der Namensbaum.
    fn named(&self, name: &[u8]) -> Option<&Object> {
        let cat = self.pdf.catalog().ok()?;
        if let Some(d) = cat
            .get(b"Dests")
            .ok()
            .and_then(|o| self.deref(o))
            .and_then(|o| o.as_dict().ok())
            && let Ok(v) = d.get(name)
        {
            return Some(v);
        }
        let tree = cat
            .get(b"Names")
            .ok()
            .and_then(|o| self.deref(o))
            .and_then(|o| o.as_dict().ok())?
            .get(b"Dests")
            .ok()
            .and_then(|o| self.deref(o))?
            .as_dict()
            .ok()?;
        self.name_tree(tree, name, 0)
    }

    fn name_tree<'a>(
        &'a self,
        node: &'a Dictionary,
        name: &[u8],
        depth: usize,
    ) -> Option<&'a Object> {
        if depth > 32 {
            return None;
        }
        if let Ok(names) = node.get(b"Names").and_then(Object::as_array) {
            for pair in names.chunks(2) {
                if let [Object::String(k, _), v] = pair
                    && k.as_slice() == name
                {
                    return Some(v);
                }
            }
        }
        let kids = node.get(b"Kids").and_then(Object::as_array).ok()?;
        kids.iter().find_map(|k| {
            let kid = self.pdf.dereference(k).ok()?.1.as_dict().ok()?;
            self.name_tree(kid, name, depth + 1)
        })
    }
}
