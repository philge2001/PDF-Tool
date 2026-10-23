//! Schwärzen in der App: Markierungen (/Redact-Annotationen) anlegen und anwenden.
//!
//! Markieren ist eine normale, rückgängig machbare Annotation. Erst "Schwärzungen anwenden"
//! entfernt die Inhalte darunter endgültig (siehe `core::redact`).

use crate::app::OpenDocument;
use crate::tools::{Tool, ToolSettings};
use pdftool_core::{AddAnnotation, AnnotKind, Batch, Command, Quad};

impl OpenDocument {
    /// Jeden Suchtreffer zum Schwärzen markieren (ein Undo-Schritt).
    pub fn redact_search_hits(&mut self, settings: &ToolSettings) {
        // Erst die Daten herauskopieren: `new_annotation` braucht `self` veränderlich.
        let hits: Vec<(usize, Vec<Quad>)> = self
            .search
            .hits
            .iter()
            .map(|h| {
                (
                    h.page,
                    h.rects.iter().copied().map(Quad::from_rect).collect(),
                )
            })
            .collect();
        let query = self.search.query.clone();
        let cmds: Vec<Box<dyn Command>> = hits
            .into_iter()
            .map(|(page, quads)| {
                let mut a = self.new_annotation(
                    page,
                    AnnotKind::Redact { quads },
                    Tool::RedactText,
                    settings,
                );
                a.contents = query.clone();
                Box::new(AddAnnotation::new(a)) as Box<dyn Command>
            })
            .collect();
        if !cmds.is_empty() {
            let label = format!("{} Treffer schwärzen", cmds.len());
            self.execute(Box::new(Batch::new(label, cmds)));
        }
    }

    /// Anzahl der markierten, noch nicht angewendeten Schwärzungen.
    pub fn pending_redactions(&self) -> usize {
        self.doc
            .annotations()
            .iter()
            .filter(|a| matches!(a.kind, AnnotKind::Redact { .. }))
            .count()
    }
}

// ---------- Anwenden ----------

use crate::app::PdfToolApp;
use eframe::egui;
use pdftool_core::Document;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Ergebnis eines Hintergrund-Jobs, das ein neues Dokument liefert (Bytes + Pfad).
pub type DocSlot = Arc<Mutex<Option<Vec<u8>>>>;

impl PdfToolApp {
    /// Rückfrage vor dem endgültigen Schwärzen.
    pub(crate) fn redact_confirm_ui(&mut self, ctx: &egui::Context) {
        if !self.redact_confirm {
            return;
        }
        let Some(d) = &self.document else {
            self.redact_confirm = false;
            return;
        };
        let count = d.pending_redactions();
        let signed = d.doc.has_signatures();
        let mut apply = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("redact_confirm")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading("Schwärzungen anwenden");
            ui.label(format!(
                "{count} markierte Schwärzung(en) werden angewendet: Text, Grafik und Bildbereiche \
                 darunter werden endgültig entfernt, ebenso Kommentare und Formularfelder in diesen \
                 Bereichen. Das Dokument wird dafür komplett neu geschrieben."
            ));
            ui.add_space(4.0);
            ui.label(
                "Das Ergebnis erscheint als ungespeicherte Fassung. Erst beim Speichern wird die \
                 Datei überschrieben – danach lässt sich nichts mehr zurückholen.",
            );
            if signed {
                ui.add_space(4.0);
                ui.colored_label(
                    crate::signatures::RED,
                    "Achtung: Das Dokument ist signiert. Alle Signaturen werden dadurch ungültig.",
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Schwärzen").clicked() {
                    apply = true;
                }
                if ui.button("Abbrechen").clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.redact_confirm = false;
        }
        if apply {
            self.redact_confirm = false;
            self.start_redaction(ctx);
        }
    }

    fn start_redaction(&mut self, ctx: &egui::Context) {
        let Some(d) = &self.document else { return };
        if self.job.is_some() {
            return;
        }
        let (bytes, areas) = match d.doc.redaction_input() {
            Ok(x) => x,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let slot: DocSlot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        // Das Schwärzen selbst läuft im Hintergrund: Bytes und Bereiche wandern in den Job.
        let job = crate::jobs::Job::spawn("Schwärzen", ctx, move |c| {
            c.progress(0.2, "Inhalte werden entfernt …");
            let (result, report) = pdftool_core::redact::redact_pdf(&bytes, &areas)
                .map_err(|e| format!("Schwärzen fehlgeschlagen:\n{e}"))?;
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

    /// Nach einem erfolgreichen Job: neues Dokument übernehmen (falls einer eins geliefert hat).
    pub(crate) fn take_pending_doc(&mut self) {
        let Some((slot, path)) = self.pending_doc.take() else {
            return;
        };
        let bytes = slot.lock().ok().and_then(|mut s| s.take());
        let Some(bytes) = bytes else { return };
        match Document::replacement(bytes, path) {
            Ok(mut doc) => {
                // Eine neu geschriebene Fassung behält den Passwortschutz des Originals.
                if let Some(old) = &self.document {
                    doc.inherit_security(&old.doc);
                }
                self.adopt_document(doc)
            }
            Err(e) => self.error = Some(format!("Ergebnis unlesbar:\n{e}")),
        }
    }
}

// ---------- Bereinigen ----------

use pdftool_core::sanitize::{Findings, SanitizeOptions};

pub struct SanitizeDialog {
    findings: Findings,
    opts: SanitizeOptions,
    /// Aktueller Stand des Dokuments (inkl. ungespeicherter Änderungen).
    bytes: Vec<u8>,
}

impl PdfToolApp {
    pub(crate) fn open_sanitize_dialog(&mut self) {
        let Some(d) = &self.document else { return };
        let result = d
            .doc
            .to_bytes()
            .and_then(|b| Ok((pdftool_core::sanitize::inspect(&b)?, b)));
        match result {
            Ok((findings, bytes)) => {
                self.sanitize_dialog = Some(SanitizeDialog {
                    findings,
                    opts: SanitizeOptions::default(),
                    bytes,
                })
            }
            Err(e) => self.error = Some(format!("Dokument konnte nicht untersucht werden:\n{e}")),
        }
    }

    pub(crate) fn sanitize_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.sanitize_dialog else {
            return;
        };
        let mut open = true;
        let mut run = false;
        let signed = self
            .document
            .as_ref()
            .is_some_and(|d| d.doc.has_signatures());
        egui::Window::new("Dokument bereinigen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.label("Entfernt versteckte oder vergessene Informationen. Gefunden:");
                ui.add_space(4.0);
                let f = &dlg.findings;
                let o = &mut dlg.opts;
                let row =
                    |ui: &mut egui::Ui, flag: &mut bool, count: usize, label: &str, hint: &str| {
                        ui.add_enabled_ui(count > 0, |ui| {
                            ui.checkbox(flag, format!("{label} ({count})"))
                                .on_hover_text(hint);
                        });
                        if count == 0 {
                            *flag = false;
                        }
                    };
                row(
                    ui,
                    &mut o.metadata,
                    f.metadata,
                    "Metadaten",
                    "Dokumentinfo (Autor, Titel, Programm …), XMP-Metadaten, private Programmdaten",
                );
                row(
                    ui,
                    &mut o.javascript,
                    f.javascript,
                    "JavaScript und automatische Aktionen",
                    "Skripte, Aktionen beim Öffnen, Zusatzaktionen von Seiten und Feldern",
                );
                row(
                    ui,
                    &mut o.embedded_files,
                    f.embedded_files,
                    "Eingebettete Dateien",
                    "Dateianhänge des Dokuments und Dateianhang-Kommentare",
                );
                row(
                    ui,
                    &mut o.hidden_layers,
                    f.hidden_layers,
                    "Ausgeblendete Ebenen",
                    "Inhalte von Ebenen, die standardmäßig nicht angezeigt werden",
                );
                row(
                    ui,
                    &mut o.comments,
                    f.comments,
                    "Kommentare",
                    "Alle Kommentare und Markierungen (Formularfelder und Links bleiben)",
                );
                row(
                    ui,
                    &mut o.form_data,
                    f.form_values,
                    "Formulardaten",
                    "Eingetragene Werte in Formularfeldern (die Felder selbst bleiben)",
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Immer: alte Revisionen und nicht mehr benutzte Objekte werden entfernt \
                         (die Datei wird komplett neu geschrieben).",
                    )
                    .weak(),
                );
                if signed {
                    ui.colored_label(
                        crate::signatures::RED,
                        "Achtung: Das Dokument ist signiert. Alle Signaturen werden ungültig.",
                    );
                }
                ui.add_space(6.0);
                if ui.button("Bereinigen").clicked() {
                    run = true;
                }
            });
        if !open {
            self.sanitize_dialog = None;
        }
        if run {
            self.start_sanitize(ctx);
        }
    }

    fn start_sanitize(&mut self, ctx: &egui::Context) {
        if self.job.is_some() {
            return;
        }
        let Some(dlg) = self.sanitize_dialog.take() else {
            return;
        };
        let path = self
            .document
            .as_ref()
            .and_then(|d| d.doc.path().map(PathBuf::from));
        let slot: DocSlot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let job = crate::jobs::Job::spawn("Bereinigen", ctx, move |c| {
            c.progress(0.2, "Dokument wird bereinigt …");
            let (result, done) = pdftool_core::sanitize::sanitize_pdf(&dlg.bytes, dlg.opts)
                .map_err(|e| format!("Bereinigen fehlgeschlagen:\n{e}"))?;
            if let Ok(mut s) = out.lock() {
                *s = Some(result);
            }
            Ok(format!(
                "{}\n\nNoch nicht gespeichert – zum Übernehmen speichern (Strg+S).",
                done.join("\n")
            ))
        });
        self.pending_doc = Some((slot, path));
        self.job = Some(job);
    }
}
