//! Dialog "Texterkennung (OCR)": Seiten wählen, Sprachen, nur Seiten ohne Text.
//!
//! Ablauf: Der aktuelle Stand des Dokuments (inkl. ungespeicherter Änderungen) wird als
//! Momentaufnahme an einen Hintergrund-Job gegeben. Der Job lässt jede Seite vom Render-Thread
//! in 300 dpi rendern (`OffscreenRenderer`), erkennt den Text mit Tesseract und legt die Wörter
//! (in Anzeige-Einheiten) in `OcrSlot`. Nach dem Job übernimmt die UI sie als unsichtbare
//! Textebene – ein Undo-Schritt, danach funktionieren Suche, Markieren und Kopieren.
//!
//! Tesseract selbst: Desktop über die Crate `pdftool-ocr` (Feature `ocr`), im Browser über
//! tesseract.js (siehe [`web`]) – dieselbe Erkennung und dieselben Sprachdaten.

use crate::app::PdfToolApp;
use eframe::egui;
use pdftool_core::{OcrWord, parse_page_ranges};
use pdftool_render::{OffscreenRenderer, RgbaImage};
use std::sync::{Arc, Mutex};

/// Auflösung für die Erkennung (Tesseract arbeitet am besten um 300 dpi).
const DPI: f32 = 300.0;

/// Wörter unter dieser Sicherheit (0–100) sind meist Bildrauschen und werden verworfen.
const MIN_CONFIDENCE: f32 = 30.0;

/// Sprachen zur Auswahl (Tesseract-Code, Anzeigename). Desktop: eingebaut in `pdftool-ocr`,
/// Browser: `ocr/<code>.traineddata.gz` neben der Seite.
const LANGUAGES: &[(&str, &str)] = &[("deu", "Deutsch"), ("eng", "Englisch")];

/// Ist die Texterkennung hier verfügbar? Im Browser nur, wenn die Seite tesseract.js
/// eingebunden hat (Funktion `pdftoolOcr`, siehe PDF Webeditor `web/index.html`).
pub(crate) fn available() -> bool {
    #[cfg(feature = "ocr")]
    return true;
    #[cfg(not(feature = "ocr"))]
    return web::available();
}

/// Ein erkanntes Wort in Pixeln des gerenderten Bilds (Ursprung oben links).
struct RawWord {
    text: String,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    confidence: f32,
}

/// Pixel → Anzeige-Einheiten (0..1), so wie core sie erwartet; unsichere Wörter fallen weg.
fn to_ocr_words(words: Vec<RawWord>, image: &RgbaImage) -> Vec<OcrWord> {
    let (w, h) = (image.width as f32, image.height as f32);
    words
        .into_iter()
        .filter(|word| word.confidence >= MIN_CONFIDENCE)
        .map(|word| OcrWord {
            left: word.left as f32 / w,
            top: word.top as f32 / h,
            right: (word.left + word.width) as f32 / w,
            bottom: (word.top + word.height) as f32 / h,
            text: word.text,
        })
        .collect()
}

/// Was beide Varianten für den Job brauchen (alles besessen, damit es in den Job wandern kann).
struct OcrInput {
    renderer: OffscreenRenderer,
    snapshot: Arc<Vec<u8>>,
    /// Seitenindex und Anzeigegröße (nach /Rotate) – so rendert der Worker die Seite.
    pages: Vec<(usize, (f32, f32))>,
    /// z. B. "deu+eng".
    languages: String,
    out: OcrSlot,
}

impl OcrInput {
    fn render(&self, page: usize, size: (f32, f32)) -> Result<RgbaImage, String> {
        self.renderer
            .render_page(&self.snapshot, page as u32, size, DPI / 72.0)
            .map_err(|e| format!("Seite {} rendern: {e}", page + 1))
    }

    /// Ergebnis ablegen und die Meldung für den Nutzer bilden.
    fn finish(&self, result: Vec<(usize, Vec<OcrWord>)>) -> String {
        let total = result.len();
        let word_count: usize = result.iter().map(|(_, w)| w.len()).sum();
        if let Ok(mut s) = self.out.lock() {
            *s = Some(result);
        }
        format!(
            "{word_count} Wörter auf {total} Seite(n) erkannt.\n\
             Suchen, Markieren und Kopieren funktionieren jetzt auch dort.\n\n\
             Noch nicht gespeichert – zum Übernehmen speichern (Strg+S)."
        )
    }
}

fn page_progress(i: usize, total: usize, page: usize) -> (f32, String) {
    (
        i as f32 / total as f32,
        format!("Seite {} ({} von {total}) …", page + 1, i + 1),
    )
}

const CANCELLED: &str = "Texterkennung abgebrochen – nichts wurde geändert.";

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
            languages: LANGUAGES
                .iter()
                .map(|(code, _)| *code == "deu" || *code == "eng")
                .collect(),
            error: None,
        }
    }
}

/// Ergebnis des Jobs: erkannte Wörter je Seitenindex.
pub type OcrSlot = Arc<Mutex<Option<Vec<(usize, Vec<OcrWord>)>>>>;

/// RGBA -> Graustufen (Tesseract braucht keine Farbe; spart Speicher und Zeit).
#[cfg(feature = "ocr")]
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
                    for ((_, name), on) in LANGUAGES.iter().zip(&mut dlg.languages) {
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
        let languages: Vec<&str> = LANGUAGES
            .iter()
            .zip(&dlg.languages)
            .filter(|(_, on)| **on)
            .map(|((code, _), _)| *code)
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
        let input = OcrInput {
            renderer,
            snapshot,
            pages: sizes,
            languages,
            out: slot.clone(),
        };
        #[cfg(feature = "ocr")]
        let job = crate::jobs::Job::spawn("Texterkennung", ctx, move |c| run_tesseract(c, &input));
        #[cfg(not(feature = "ocr"))]
        let job = web::spawn(ctx, input);
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

/// Desktop: Tesseract über `pdftool-ocr`, im Job-Thread.
#[cfg(feature = "ocr")]
fn run_tesseract(c: &crate::jobs::JobCtx, input: &OcrInput) -> crate::jobs::JobResult {
    c.progress(0.0, "Tesseract wird gestartet …");
    let engine = pdftool_ocr::Engine::new(&input.languages).map_err(|e| e.to_string())?;
    let total = input.pages.len();
    let mut result = Vec::with_capacity(total);
    for (i, &(page, size)) in input.pages.iter().enumerate() {
        if c.cancelled() {
            return Err(CANCELLED.into());
        }
        let (fraction, text) = page_progress(i, total, page);
        c.progress(fraction, text);
        let image = input.render(page, size)?;
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
        let words = words
            .into_iter()
            .map(|w| RawWord {
                text: w.text,
                left: w.left,
                top: w.top,
                width: w.width,
                height: w.height,
                confidence: w.confidence,
            })
            .collect();
        result.push((page, to_ocr_words(words, &image)));
    }
    Ok(input.finish(result))
}

/// Liest Tesseracts TSV-Ausgabe: level, page, block, par, line, word, left, top, width,
/// height, conf, text. Wörter haben level 5. (Wie in `pdftool-ocr`; im Browser liefert
/// tesseract.js dieselbe Tabelle.)
#[cfg_attr(feature = "ocr", allow(dead_code))]
fn parse_tsv(tsv: &str) -> Vec<RawWord> {
    tsv.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(12, '\t').collect();
            if f.len() < 12 || f[0] != "5" {
                return None;
            }
            let n = |i: usize| f[i].trim().parse::<u32>().ok();
            let text = f[11].trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(RawWord {
                text,
                left: n(6)?,
                top: n(7)?,
                width: n(8)?,
                height: n(9)?,
                confidence: f[10].trim().parse().unwrap_or(0.0),
            })
        })
        .collect()
}

/// Browser: tesseract.js. Die Seite (PDF Webeditor) stellt die JavaScript-Funktion
/// `pdftoolOcr(rgba, breite, höhe, sprachen)` bereit; sie liefert ein Promise mit Tesseracts
/// TSV-Tabelle. tesseract.js rechnet in einem eigenen Web Worker – die Oberfläche bleibt
/// bedienbar. Das Rendern der Seite läuft dagegen kurz im UI-Thread (PDFium hat keinen Thread).
#[cfg(not(feature = "ocr"))]
mod web {
    use super::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    #[wasm_bindgen]
    extern "C" {
        /// `catch`: Wirft die Funktion (oder fehlt sie), kommt der Fehler als `Err` zurück
        /// statt die App abstürzen zu lassen. `rgba` ist eine Sicht in den WebAssembly-Speicher –
        /// die JavaScript-Seite muss die Pixel sofort kopieren (sie tut es mit `new ImageData`).
        #[wasm_bindgen(catch, js_namespace = window, js_name = pdftoolOcr)]
        fn pdftool_ocr(
            rgba: &[u8],
            width: u32,
            height: u32,
            languages: &str,
        ) -> Result<js_sys::Promise, JsValue>;
    }

    pub(super) fn available() -> bool {
        js_sys::Reflect::has(&js_sys::global(), &JsValue::from_str("pdftoolOcr")).unwrap_or(false)
    }

    /// Startet die Erkennung als Future; der Job zeigt Fortschritt und kann abbrechen
    /// (zwischen zwei Seiten – eine laufende Seite rechnet tesseract.js noch zu Ende).
    pub(super) fn spawn(ctx: &egui::Context, input: OcrInput) -> crate::jobs::Job {
        let (job, handle) = crate::jobs::Job::external("Texterkennung", ctx);
        wasm_bindgen_futures::spawn_local(async move {
            let result = run(handle.ctx(), &input).await;
            handle.finish(result);
        });
        job
    }

    async fn run(c: &crate::jobs::JobCtx, input: &OcrInput) -> crate::jobs::JobResult {
        let total = input.pages.len();
        let mut result = Vec::with_capacity(total);
        for (i, &(page, size)) in input.pages.iter().enumerate() {
            if c.cancelled() {
                return Err(CANCELLED.into());
            }
            let (fraction, mut text) = page_progress(i, total, page);
            if i == 0 {
                text.push_str("\n(Beim ersten Mal wird die Texterkennung geladen.)");
            }
            c.progress(fraction, text);
            // Einen Frame Luft lassen, damit das Fortschrittsfenster erscheint, bevor die Seite
            // im UI-Thread gerendert wird.
            yield_to_browser().await;
            let image = input.render(page, size)?;
            let promise = pdftool_ocr(&image.pixels, image.width, image.height, &input.languages)
                .map_err(js_error)?;
            let tsv = JsFuture::from(promise).await.map_err(js_error)?;
            let tsv = tsv
                .as_string()
                .ok_or_else(|| "Texterkennung: unerwartete Antwort".to_string())?;
            result.push((page, to_ocr_words(parse_tsv(&tsv), &image)));
        }
        Ok(input.finish(result))
    }

    /// Wartet bis zum nächsten Durchlauf der Ereignisschleife (setTimeout 0).
    async fn yield_to_browser() {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 16);
            }
        });
        let _ = JsFuture::from(promise).await;
    }

    fn js_error(e: JsValue) -> String {
        let msg = e
            .dyn_ref::<js_sys::Error>()
            .map(|e| String::from(e.message()))
            .or_else(|| e.as_string())
            .unwrap_or_else(|| format!("{e:?}"));
        format!("Texterkennung fehlgeschlagen: {msg}")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tsv_words() {
        let tsv = "1\t1\t0\t0\t0\t0\t0\t0\t100\t50\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t10\t20\t30\t12\t96.5\tHallo\n\
                   5\t1\t1\t1\t1\t2\t45\t20\t28\t12\t91\tWelt\n\
                   5\t1\t1\t1\t2\t1\t10\t40\t5\t12\t30\t \n";
        let words = super::parse_tsv(tsv);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Hallo");
        assert_eq!((words[1].left, words[1].width), (45, 28));
    }

    #[test]
    fn words_in_display_units_without_noise() {
        let image = pdftool_render::RgbaImage {
            width: 200,
            height: 100,
            pixels: Vec::new(),
        };
        let words = vec![
            super::RawWord {
                text: "Hallo".into(),
                left: 20,
                top: 10,
                width: 40,
                height: 20,
                confidence: 90.0,
            },
            super::RawWord {
                text: "~".into(),
                left: 0,
                top: 0,
                width: 5,
                height: 5,
                confidence: 12.0,
            },
        ];
        let w = super::to_ocr_words(words, &image);
        assert_eq!(w.len(), 1);
        assert_eq!(
            (w[0].left, w[0].top, w[0].right, w[0].bottom),
            (0.1, 0.1, 0.3, 0.3)
        );
    }

    /// Die Sprachliste des Dialogs passt zu den eingebauten Sprachdaten.
    #[cfg(feature = "ocr")]
    #[test]
    fn languages_match_tesseract_data() {
        let ours: Vec<&str> = super::LANGUAGES.iter().map(|(c, _)| *c).collect();
        let built_in: Vec<&str> = pdftool_ocr::LANGUAGES.iter().map(|(c, _, _)| *c).collect();
        assert_eq!(ours, built_in);
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn gray_weights() {
        assert_eq!(
            super::to_gray(&[255, 255, 255, 255, 0, 0, 0, 255]),
            vec![255, 0]
        );
        assert_eq!(super::to_gray(&[255, 0, 0, 255]), vec![76]);
    }
}
