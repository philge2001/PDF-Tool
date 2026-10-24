//! Export: Seiten als Bilder (PNG/JPEG, Auflösung wählbar) oder den Text als TXT-Datei.
//!
//! Exportiert wird der aktuelle Stand (Momentaufnahme inkl. ungespeicherter Änderungen).
//! Das Rendern übernimmt der Render-Thread (`OffscreenRenderer`), der Rest läuft als Job.

use crate::app::PdfToolApp;
use eframe::egui;
use pdftool_core::image_codec::{self, PixelFormat};
use pdftool_core::parse_page_ranges;
use pdftool_render::OffscreenRenderer;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Jpeg,
    Text,
    Docx,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 4] = [
        ExportFormat::Png,
        ExportFormat::Jpeg,
        ExportFormat::Text,
        ExportFormat::Docx,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Png => "PNG-Bilder (verlustfrei)",
            ExportFormat::Jpeg => "JPEG-Bilder (klein)",
            ExportFormat::Text => "Text (TXT)",
            ExportFormat::Docx => "Word (DOCX, einfach)",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Text => "txt",
            ExportFormat::Docx => "docx",
        }
    }

    /// Eine Datei pro Seite (Bilder) oder eine Datei fürs Ganze?
    fn per_page(self) -> bool {
        matches!(self, ExportFormat::Png | ExportFormat::Jpeg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub dpi: u32,
    /// JPEG-Qualität 1–100.
    pub quality: u8,
}

pub struct ExportDialog {
    ranges: String,
    options: ExportOptions,
    error: Option<String>,
}

impl Default for ExportDialog {
    fn default() -> Self {
        Self {
            ranges: String::new(),
            options: ExportOptions {
                format: ExportFormat::Png,
                dpi: 150,
                quality: 85,
            },
            error: None,
        }
    }
}

/// Dateiname einer Seite: "<stamm>-Seite-007.png" (Nullen nach Seitenzahl, damit die
/// Dateien in jedem Dateimanager richtig sortiert sind).
pub fn page_file_name(stem: &str, page: usize, page_count: usize, ext: &str) -> String {
    let digits = page_count.max(1).to_string().len();
    format!("{stem}-Seite-{:0digits$}.{ext}", page + 1)
}

/// Fortschritt melden; `false` = abbrechen.
pub type Progress<'a> = &'a dyn Fn(f32, String) -> bool;

/// Wohin Bilder geschrieben werden: Ordner, Dateistamm und Seitenzahl des Dokuments
/// (für die Anzahl führender Nullen in den Dateinamen).
pub struct ImageTarget<'a> {
    pub dir: &'a Path,
    pub stem: &'a str,
    pub page_count: usize,
}

/// Exportiert Seiten als Bilder (je Seite eine Datei). Liefert die Anzahl Dateien.
pub fn export_images(
    renderer: &OffscreenRenderer,
    bytes: &Arc<Vec<u8>>,
    pages: &[(usize, (f32, f32))],
    options: &ExportOptions,
    target: &ImageTarget,
    progress: Progress,
) -> Result<usize, String> {
    let ImageTarget {
        dir,
        stem,
        page_count,
    } = *target;
    let total = pages.len();
    for (i, &(page, size)) in pages.iter().enumerate() {
        if !progress(
            i as f32 / total as f32,
            format!("Seite {} ({} von {total}) …", page + 1, i + 1),
        ) {
            return Err("Export abgebrochen.".into());
        }
        let img = renderer
            .render_page(bytes, page as u32, size, options.dpi as f32 / 72.0)
            .map_err(|e| format!("Seite {} rendern: {e}", page + 1))?;
        let data = match options.format {
            ExportFormat::Jpeg => image_codec::encode_jpeg(
                &img.pixels,
                img.width,
                img.height,
                PixelFormat::Rgba,
                options.quality,
            ),
            _ => image_codec::encode_png(&img.pixels, img.width, img.height, PixelFormat::Rgba),
        }
        .map_err(|e| e.to_string())?;
        let path = dir.join(page_file_name(
            stem,
            page,
            page_count,
            options.format.extension(),
        ));
        crate::platform::write(&path, &data).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(total)
}

/// Text der Seiten; Seiten getrennt durch einen Seitenvorschub (U+000C), wie bei pdftotext.
pub fn export_text(
    renderer: &OffscreenRenderer,
    bytes: &Arc<Vec<u8>>,
    pages: &[usize],
    progress: Progress,
) -> Result<String, String> {
    let total = pages.len();
    let mut out = String::new();
    for (i, &page) in pages.iter().enumerate() {
        if !progress(
            i as f32 / total as f32,
            format!("Seite {} ({} von {total}) …", page + 1, i + 1),
        ) {
            return Err("Export abgebrochen.".into());
        }
        let text = renderer
            .page_text(bytes, page as u32)
            .map_err(|e| format!("Seite {}: {e}", page + 1))?;
        // PDFium trennt Zeilen mit "\r\n" – einheitlich "\n".
        out.push_str(&text.replace("\r\n", "\n"));
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\u{c}');
    }
    Ok(out)
}

impl PdfToolApp {
    pub(crate) fn open_export_dialog(&mut self) {
        if self.document.is_some() {
            self.export_dialog = Some(ExportDialog::default());
        }
    }

    pub(crate) fn export_dialog_ui(&mut self, ctx: &egui::Context) {
        let (Some(dlg), Some(d)) = (&mut self.export_dialog, &self.document) else {
            return;
        };
        let n = d.doc.page_count();
        let mut open = true;
        let mut confirmed = false;
        egui::Window::new("Exportieren")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let o = &mut dlg.options;
                for f in ExportFormat::ALL {
                    ui.radio_value(&mut o.format, f, f.label());
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Seiten:");
                    ui.add(
                        egui::TextEdit::singleline(&mut dlg.ranges)
                            .hint_text("alle")
                            .desired_width(140.0),
                    )
                    .on_hover_text(format!("z. B. 1-3, 5 – leer = alle {n} Seiten"));
                });
                if o.format.per_page() {
                    ui.horizontal(|ui| {
                        ui.label("Auflösung:");
                        ui.add(
                            egui::DragValue::new(&mut o.dpi)
                                .range(36..=600)
                                .suffix(" dpi"),
                        );
                    });
                }
                if o.format == ExportFormat::Jpeg {
                    ui.horizontal(|ui| {
                        ui.label("Qualität:");
                        ui.add(egui::Slider::new(&mut o.quality, 10..=100));
                    });
                }
                if o.format == ExportFormat::Docx {
                    ui.weak(
                        "Absätze und Bilder in Lesereihenfolge – ohne originalgetreues Layout.",
                    );
                }
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.add_space(6.0);
                confirmed = ui.button("Exportieren…").clicked();
            });
        if !open {
            self.export_dialog = None;
            return;
        }
        if !confirmed {
            return;
        }
        let pages = if dlg.ranges.trim().is_empty() {
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
        let options = dlg.options;
        let stem = d.doc.path().and_then(|p| p.file_stem()).map_or_else(
            || "Dokument".to_string(),
            |s| s.to_string_lossy().into_owned(),
        );
        // Ziel wählen: Ordner für Bilder, Datei sonst.
        let target = if options.format.per_page() {
            crate::platform::FileDialog::new()
                .set_title("Ordner für die Bilder wählen")
                .pick_folder()
        } else {
            let ext = options.format.extension();
            crate::platform::FileDialog::new()
                .add_filter(ext.to_uppercase(), &[ext])
                .set_file_name(format!("{stem}.{ext}"))
                .save_file()
        };
        let Some(target) = target else { return };
        self.export_dialog = None;
        self.start_export(ctx, pages, options, target, stem);
    }

    fn start_export(
        &mut self,
        ctx: &egui::Context,
        pages: Vec<usize>,
        options: ExportOptions,
        target: PathBuf,
        stem: String,
    ) {
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
        let page_count = d.doc.page_count();
        let sized: Vec<(usize, (f32, f32))> = pages
            .iter()
            .filter_map(|&p| Some((p, d.doc.page(p).ok()?.display_size())))
            .collect();
        // DOCX braucht die Absätze aus core – die liest der Job aus der Momentaufnahme.
        let job = crate::jobs::Job::spawn("Exportieren", ctx, move |c| {
            let progress = |f: f32, text: String| {
                c.progress(f, text);
                !c.cancelled()
            };
            match options.format {
                ExportFormat::Png | ExportFormat::Jpeg => {
                    let dest = ImageTarget {
                        dir: &target,
                        stem: &stem,
                        page_count,
                    };
                    let n =
                        export_images(&renderer, &snapshot, &sized, &options, &dest, &progress)?;
                    Ok(format!("{n} Bild(er) gespeichert in\n{}", target.display()))
                }
                ExportFormat::Text => {
                    let text = export_text(&renderer, &snapshot, &pages, &progress)?;
                    crate::platform::write(&target, text.as_bytes()).map_err(|e| e.to_string())?;
                    Ok(format!("Text gespeichert:\n{}", target.display()))
                }
                ExportFormat::Docx => {
                    let doc = pdftool_core::Document::from_bytes(snapshot.to_vec())
                        .map_err(|e| e.to_string())?;
                    let bytes =
                        pdftool_core::docx::export_docx(&doc, &pages, &|f, t| progress(f, t))
                            .map_err(|e| e.to_string())?;
                    crate::platform::write(&target, &bytes).map_err(|e| e.to_string())?;
                    Ok(format!("Word-Datei gespeichert:\n{}", target.display()))
                }
            }
        });
        self.job = Some(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_sort_correctly() {
        assert_eq!(page_file_name("Brief", 0, 9, "png"), "Brief-Seite-1.png");
        assert_eq!(
            page_file_name("Brief", 6, 120, "jpg"),
            "Brief-Seite-007.jpg"
        );
    }

    /// Echter Export (nur mit PDFium, sonst übersprungen).
    #[test]
    fn exports_images_and_text() {
        let Ok(svc) = pdftool_render::RenderService::start(|| {}) else {
            eprintln!("PDFium nicht gefunden – Test übersprungen");
            return;
        };
        let off = svc.offscreen().unwrap();
        let path = format!(
            "{}/../../tests/fixtures/text_two_pages.pdf",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes = Arc::new(std::fs::read(path).unwrap());
        let dir = std::env::temp_dir().join(format!("fc_export_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pages = [(0, (595.0, 842.0)), (1, (595.0, 842.0))];
        for format in [ExportFormat::Png, ExportFormat::Jpeg] {
            let opts = ExportOptions {
                format,
                dpi: 72,
                quality: 80,
            };
            let dest = ImageTarget {
                dir: &dir,
                stem: "t",
                page_count: 2,
            };
            let n = export_images(&off, &bytes, &pages, &opts, &dest, &|_, _| true).unwrap();
            assert_eq!(n, 2);
            let file = dir.join(page_file_name("t", 1, 2, format.extension()));
            let img = pdftool_core::NewImage::from_bytes(&std::fs::read(file).unwrap()).unwrap();
            assert_eq!((img.width, img.height), (595, 842));
        }
        let text = export_text(&off, &bytes, &[0, 1], &|_, _| true).unwrap();
        assert!(text.contains("PDF Tool Testdokument"));
        assert_eq!(text.matches('\u{c}').count(), 2);
        // Abbrechen wird beachtet.
        assert!(export_text(&off, &bytes, &[0], &|_, _| false).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
