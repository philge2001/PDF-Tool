//! Unsichtbare Textebene für gescannte Seiten (Phase 7).
//!
//! Die Texterkennung liefert Wörter mit Rechtecken. Daraus wird Text im Darstellungsmodus 3
//! („unsichtbar“, `3 Tr`) – man sieht weiterhin das Bild, kann aber suchen, markieren und
//! kopieren. Jedes Wort steht in der eingebetteten Noto Sans, so gestaucht/gedehnt (`Tz`),
//! dass es genau die erkannte Breite hat; die Schriftgröße folgt der Höhe des Wortes.
//!
//! Die Ebene steht in `/FcOcr BMC … EMC`: Wird eine Seite erneut erkannt, ersetzt die neue
//! Ebene die alte. Alle Seiten eines Durchlaufs sind ein Undo-Schritt.

use crate::command::{Batch, Command};
use crate::content::Item;
use crate::document::Document;
use crate::embedded_font;
use crate::error::Result;
use crate::geometry::Point;
use crate::text_edit::{PageContent, ReplacePageContent, append_ops, fallback_name, isolate};
use lopdf::Object;
use lopdf::content::{Content, Operation};

const MARK: &[u8] = b"FcOcr";

/// Ein erkanntes Wort. Rechteck in Anzeige-Einheiten der Seite: 0..1, Ursprung oben links,
/// so wie die Seite (nach /Rotate) gerendert wurde.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrWord {
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// Entfernt einen vorhandenen `/FcOcr`-Abschnitt aus den Operationen.
fn remove_old_layer(ops: Vec<Operation>) -> (Vec<Operation>, bool) {
    let is_mark = |o: &Operation| {
        o.operator == "BMC"
            && o.operands
                .first()
                .and_then(|n| n.as_name().ok())
                .is_some_and(|n| n == MARK)
    };
    let mut out = Vec::with_capacity(ops.len());
    // Tiefe innerhalb des zu entfernenden Abschnitts (0 = außerhalb).
    let mut depth = 0usize;
    let mut removed = false;
    for op in ops {
        if depth == 0 && is_mark(&op) {
            depth = 1;
            removed = true;
            continue;
        }
        if depth > 0 {
            match op.operator.as_str() {
                "BMC" | "BDC" => depth += 1,
                "EMC" => depth -= 1,
                _ => {}
            }
            continue;
        }
        out.push(op);
    }
    (out, removed)
}

impl Document {
    /// Hat die Seite schon Text (sichtbar oder unsichtbar)? Für "nur Seiten ohne Text".
    pub fn page_has_text(&self, page_idx: usize) -> Result<bool> {
        let pc = self.page_content(page_idx)?;
        let page = self.page(page_idx)?.object_id;
        let it = self.interpret_content(page, &pc)?;
        Ok(it.items.iter().any(
            |i| matches!(i, Item::Text(t) if t.glyphs.iter().any(|g| !g.text.trim().is_empty())),
        ))
    }

    /// Legt für jede Seite eine unsichtbare Textebene an (ersetzt eine frühere). Ein
    /// Undo-Schritt für alle Seiten. Seiten ohne Wörter bleiben unverändert.
    pub fn add_ocr_layers(&mut self, pages: Vec<(usize, Vec<OcrWord>)>) -> Result<()> {
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        for (page_idx, words) in pages {
            let words: Vec<OcrWord> = words
                .into_iter()
                .map(|mut w| {
                    w.text = w
                        .text
                        .chars()
                        .filter(|&c| embedded_font::has_glyph(c))
                        .collect();
                    w
                })
                .filter(|w| !w.text.is_empty() && w.right > w.left && w.bottom > w.top)
                .collect();
            if words.is_empty() {
                continue;
            }
            let before = self.page_content(page_idx)?;
            let after = self.ocr_content(page_idx, &before, &words)?;
            let page = self.page(page_idx)?.object_id;
            commands.push(Box::new(ReplacePageContent::new(
                page,
                before,
                after,
                "Texterkennung",
            )));
        }
        if commands.is_empty() {
            return Ok(());
        }
        self.execute(Box::new(Batch::new("Texterkennung", commands)))
    }

    fn ocr_content(
        &self,
        page_idx: usize,
        before: &PageContent,
        words: &[OcrWord],
    ) -> Result<PageContent> {
        let info = self.page(page_idx)?.clone();
        let mut after = before.clone();
        let (ops, removed) = remove_old_layer(Content::decode(&after.content)?.operations);
        if removed {
            after.content = Content { operations: ops }.encode()?;
        }
        isolate(&mut after);
        let resources = crate::content::page_resources(self.pdf(), info.object_id);
        let name = fallback_name(&after, resources, self.pdf());
        let sub = after.fonts.entry(name.clone()).or_default();

        let (disp_w, disp_h) = info.display_size();
        // Text läuft in der Anzeige waagerecht: im PDF-Raum um die Seitendrehung gedreht.
        let (sin, cos) = f32::from(info.rotation).to_radians().sin_cos();
        let mut ops = vec![
            Operation::new("BMC", vec![Object::Name(MARK.to_vec())]),
            Operation::new("q", vec![]),
            Operation::new("BT", vec![]),
            Operation::new("Tr", vec![3.into()]),
        ];
        for w in words {
            let height = (w.bottom - w.top) * disp_h;
            let width = (w.right - w.left) * disp_w;
            // Schriftgröße ≈ Höhe des Wortes; Grundlinie etwas über der Unterkante
            // (Unterlängen wie bei "g" oder "p").
            let size = height.max(1.0);
            let baseline = w.bottom - 0.2 * (w.bottom - w.top);
            let origin = info.display_unit_to_pdf(Point::new(w.left, baseline));
            let natural = embedded_font::text_width(&w.text, size);
            let scale = if natural > 0.0 {
                width / natural * 100.0
            } else {
                100.0
            };
            ops.push(Operation::new(
                "Tf",
                vec![Object::Name(name.clone()), size.into()],
            ));
            ops.push(Operation::new("Tz", vec![scale.into()]));
            ops.push(Operation::new(
                "Tm",
                vec![
                    cos.into(),
                    sin.into(),
                    (-sin).into(),
                    cos.into(),
                    origin.x.into(),
                    origin.y.into(),
                ],
            ));
            ops.push(Operation::new("Tj", vec![sub.encode_object(&w.text)]));
        }
        ops.push(Operation::new("ET", vec![]));
        ops.push(Operation::new("Q", vec![]));
        ops.push(Operation::new("EMC", vec![]));
        append_ops(&mut after, ops)?;
        Ok(after)
    }
}
