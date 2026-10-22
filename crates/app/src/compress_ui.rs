//! Dialog "Komprimieren": Ziel-Auflösung für Bilder und JPEG-Qualität wählen. Das Ergebnis
//! ist – wie beim Schwärzen – eine neue, noch ungespeicherte Fassung des Dokuments.

use crate::app::FolioApp;
use crate::redact::DocSlot;
use eframe::egui;
use foliocraft_core::compress::{CompressOptions, compress_pdf, human};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Auswahl für die Bildauflösung (dpi; 0 = Bilder nicht verkleinern).
const DPI_CHOICES: [(u32, &str); 5] = [
    (300, "300 dpi – Druck"),
    (150, "150 dpi – Bildschirm und Büro (empfohlen)"),
    (96, "96 dpi – klein"),
    (72, "72 dpi – sehr klein"),
    (0, "Bilder nicht verkleinern"),
];

pub struct CompressDialog {
    dpi: u32,
    quality: u8,
}

impl Default for CompressDialog {
    fn default() -> Self {
        Self {
            dpi: 150,
            quality: 75,
        }
    }
}

impl FolioApp {
    pub(crate) fn open_compress_dialog(&mut self) {
        if self.document.is_some() {
            self.compress_dialog = Some(CompressDialog::default());
        }
    }

    pub(crate) fn compress_dialog_ui(&mut self, ctx: &egui::Context) {
        let (Some(dlg), Some(d)) = (&mut self.compress_dialog, &self.document) else {
            return;
        };
        let signed = d.doc.has_signatures();
        let size = d.doc.bytes().len();
        let mut open = true;
        let mut confirmed = false;
        egui::Window::new("Komprimieren")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.label(format!("Aktuelle Dateigröße: {}", human(size)));
                ui.add_space(4.0);
                ui.label("Bilder verkleinern auf:");
                for (dpi, label) in DPI_CHOICES {
                    ui.radio_value(&mut dlg.dpi, dpi, label);
                }
                ui.add_enabled_ui(dlg.dpi > 0, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("JPEG-Qualität:");
                        ui.add(egui::Slider::new(&mut dlg.quality, 30..=95));
                    });
                });
                ui.weak("Außerdem: alle Inhalte komprimieren, alte Fassungen und unbenutzte Objekte entfernen.");
                if signed {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Achtung: Das Dokument ist signiert – die Datei wird komplett neu geschrieben, \
                         die Signaturen werden dadurch ungültig.",
                    );
                }
                ui.add_space(6.0);
                confirmed = ui.button("Komprimieren").clicked();
            });
        if !open {
            self.compress_dialog = None;
            return;
        }
        if !confirmed {
            return;
        }
        let options = CompressOptions {
            image_dpi: (dlg.dpi > 0).then_some(dlg.dpi),
            jpeg_quality: dlg.quality,
        };
        self.compress_dialog = None;
        self.start_compress(ctx, options);
    }

    fn start_compress(&mut self, ctx: &egui::Context, options: CompressOptions) {
        let Some(d) = &self.document else { return };
        if self.job.is_some() {
            return;
        }
        // Aktueller Stand inkl. ungespeicherter Änderungen.
        let bytes = match d.doc.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let slot: DocSlot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let job = crate::jobs::Job::spawn("Komprimieren", ctx, move |c| {
            let (result, report) = compress_pdf(&bytes, &options, &|f, text| {
                c.progress(f, text);
                !c.cancelled()
            })
            .map_err(|e| format!("Komprimieren fehlgeschlagen:\n{e}"))?;
            if let Ok(mut s) = out.lock() {
                *s = Some(result);
            }
            Ok(format!(
                "{}\n\nNoch nicht gespeichert – zum Übernehmen speichern (Strg+S).",
                report.summary()
            ))
        });
        self.pending_doc = Some((slot, d.doc.path().map(PathBuf::from)));
        self.job = Some(job);
    }
}
