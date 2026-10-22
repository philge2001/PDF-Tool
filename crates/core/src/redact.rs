//! Schwärzungen anwenden: Inhalte in markierten Bereichen WIRKLICH entfernen.
//!
//! Ein schwarzes Rechteck allein reicht nicht – der Text darunter wäre weiter kopier- und
//! durchsuchbar. Deshalb wird pro Seite der Content Stream umgeschrieben:
//! - Text: betroffene Zeichen werden aus Tj/TJ entfernt und durch eine gleich breite
//!   Verschiebung ersetzt (der restliche Text bleibt, wo er war). Bei unsicheren
//!   Schriftbreiten fällt die ganze Textausgabe weg (lieber zu viel als zu wenig).
//!   Ersatztexte (/ActualText, /Alt, /E) betroffener Abschnitte werden gelöscht.
//! - Pfade im Bereich werden nicht mehr gezeichnet (reine Rechtecke bleiben).
//! - Bilder: Pixel im Bereich werden schwarz (auf einer Kopie). Nicht lesbare Formate:
//!   ganzes Bild entfernt.
//! - Formular-XObjects werden rekursiv genauso behandelt (als Kopie).
//! - Annotationen und Formularfelder im Bereich werden entfernt.
//! - Schwarze Rechtecke über jeden Bereich.
//!
//! Danach wird die Datei KOMPLETT neu geschrieben: nur noch erreichbare Objekte, keine alten
//! Revisionen – sonst stünde das Geschwärzte weiter in der Datei.

use crate::content::{Item, Matrix, TextShow, interpret, page_content_bytes, page_resources};
use crate::document::Document;
use crate::error::{CoreError, Result};
use crate::geometry::{Point, Rect};
use crate::raster::{Raster, normalize_inline};
use crate::{AnnotKind, FieldKind};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Maximale Verschachtelung von Formular-XObjects.
const MAX_DEPTH: usize = 8;

/// Zu schwärzende Bereiche je Seite (Objektnummer des Seiten-Dictionarys).
pub type RedactionAreas = BTreeMap<ObjectId, Vec<Rect>>;

/// Was beim Schwärzen passiert ist (für die Rückmeldung an den Nutzer).
#[derive(Debug, Default, Clone)]
pub struct RedactionReport {
    pub areas: usize,
    pub glyphs: usize,
    /// Textausgaben, die wegen unsicherer Schriftbreiten ganz entfernt wurden.
    pub whole_text_ops: usize,
    pub paths: usize,
    pub images_changed: usize,
    pub images_removed: usize,
    pub annotations: usize,
    pub fields: usize,
    pub actual_texts: usize,
    /// Hinweise (z. B. warum ein Bild ganz entfernt wurde).
    pub notes: Vec<String>,
}

impl RedactionReport {
    /// Kurze Zusammenfassung auf Deutsch.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("{} Bereich(e) geschwärzt", self.areas)];
        let mut add = |n: usize, what: &str| {
            if n > 0 {
                parts.push(format!("{n} {what}"));
            }
        };
        add(self.glyphs, "Zeichen entfernt");
        add(
            self.whole_text_ops,
            "Textblöcke vollständig entfernt (Schriftbreiten unsicher)",
        );
        add(self.paths, "Grafikelemente entfernt");
        add(self.images_changed, "Bilder geschwärzt");
        add(self.images_removed, "Bilder vollständig entfernt");
        add(self.annotations, "Kommentare/Annotationen entfernt");
        add(self.fields, "Formularfelder entfernt");
        add(self.actual_texts, "Ersatztexte entfernt");
        let mut s = parts.join("\n");
        for n in &self.notes {
            s.push_str("\nHinweis: ");
            s.push_str(n);
        }
        s
    }
}

impl Document {
    /// Markierte Schwärzungen pro Seitenindex (ein Rechteck pro Viereck).
    pub fn redaction_areas(&self) -> BTreeMap<usize, Vec<Rect>> {
        let mut out: BTreeMap<usize, Vec<Rect>> = BTreeMap::new();
        for a in self.annotations().iter() {
            if let AnnotKind::Redact { quads } = &a.kind {
                out.entry(a.page)
                    .or_default()
                    .extend(quads.iter().map(|q| q.bounds()));
            }
        }
        out
    }

    /// Enthält das Dokument (in der Datei) Signaturen? Schwärzen macht sie ungültig.
    pub fn has_signatures(&self) -> bool {
        let pdf = self.pdf();
        self.forms().iter().any(|f| {
            f.kind == FieldKind::Signature
                && f.source
                    .and_then(|id| pdf.get_dictionary(id).ok())
                    .is_some_and(|d| d.has(b"V"))
        })
    }

    /// Eingaben für [`redact_pdf`]: aktueller Stand als Bytes und Bereiche pro Seitenobjekt.
    /// Getrennt, damit das eigentliche Schwärzen in einem Hintergrund-Thread laufen kann.
    pub fn redaction_input(&self) -> Result<(Vec<u8>, RedactionAreas)> {
        let areas = self.redaction_areas();
        if areas.is_empty() {
            return Err(CoreError::Other(
                "Es sind keine Schwärzungen markiert.".into(),
            ));
        }
        let by_page = areas
            .into_iter()
            .filter_map(|(i, r)| Some((self.pages().get(i)?.object_id, r)))
            .collect();
        Ok((self.to_bytes()?, by_page))
    }
}

/// Schwärzt `bytes` in den Bereichen (pro Seitenobjekt) und schreibt die Datei komplett neu.
pub fn redact_pdf(
    bytes: &[u8],
    areas: &RedactionAreas,
) -> Result<(Vec<u8>, RedactionReport)> {
    let mut pdf = lopdf::Document::load_mem(bytes)?;
    let mut report = RedactionReport::default();
    let mut removed_widgets = HashSet::new();
    for (&page, rects) in areas {
        report.areas += rects.len();
        redact_page(&mut pdf, page, rects, &mut report)?;
        remove_annotations(&mut pdf, page, rects, &mut report, &mut removed_widgets)?;
    }
    // Übrige /Redact-Markierungen (falls ohne Bereich) ebenfalls entfernen.
    for page in pdf.get_pages().into_values().collect::<Vec<_>>() {
        remove_annotations(&mut pdf, page, &[], &mut report, &mut removed_widgets)?;
    }
    if !removed_widgets.is_empty() {
        report.fields += remove_fields(&mut pdf, &removed_widgets);
    }
    Ok((rewrite(pdf)?, report))
}

/// Schreibt das Dokument komplett neu (ohne alte Revisionen und unbenutzte Objekte).
pub fn rewrite(mut pdf: lopdf::Document) -> Result<Vec<u8>> {
    pdf.trailer.remove(b"Prev");
    pdf.trailer.remove(b"XRefStm");
    pdf.prune_objects();
    pdf.compress();
    let mut out = Vec::new();
    pdf.save_to(&mut out)
        .map_err(|e| CoreError::Other(format!("PDF schreiben: {e}")))?;
    Ok(out)
}

/// Zeichen gelten als geschwärzt, wenn sie mindestens zu diesem Anteil (in Breite und Höhe)
/// im Bereich liegen. Kaum berührte Nachbarzeichen (z. B. durch Kerning) bleiben – sie sind
/// ohnehin außerhalb des schwarzen Kastens sichtbar.
const GLYPH_OVERLAP: f32 = 0.4;

fn glyph_hits(r: &Rect, areas: &[Rect]) -> bool {
    areas.iter().any(|a| {
        let w = r.x1.min(a.x1) - r.x0.max(a.x0);
        let h = r.y1.min(a.y1) - r.y0.max(a.y0);
        w > 0.01
            && h > 0.01
            && w >= GLYPH_OVERLAP * r.width().max(0.01)
            && h >= GLYPH_OVERLAP * r.height().max(0.01)
    })
}

/// Echte Überlappung (nicht nur Berührung an der Kante).
fn hits(r: &Rect, areas: &[Rect]) -> bool {
    areas.iter().any(|a| {
        let w = r.x1.min(a.x1) - r.x0.max(a.x0);
        let h = r.y1.min(a.y1) - r.y0.max(a.y0);
        w > 0.01 && h > 0.01
    })
}

fn redact_page(
    pdf: &mut lopdf::Document,
    page: ObjectId,
    areas: &[Rect],
    report: &mut RedactionReport,
) -> Result<()> {
    let resources = page_resources(pdf, page).cloned().unwrap_or_default();
    let bytes = page_content_bytes(pdf, page)?;
    let ops = Content::decode(&bytes)?.operations;
    let (ops, resources) = redact_ops(pdf, ops, resources, Matrix::IDENTITY, areas, report, 0)?;

    // Original in q/Q einschließen, damit die schwarzen Kästen unabhängig vom Grafikzustand
    // am Ende des Streams (Transformation, Farbe …) genau dort landen, wo sie hingehören.
    let mut all = Vec::with_capacity(ops.len() + 4 + areas.len());
    all.push(Operation::new("q", vec![]));
    all.extend(ops);
    all.push(Operation::new("Q", vec![]));
    all.push(Operation::new("q", vec![]));
    all.push(Operation::new("rg", vec![0.into(), 0.into(), 0.into()]));
    for r in areas {
        all.push(Operation::new(
            "re",
            vec![
                r.x0.into(),
                r.y0.into(),
                r.width().into(),
                r.height().into(),
            ],
        ));
    }
    all.push(Operation::new("f", vec![]));
    all.push(Operation::new("Q", vec![]));
    let mut stream = Stream::new(Dictionary::new(), Content { operations: all }.encode()?);
    let _ = stream.compress();
    let content_id = pdf.add_object(stream);
    let dict = pdf.get_object_mut(page).and_then(Object::as_dict_mut)?;
    dict.set("Contents", content_id);
    dict.set("Resources", resources);
    Ok(())
}

/// Was an einer Operation zu tun ist.
enum Edit {
    /// Zeichen entfernen (Position im String) – mit den Daten der Textausgabe.
    Glyphs(TextShow, HashSet<(usize, usize)>),
    /// Ganze Textausgabe entfernen (unsichere Breiten).
    WholeText,
    /// Pfad nicht zeichnen (Malbefehl -> `n`, Clip bleibt).
    NoPaint,
    /// Operation weglassen (z. B. nicht schwärzbares Bild).
    Drop,
    /// XObject-Name ersetzen (geschwärzte Kopie).
    Rename(Vec<u8>),
    /// Inline-Bild ersetzen.
    Inline(Stream),
}

#[allow(clippy::too_many_arguments)]
fn redact_ops(
    pdf: &mut lopdf::Document,
    ops: Vec<Operation>,
    mut resources: Dictionary,
    ctm: Matrix,
    areas: &[Rect],
    report: &mut RedactionReport,
    depth: usize,
) -> Result<(Vec<Operation>, Dictionary)> {
    // Phase 1: interpretieren und planen (nur lesend auf `pdf`).
    let interp = interpret(pdf, ops, Some(&resources), ctm);
    let mut edits: HashMap<usize, Edit> = HashMap::new();
    let mut images = Vec::new();
    let mut forms = Vec::new();
    for item in &interp.items {
        match item {
            Item::Text(t) => {
                if !t.exact {
                    if hits(&t.bounds, areas) {
                        edits.insert(t.op, Edit::WholeText);
                        report.whole_text_ops += 1;
                    }
                    continue;
                }
                let gone: HashSet<(usize, usize)> = t
                    .glyphs
                    .iter()
                    .filter(|g| glyph_hits(&g.rect, areas))
                    .map(|g| (g.at.part, g.at.start))
                    .collect();
                if !gone.is_empty() {
                    report.glyphs += gone.len();
                    edits.insert(t.op, Edit::Glyphs(t.clone(), gone));
                }
            }
            Item::Path {
                start,
                paint,
                bounds,
                painted,
            } => {
                let only_rects = interp.ops[*start..*paint]
                    .iter()
                    .all(|o| matches!(o.operator.as_str(), "re" | "W" | "W*"));
                if *painted && !only_rects && hits(bounds, areas) {
                    edits.insert(*paint, Edit::NoPaint);
                    report.paths += 1;
                }
            }
            Item::Image {
                op,
                name,
                matrix,
                bounds,
            } if hits(bounds, areas) => images.push((*op, name.clone(), *matrix)),
            Item::Form {
                op,
                name,
                matrix,
                bounds,
                ..
            } if hits(bounds, areas) => forms.push((*op, name.clone(), *matrix)),
            _ => {}
        }
    }
    let mut ops = interp.ops;

    // Phase 2: Bilder und Formulare als geschwärzte Kopien anlegen.
    let mut xobjects: Dictionary = resources
        .get(b"XObject")
        .ok()
        .and_then(|o| pdf.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    let mut counter = 0;
    // Ersetzte Originale: werden aus den Ressourcen entfernt, sobald sie nicht mehr benutzt werden.
    let mut replaced: Vec<Vec<u8>> = Vec::new();
    let mut fresh_name = |xobjects: &Dictionary| loop {
        counter += 1;
        let n = format!("FcRed{counter}").into_bytes();
        if !xobjects.has(&n) {
            break n;
        }
    };

    for (op, name, matrix) in images {
        let result = match &name {
            Some(n) => xobject_stream(pdf, &xobjects, n).ok_or_else(|| "Bild fehlt".to_string()),
            None => match ops[op].operands.first() {
                Some(Object::Stream(s)) => {
                    let mut s = s.clone();
                    s.dict = normalize_inline(&s.dict);
                    Ok(s)
                }
                _ => Err("Inline-Bild unlesbar".into()),
            },
        }
        .and_then(|stream| blacken_image(pdf, &stream, matrix, areas));
        match (result, name) {
            (Ok(new), Some(old)) => {
                replaced.push(old);
                let id = pdf.add_object(new);
                let n = fresh_name(&xobjects);
                xobjects.set(n.clone(), id);
                edits.insert(op, Edit::Rename(n));
                report.images_changed += 1;
            }
            (Ok(new), None) => {
                edits.insert(op, Edit::Inline(abbreviate_inline(new)));
                report.images_changed += 1;
            }
            (Err(e), old) => {
                replaced.extend(old);
                edits.insert(op, Edit::Drop);
                report.images_removed += 1;
                report
                    .notes
                    .push(format!("{e} – Bild vollständig entfernt"));
            }
        }
    }

    for (op, name, matrix) in forms {
        let Some(form) = xobject_stream(pdf, &xobjects, &name) else {
            continue;
        };
        if depth >= MAX_DEPTH {
            edits.insert(op, Edit::Drop);
            report
                .notes
                .push("Zu tief verschachtelte Formulare entfernt".into());
            continue;
        }
        let form_res = form
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .cloned()
            .unwrap_or_else(|| resources.clone());
        let data = form
            .decompressed_content()
            .unwrap_or_else(|_| form.content.clone());
        let form_ops = Content::decode(&data)?.operations;
        let (new_ops, new_res) =
            redact_ops(pdf, form_ops, form_res, matrix, areas, report, depth + 1)?;
        let mut dict = form.dict.clone();
        dict.remove(b"Filter");
        dict.remove(b"DecodeParms");
        dict.remove(b"Length");
        dict.set("Resources", new_res);
        let mut s = Stream::new(
            dict,
            Content {
                operations: new_ops,
            }
            .encode()?,
        );
        let _ = s.compress();
        let id = pdf.add_object(s);
        let n = fresh_name(&xobjects);
        xobjects.set(n.clone(), id);
        edits.insert(op, Edit::Rename(n));
        replaced.push(name);
    }

    // Phase 3: Operationen neu zusammensetzen.
    let mut out: Vec<Operation> = Vec::with_capacity(ops.len());
    // Offene Markierungsabschnitte (BDC/BMC) als Index in `out`, und ob darin etwas entfernt wurde.
    let mut marked: Vec<(usize, bool)> = Vec::new();
    let mut dirty_marks: Vec<usize> = Vec::new();
    for (i, op) in ops.drain(..).enumerate() {
        match op.operator.as_str() {
            "BDC" | "BMC" => marked.push((out.len(), false)),
            "EMC" => {
                if let Some((idx, true)) = marked.pop() {
                    dirty_marks.push(idx);
                }
            }
            _ => {}
        }
        let Some(edit) = edits.remove(&i) else {
            out.push(op);
            continue;
        };
        match edit {
            Edit::Glyphs(show, gone) => {
                out.extend(rewrite_show(&op, &show, &gone));
                mark_dirty(&mut marked);
            }
            Edit::WholeText => {
                // Zeilenwechsel von ' und " erhalten, den Text selbst nicht.
                if op.operator == "\"" {
                    if let Some(aw) = op.operands.first() {
                        out.push(Operation::new("Tw", vec![aw.clone()]));
                    }
                    if let Some(ac) = op.operands.get(1) {
                        out.push(Operation::new("Tc", vec![ac.clone()]));
                    }
                }
                if op.operator == "'" || op.operator == "\"" {
                    out.push(Operation::new("T*", vec![]));
                }
                mark_dirty(&mut marked);
            }
            Edit::NoPaint => out.push(Operation::new("n", vec![])),
            Edit::Drop => {}
            Edit::Rename(n) => out.push(Operation::new("Do", vec![Object::Name(n)])),
            Edit::Inline(s) => out.push(Operation::new("BI", vec![Object::Stream(s)])),
        }
    }
    // Ersetzte Originale aus den Ressourcen nehmen, wenn kein `Do` sie mehr benutzt – sonst
    // blieben sie über die Ressourcen erreichbar und stünden weiter in der Datei.
    let used: HashSet<&[u8]> = out
        .iter()
        .filter(|o| o.operator == "Do")
        .filter_map(|o| o.operands.first()?.as_name().ok())
        .collect();
    for name in &replaced {
        if !used.contains(name.as_slice()) {
            xobjects.remove(name);
        }
    }
    if !xobjects.is_empty() || !replaced.is_empty() {
        resources.set("XObject", xobjects);
    }
    // Nicht geschlossene Abschnitte am Ende zählen auch.
    dirty_marks.extend(marked.iter().filter(|(_, d)| *d).map(|(i, _)| *i));
    for idx in dirty_marks {
        if let Some(Object::Dictionary(props)) = out[idx].operands.get_mut(1) {
            for key in [&b"ActualText"[..], b"Alt", b"E"] {
                if props.remove(key).is_some() {
                    report.actual_texts += 1;
                }
            }
        }
    }
    Ok((out, resources))
}

fn mark_dirty(marked: &mut [(usize, bool)]) {
    for m in marked.iter_mut() {
        m.1 = true;
    }
}

fn xobject_stream(pdf: &lopdf::Document, xobjects: &Dictionary, name: &[u8]) -> Option<Stream> {
    let o = xobjects.get(name).ok()?;
    pdf.dereference(o).ok()?.1.as_stream().ok().cloned()
}

/// Schwärzt die Pixel eines Bildes, die in den Bereichen liegen. `matrix` bildet das
/// Einheitsquadrat des Bildes auf die Seite ab.
fn blacken_image(
    pdf: &lopdf::Document,
    stream: &Stream,
    matrix: Matrix,
    areas: &[Rect],
) -> std::result::Result<Stream, String> {
    let mut raster = Raster::decode(pdf, stream)?;
    let inv = matrix.invert().ok_or("Bild ohne Fläche")?;
    let (w, h) = (raster.width as f32, raster.height as f32);
    for a in areas {
        let corners = [
            Point::new(a.x0, a.y0),
            Point::new(a.x1, a.y0),
            Point::new(a.x0, a.y1),
            Point::new(a.x1, a.y1),
        ]
        .map(|p| inv.apply(p));
        let Some(u) = Rect::bounding(corners) else {
            continue;
        };
        // Einheitsquadrat: u nach rechts, v nach oben; Bildzeile 0 ist OBEN (v = 1).
        let (u0, u1) = (u.x0.clamp(0.0, 1.0), u.x1.clamp(0.0, 1.0));
        let (v0, v1) = (u.y0.clamp(0.0, 1.0), u.y1.clamp(0.0, 1.0));
        if u1 <= u0 || v1 <= v0 {
            continue;
        }
        // Großzügig runden: lieber ein Pixel mehr schwärzen.
        let x0 = (u0 * w).floor() as usize;
        let x1 = (u1 * w).ceil() as usize;
        let y0 = ((1.0 - v1) * h).floor() as usize;
        let y1 = ((1.0 - v0) * h).ceil() as usize;
        raster.blacken(x0, y0, x1, y1);
    }
    Ok(raster.encode(&stream.dict))
}

/// Inline-Bilder: lange Schlüssel/Namen wieder abkürzen (so verlangt es der Standard für BI).
fn abbreviate_inline(s: Stream) -> Stream {
    let mut d = Dictionary::new();
    for (k, v) in s.dict.iter() {
        let key: &[u8] = match k.as_slice() {
            b"Width" => b"W",
            b"Height" => b"H",
            b"ColorSpace" => b"CS",
            b"BitsPerComponent" => b"BPC",
            b"Filter" => b"F",
            b"DecodeParms" => b"DP",
            b"ImageMask" => b"IM",
            b"Decode" => b"D",
            b"Length" => continue,
            other => other,
        };
        let short = |n: &[u8]| -> Vec<u8> {
            match n {
                b"DeviceGray" => b"G".to_vec(),
                b"DeviceRGB" => b"RGB".to_vec(),
                b"DeviceCMYK" => b"CMYK".to_vec(),
                b"Indexed" => b"I".to_vec(),
                b"FlateDecode" => b"Fl".to_vec(),
                other => other.to_vec(),
            }
        };
        let value = match v {
            Object::Name(n) => Object::Name(short(n)),
            o => o.clone(),
        };
        d.set(key, value);
    }
    Stream::new(d, s.content).with_compression(false)
}

/// Baut eine Textausgabe ohne die entfernten Zeichen. Jedes entfernte Zeichen wird durch eine
/// TJ-Verschiebung gleicher Breite ersetzt, damit der Rest an seiner Stelle bleibt.
fn rewrite_show(op: &Operation, show: &TextShow, gone: &HashSet<(usize, usize)>) -> Vec<Operation> {
    let parts: Vec<Object> = match op.operator.as_str() {
        "TJ" => op
            .operands
            .first()
            .and_then(|o| o.as_array().ok())
            .cloned()
            .unwrap_or_default(),
        "\"" => op.operands.get(2).cloned().into_iter().collect(),
        _ => op.operands.first().cloned().into_iter().collect(),
    };
    let size = show.font_size;
    let mut out: Vec<Object> = Vec::new();
    // TJ-Zahl für einen Vorschub (in Textraum-Einheiten): -Vorschub / Größe × 1000.
    let shift = |advance: f32| -> f32 {
        if size.abs() > f32::EPSILON {
            -advance / size * 1000.0
        } else {
            0.0
        }
    };
    let push_num = |out: &mut Vec<Object>, n: f32| {
        if let Some(Object::Real(prev)) = out.last_mut() {
            *prev += n;
        } else {
            out.push(Object::Real(n));
        }
    };
    for (pi, part) in parts.into_iter().enumerate() {
        match part {
            Object::String(bytes, fmt) => {
                let mut cur: Vec<u8> = Vec::new();
                for g in show.glyphs.iter().filter(|g| g.at.part == pi) {
                    let piece = &bytes[g.at.start..g.at.start + g.at.len];
                    if gone.contains(&(pi, g.at.start)) {
                        if !cur.is_empty() {
                            out.push(Object::String(std::mem::take(&mut cur), fmt));
                        }
                        push_num(&mut out, shift(g.advance));
                    } else {
                        cur.extend_from_slice(piece);
                    }
                }
                if !cur.is_empty() {
                    out.push(Object::String(cur, fmt));
                }
            }
            other => {
                let n = other.as_float().unwrap_or(0.0);
                push_num(&mut out, n);
            }
        }
    }
    let mut result = Vec::new();
    if op.operator == "\"" {
        if let Some(aw) = op.operands.first() {
            result.push(Operation::new("Tw", vec![aw.clone()]));
        }
        if let Some(ac) = op.operands.get(1) {
            result.push(Operation::new("Tc", vec![ac.clone()]));
        }
    }
    if op.operator == "'" || op.operator == "\"" {
        result.push(Operation::new("T*", vec![]));
    }
    result.push(Operation::new("TJ", vec![Object::Array(out)]));
    result
}

/// Entfernt Annotationen, die einen Bereich berühren, und alle /Redact-Markierungen.
fn remove_annotations(
    pdf: &mut lopdf::Document,
    page: ObjectId,
    areas: &[Rect],
    report: &mut RedactionReport,
    widgets: &mut HashSet<ObjectId>,
) -> Result<()> {
    let annots: Vec<Object> = match pdf.get_dictionary(page)?.get(b"Annots") {
        Ok(o) => match pdf.dereference(o) {
            Ok((_, Object::Array(a))) => a.clone(),
            _ => return Ok(()),
        },
        Err(_) => return Ok(()),
    };
    let mut removed = HashSet::new();
    for o in &annots {
        let Ok(id) = o.as_reference() else { continue };
        let Ok(d) = pdf.get_dictionary(id) else {
            continue;
        };
        let subtype = d
            .get(b"Subtype")
            .and_then(Object::as_name)
            .unwrap_or_default();
        let rect = d
            .get(b"Rect")
            .ok()
            .and_then(|r| r.as_array().ok())
            .map(|a| {
                a.iter()
                    .filter_map(|o| o.as_float().ok())
                    .collect::<Vec<_>>()
            })
            .filter(|v| v.len() == 4)
            .map(|v| Rect::new(v[0], v[1], v[2], v[3]).normalized());
        let in_area = rect.is_some_and(|r| hits(&r, areas));
        if subtype == b"Redact" || in_area {
            removed.insert(id);
            if subtype == b"Widget" {
                widgets.insert(id);
            }
            if let Ok(popup) = d.get(b"Popup").and_then(Object::as_reference) {
                removed.insert(popup);
            }
            if subtype != b"Redact" {
                report.annotations += 1;
            }
        }
    }
    if removed.is_empty() {
        return Ok(());
    }
    // Popups, deren Elternteil entfernt wurde, ebenfalls.
    let kept: Vec<Object> = annots
        .into_iter()
        .filter(|o| {
            let Ok(id) = o.as_reference() else {
                return true;
            };
            if removed.contains(&id) {
                return false;
            }
            let parent = pdf
                .get_dictionary(id)
                .ok()
                .and_then(|d| d.get(b"Parent").ok())
                .and_then(|p| p.as_reference().ok());
            !parent.is_some_and(|p| removed.contains(&p))
        })
        .collect();
    pdf.get_object_mut(page)
        .and_then(Object::as_dict_mut)?
        .set("Annots", kept);
    Ok(())
}

/// Entfernt Felder mit den gegebenen Widgets aus /AcroForm (samt leer gewordener Eltern).
/// Liefert die Zahl entfernter Feldknoten.
fn remove_fields(pdf: &mut lopdf::Document, widgets: &HashSet<ObjectId>) -> usize {
    // `true` = Knoten bleibt. Rekursiv durch /Kids.
    fn prune(
        pdf: &mut lopdf::Document,
        id: ObjectId,
        widgets: &HashSet<ObjectId>,
        removed: &mut usize,
        depth: usize,
    ) -> bool {
        if widgets.contains(&id) || depth > 32 {
            *removed += 1;
            return false;
        }
        let kids = match pdf
            .get_dictionary(id)
            .ok()
            .and_then(|d| d.get(b"Kids").ok())
        {
            Some(Object::Array(k)) => k.clone(),
            _ => return true,
        };
        let kept: Vec<Object> = kids
            .into_iter()
            .filter(|k| {
                k.as_reference()
                    .map_or(true, |kid| prune(pdf, kid, widgets, removed, depth + 1))
            })
            .collect();
        let empty = kept.is_empty();
        if let Ok(d) = pdf.get_object_mut(id).and_then(Object::as_dict_mut) {
            d.set("Kids", kept);
        }
        if empty {
            *removed += 1;
        }
        !empty
    }

    let Ok(root) = pdf.trailer.get(b"Root").and_then(Object::as_reference) else {
        return 0;
    };
    // /AcroForm ist ein eigenes Objekt oder steht direkt im Katalog.
    let acro_ref = pdf
        .get_dictionary(root)
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|o| o.as_reference().ok());
    let fields: Vec<Object> = {
        let acro = match acro_ref {
            Some(id) => pdf.get_dictionary(id).ok(),
            None => pdf
                .get_dictionary(root)
                .ok()
                .and_then(|c| c.get(b"AcroForm").ok())
                .and_then(|a| a.as_dict().ok()),
        };
        match acro.and_then(|a| a.get(b"Fields").ok()) {
            Some(Object::Array(f)) => f.clone(),
            _ => return 0,
        }
    };
    let mut removed = 0;
    let kept: Vec<Object> = fields
        .into_iter()
        .filter(|f| {
            f.as_reference()
                .map_or(true, |id| prune(pdf, id, widgets, &mut removed, 0))
        })
        .collect();
    let target = match acro_ref {
        Some(id) => pdf.get_object_mut(id).and_then(Object::as_dict_mut).ok(),
        None => pdf
            .get_object_mut(root)
            .and_then(Object::as_dict_mut)
            .ok()
            .and_then(|c| c.get_mut(b"AcroForm").ok())
            .and_then(|a| a.as_dict_mut().ok()),
    };
    if let Some(af) = target {
        af.set("Fields", kept);
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_needs_area() {
        let a = [Rect::new(0.0, 0.0, 10.0, 10.0)];
        assert!(hits(&Rect::new(5.0, 5.0, 15.0, 15.0), &a));
        assert!(!hits(&Rect::new(10.0, 0.0, 20.0, 10.0), &a), "nur Kante");
    }
}
