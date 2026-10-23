//! Dialog "Texterkennung (OCR)": Seiten wählen, Sprachen, nur Seiten ohne Text.
//!
//! Ablauf: Der aktuelle Stand des Dokuments (inkl. ungespeicherter Änderungen) wird als
//! Momentaufnahme an einen Hintergrund-Job gegeben. Der Job lässt jede Seite vom Render-Thread
//! in 300 dpi rendern (`OffscreenRenderer`), erkennt den Text mit Tesseract und legt die Wörter
//! (in Anzeige-Einheiten) in `OcrSlot`. Nach dem Job übernimmt die UI sie als unsichtbare
//! Textebene – ein Undo-Schritt, danach funktionieren Suche, Markieren und Kopieren.

use crate::app::PdfToolApp;
use eframe::egui;
use pdftool_core::{OcrWord, parse_page_ranges};
use std::sync::{Arc, Mutex};

/// Auflösung für die Erkennung (Tesseract arbeitet am besten um 300 dpi).
const DPI: f32 = 300.0;

pub struct OcrDialog {
    /// Seitenbereiche wie "1-3, 5"; leer = alle Seiten.
    ranges: String,
    only_without_text: bool,
    /// Je eingebaute Sprache: ausgewählt?
    languages: Vec<bool>,
    error: Option<String>,
}

impl Default for OcrDialog {
    fn default() -> Self {
        Self {
            ranges: String::new(),
            only_without_text: true,
            languages: pdftool_ocr::LANGUAGES
                .iter()
                .map(|(code, _, _)| *code == "deu" || *code == "eng")
                .collect(),
            error: None,
        }
    }
}

/// Ergebnis des Jobs: erkannte Wörter je Seitenindex.
pub type OcrSlot = Arc<Mutex<Option<Vec<(usize, Vec<OcrWord>)>>>>;

/// RGBA -> Graustufen (Tesseract braucht keine Farbe; spart Speicher und Zeit).
fn to_gray(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .map(|p| {
            ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000) as u8
        })
        .collect()
}

impl PdfToolApp {
    pub(crate) fn open_ocr_dialog(&mut self) {
        if self.document.is_some() {
            self.ocr_dialog = Some(OcrDialog::default());
        }
    }

    pub(crate) fn ocr_dialog_ui(&mut self, ctx: &egui::Context) {
        let (Some(dlg), Some(d)) = (&mut self.ocr_dialog, &self.document) else {
            return;
        };
        let n = d.doc.page_count();
        let mut open = true;
        let mut confirmed = false;
        egui::Window::new("Texterkennung (OCR)")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.label(
                    "Macht gescannte Seiten durchsuchbar: Der erkannte Text wird unsichtbar \
                          über das Bild gelegt (Suchen, Markieren, Kopieren).",
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Seiten:");
                    ui.add(
                        egui::TextEdit::singleline(&mut dlg.ranges)
                            .hint_text("alle")
                            .desired_width(140.0),
                    )
                    .on_hover_text(format!("z. B. 1-3, 5 – leer = alle {n} Seiten"));
                });
                ui.checkbox(&mut dlg.only_without_text, "Nur Seiten ohne Text")
                    .on_hover_text("Seiten, die schon Text enthalten, werden übersprungen");
                ui.horizontal(|ui| {
                    ui.label("Sprachen:");
                    for ((_, name, _), on) in pdftool_ocr::LANGUAGES.iter().zip(&mut dlg.languages)
                    {
                        ui.checkbox(on, *name);
                    }
                });
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.add_space(6.0);
                confirmed = ui.button("Text erkennen").clicked();
            });
        if !open {
            self.ocr_dialog = None;
            return;
        }
        if !confirmed {
            return;
        }

        // Seiten bestimmen.
        let mut pages = if dlg.ranges.trim().is_empty() {
            (0..n).collect()
        } else {
            match parse_page_ranges(&dlg.ranges, n) {
                Ok(p) => p,
                Err(e) => {
                    dlg.error = Some(e.to_string());
                    return;
                }
            }
        };
        if dlg.only_without_text {
            pages.retain(|&p| !d.doc.page_has_text(p).unwrap_or(false));
        }
        if pages.is_empty() {
            dlg.error =
                Some("Keine passenden Seiten – alle gewählten Seiten enthalten schon Text.".into());
            return;
        }
        let languages: Vec<&str> = pdftool_ocr::LANGUAGES
            .iter()
            .zip(&dlg.languages)
            .filter(|(_, on)| **on)
            .map(|((code, _, _), _)| *code)
            .collect();
        if languages.is_empty() {
            dlg.error = Some("Bitte mindestens eine Sprache wählen.".into());
            return;
        }
        let languages = languages.join("+");
        self.ocr_dialog = None;
        self.start_ocr(ctx, pages, languages);
    }

    fn start_ocr(&mut self, ctx: &egui::Context, pages: Vec<usize>, languages: String) {
        let Some(d) = &self.document else { return };
        if self.job.is_some() {
            return;
        }
        let renderer = match &self.render {
            Ok(r) => r.offscreen(),
            Err(e) => {
                self.error = Some(e.clone());
                return;
            }
        };
        let Some(renderer) = renderer else { return };
        let snapshot = match d.doc.render_snapshot() {
            Ok(s) => Arc::new(s.bytes),
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        // Anzeigegröße je Seite (nach /Rotate) – so rendert der Worker die Seite.
        let sizes: Vec<(usize, (f32, f32))> = pages
            .iter()
            .filter_map(|&p| Some((p, d.doc.page(p).ok()?.display_size())))
            .collect();
        let slot: OcrSlot = Arc::new(Mutex::new(None));
        let out = slot.clone();

        let job = crate::jobs::Job::spawn("Texterkennung", ctx, move |c| {
            c.progress(0.0, "Tesseract wird gestartet …");
            let engine = pdftool_ocr::Engine::new(&languages).map_err(|e| e.to_string())?;
            let total = sizes.len();
            let mut result = Vec::with_capacity(total);
            let mut word_count = 0;
            for (i, &(page, size)) in sizes.iter().enumerate() {
                if c.cancelled() {
                    return Err("Texterkennung abgebrochen – nichts wurde geändert.".into());
                }
                c.progress(
                    i as f32 / total as f32,
                    format!("Seite {} ({} von {total}) …", page + 1, i + 1),
                );
                let image = renderer
                    .render_page(&snapshot, page as u32, size, DPI / 72.0)
                    .map_err(|e| format!("Seite {} rendern: {e}", page + 1))?;
                let gray = to_gray(&image.pixels);
                let words = engine
                    .recognize(&pdftool_ocr::Image {
                        data: &gray,
                        width: image.width,
                        height: image.height,
                        bytes_per_pixel: 1,
                        dpi: DPI as u32,
                    })
                    .map_err(|e| format!("Seite {}: {e}", page + 1))?;
                // Pixel -> Anzeige-Einheiten (0..1), so wie core sie erwartet.
                let (w, h) = (image.width as f32, image.height as f32);
                let words: Vec<OcrWord> = words
                    .into_iter()
                    .filter(|word| word.confidence >= 30.0)
                    .map(|word| OcrWord {
                        left: word.left as f32 / w,
                        top: word.top as f32 / h,
                        right: (word.left + word.width) as f32 / w,
                        bottom: (word.top + word.height) as f32 / h,
                        text: word.text,
                    })
                    .collect();
                word_count += words.len();
                result.push((page, words));
            }
            if let Ok(mut s) = out.lock() {
                *s = Some(result);
            }
            Ok(format!(
                "{word_count} Wörter auf {total} Seite(n) erkannt.\n\
                 Suchen, Markieren und Kopieren funktionieren jetzt auch dort.\n\n\
                 Noch nicht gespeichert – zum Übernehmen speichern (Strg+S)."
            ))
        });
        self.pending_ocr = Some(slot);
        self.job = Some(job);
    }

    /// Nach dem Job: erkannte Wörter als Textebene übernehmen (ein Undo-Schritt).
    pub(crate) fn take_pending_ocr(&mut self) {
        let Some(slot) = self.pending_ocr.take() else {
            return;
        };
        let pages = slot.lock().ok().and_then(|mut s| s.take());
        let (Some(pages), Some(d)) = (pages, &mut self.document) else {
            return;
        };
        if let Err(e) = d.doc.add_ocr_layers(pages) {
            self.error = Some(format!("Textebene konnte nicht angelegt werden:\n{e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn gray_weights() {
        assert_eq!(
            super::to_gray(&[255, 255, 255, 255, 0, 0, 0, 255]),
            vec![255, 0]
        );
        assert_eq!(super::to_gray(&[255, 0, 0, 255]), vec![76]);
    }
}
