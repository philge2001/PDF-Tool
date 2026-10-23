//! Menü "Barrierefreiheit": Prüfbericht (Fenster) und Dialog "Automatisch taggen".
//!
//! Der Bericht kommt aus `Document::accessibility_check` (core) und wird neu berechnet, sobald
//! sich Dateibytes, Titel/Sprache oder Alternativtexte ändern. Das Taggen läuft wie
//! Komprimieren/PDF/A als Hintergrund-Job; das Ergebnis ist eine neue, ungespeicherte Fassung.

use crate::app::PdfToolApp;
use crate::redact::DocSlot;
use crate::signatures::{AMBER, GREEN, RED, status_dot};
use eframe::egui::{self, RichText};
use pdftool_core::accessibility::{Check, CheckKind, CheckStatus};
use pdftool_core::autotag::{AutoTagOptions, auto_tag_pdf};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Wofür der Bericht gilt (Bytes-Zeiger, Titel/Sprache, Alternativtexte).
type ReportKey = (usize, pdftool_core::tags::DocProperties, usize);

#[derive(Default)]
pub struct A11yReport {
    key: Option<ReportKey>,
    checks: Vec<Check>,
    error: Option<String>,
}

pub struct AutoTagDialog {
    lang: String,
    title: String,
    embed_fonts: bool,
}

/// Knopf neben einem Prüfpunkt.
enum Fix {
    AutoTag,
    TagsPanel,
    #[cfg(feature = "ocr")]
    Ocr,
}

/// Passender Knopf zu einem nicht bestandenen Prüfpunkt.
fn fix_for(kind: CheckKind, tagged: bool) -> Option<(&'static str, Fix)> {
    match kind {
        CheckKind::Tagged if !tagged => Some(("Automatisch taggen…", Fix::AutoTag)),
        CheckKind::Language | CheckKind::Title | CheckKind::AltText => {
            Some(("Im Panel „Tags“ ändern", Fix::TagsPanel))
        }
        #[cfg(feature = "ocr")]
        CheckKind::Scanned => Some(("Texterkennung…", Fix::Ocr)),
        _ => None,
    }
}

impl PdfToolApp {
    pub(crate) fn open_a11y_report(&mut self) {
        if self.document.is_some() {
            self.a11y_report = Some(A11yReport::default());
        }
    }

    pub(crate) fn open_autotag_dialog(&mut self) {
        let Some(d) = &self.document else { return };
        let props = d.doc.properties();
        let stem = d
            .doc
            .path()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.autotag_dialog = Some(AutoTagDialog {
            lang: if props.lang.is_empty() {
                "de-DE".into()
            } else {
                props.lang.clone()
            },
            title: if props.title.is_empty() {
                stem
            } else {
                props.title.clone()
            },
            embed_fonts: true,
        });
    }

    pub(crate) fn a11y_report_ui(&mut self, ctx: &egui::Context) {
        let (Some(rep), Some(d)) = (&mut self.a11y_report, &self.document) else {
            return;
        };
        let key: ReportKey = (
            Arc::as_ptr(d.doc.bytes()) as usize,
            d.doc.properties().clone(),
            d.doc.model().alt_texts.len(),
        );
        // Alternativtexte ändern ihre Anzahl nicht immer (Text ersetzt Text) – dann reicht
        // "Erneut prüfen". Titel/Sprache und neue Fassungen werden automatisch erkannt.
        let mut recheck = rep.key.as_ref() != Some(&key);
        let mut open = true;
        let mut fix = None;
        let tagged = d.doc.is_tagged();
        egui::Window::new("Barrierefreiheit prüfen")
            .open(&mut open)
            .collapsible(false)
            .default_width(460.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                if let Some(e) = &rep.error {
                    ui.colored_label(RED, e);
                }
                let fails = rep
                    .checks
                    .iter()
                    .filter(|c| c.status == CheckStatus::Fail)
                    .count();
                let warns = rep
                    .checks
                    .iter()
                    .filter(|c| c.status == CheckStatus::Warn)
                    .count();
                ui.label(match (fails, warns) {
                    (0, 0) => RichText::new("Keine Probleme gefunden.").color(GREEN),
                    (0, w) => RichText::new(format!("{w} Hinweis(e).")).color(AMBER),
                    (f, w) => RichText::new(format!("{f} Problem(e), {w} Hinweis(e).")).color(RED),
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for c in &rep.checks {
                            ui.horizontal(|ui| {
                                status_dot(
                                    ui,
                                    match c.status {
                                        CheckStatus::Pass => GREEN,
                                        CheckStatus::Warn => AMBER,
                                        CheckStatus::Fail => RED,
                                    },
                                );
                                ui.label(RichText::new(&c.title).strong());
                                if c.status != CheckStatus::Pass
                                    && let Some((label, f)) = fix_for(c.kind, tagged)
                                    && ui.small_button(label).clicked()
                                {
                                    fix = Some(f);
                                }
                            });
                            ui.indent(("a11y", c.title.as_str()), |ui| {
                                ui.label(RichText::new(&c.detail).weak());
                            });
                            ui.add_space(2.0);
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Erneut prüfen").clicked() {
                        recheck = true;
                    }
                    ui.weak("Ersetzt keine vollständige PDF/UA-Prüfung.");
                });
            });
        if recheck {
            match d.doc.accessibility_check() {
                Ok(c) => {
                    rep.checks = c;
                    rep.error = None;
                }
                Err(e) => rep.error = Some(format!("Prüfung fehlgeschlagen: {e}")),
            }
            rep.key = Some(key);
        }
        if !open {
            self.a11y_report = None;
        }
        match fix {
            Some(Fix::AutoTag) => self.open_autotag_dialog(),
            Some(Fix::TagsPanel) => {
                self.right_pane = Some(crate::shell::RightPane::Tags);
            }
            #[cfg(feature = "ocr")]
            Some(Fix::Ocr) => self.open_ocr_dialog(),
            None => {}
        }
    }

    pub(crate) fn autotag_dialog_ui(&mut self, ctx: &egui::Context) {
        let (Some(dlg), Some(d)) = (&mut self.autotag_dialog, &self.document) else {
            return;
        };
        let mut open = true;
        let mut confirmed = false;
        let tagged = d.doc.is_tagged();
        egui::Window::new("Automatisch taggen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.label(
                    "Erkennt Überschriften, Absätze und Bilder aus dem Layout und legt daraus \
                     einen Strukturbaum an. Linien, Flächen sowie Kopf- und Fußzeilen werden \
                     als Artefakt markiert.",
                );
                ui.add_space(4.0);
                egui::Grid::new("autotag").num_columns(2).show(ui, |ui| {
                    ui.label("Sprache:");
                    ui.add(egui::TextEdit::singleline(&mut dlg.lang).desired_width(100.0));
                    ui.end_row();
                    ui.label("Titel:");
                    ui.add(
                        egui::TextEdit::singleline(&mut dlg.title)
                            .desired_width(260.0)
                            .hint_text("leer = erste Überschrift"),
                    );
                    ui.end_row();
                });
                ui.checkbox(
                    &mut dlg.embed_fonts,
                    "Standardschriften einbetten (Liberation, wie bei PDF/A)",
                );
                ui.weak(
                    "Die Lesereihenfolge folgt dem Seiteninhalt – bei mehrspaltigem Satz bitte \
                     im Panel „Tags“ kontrollieren. Alternativtexte für Bilder danach ergänzen.",
                );
                if tagged {
                    ui.colored_label(RED, "Das Dokument ist bereits getaggt.");
                }
                if d.doc.has_signatures() {
                    ui.colored_label(
                        AMBER,
                        "Achtung: Die Datei wird komplett neu geschrieben – Signaturen werden ungültig.",
                    );
                }
                ui.add_space(6.0);
                confirmed = ui
                    .add_enabled(!tagged, egui::Button::new("Taggen"))
                    .clicked();
            });
        if !open {
            self.autotag_dialog = None;
            return;
        }
        if !confirmed {
            return;
        }
        let opts = AutoTagOptions {
            lang: dlg.lang.trim().to_string(),
            title: dlg.title.trim().to_string(),
            embed_fonts: dlg.embed_fonts,
        };
        self.autotag_dialog = None;
        self.start_autotag(ctx, opts);
    }

    fn start_autotag(&mut self, ctx: &egui::Context, opts: AutoTagOptions) {
        let Some(d) = &self.document else { return };
        if self.job.is_some() {
            return;
        }
        // Aktueller Stand inkl. ungespeicherter Änderungen (unverschlüsselt).
        let bytes = match d.doc.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let slot: DocSlot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let job = crate::jobs::Job::spawn("Automatisch taggen", ctx, move |c| {
            let (result, report) = auto_tag_pdf(&bytes, &opts, &|f, text| {
                c.progress(f, text);
                !c.cancelled()
            })
            .map_err(|e| format!("Taggen fehlgeschlagen:\n{e}"))?;
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
        // Ergebnis gleich im Panel zeigen.
        self.right_pane = Some(crate::shell::RightPane::Tags);
    }
}
