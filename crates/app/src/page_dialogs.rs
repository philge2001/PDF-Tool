//! Dialoge für Seitenorganisation: Seiten aus Datei einfügen, Dateien zusammenführen.

use crate::app::PdfToolApp;
use eframe::egui;
use pdftool_core::{Document, parse_page_ranges};
use std::path::PathBuf;

/// Wohin eingefügt wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertPos {
    BeforeCurrent,
    AfterCurrent,
    Start,
    End,
}

/// Dialog "Seiten aus Datei einfügen".
pub struct InsertDialog {
    source: Document,
    source_name: String,
    /// Seitenbereich wie "1-3, 5" (1-basiert).
    ranges: String,
    position: InsertPos,
    error: Option<String>,
}

/// Dialog "Dateien zusammenführen".
#[derive(Default)]
pub struct MergeDialog {
    files: Vec<PathBuf>,
    error: Option<String>,
}

impl MergeDialog {
    /// Gewählte Dateien übernehmen (siehe `Pick::MergeFiles`).
    pub(crate) fn add_files(&mut self, paths: Vec<PathBuf>) {
        self.files.extend(paths);
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl PdfToolApp {
    /// Datei wählen; danach öffnet `insert_from_file` den Einfüge-Dialog (`Pick::InsertPages`).
    pub(crate) fn start_insert_from_file(&mut self, ctx: &egui::Context) {
        crate::platform::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .pick(ctx, crate::platform::Pick::InsertPages, false);
    }

    /// Öffnet den Einfüge-Dialog für die gewählte Datei.
    pub(crate) fn insert_from_file(&mut self, path: &std::path::Path) {
        let path = path.to_path_buf();
        match crate::platform::open_document(&path, None) {
            Ok(source) => {
                let n = source.page_count();
                self.insert_dialog = Some(InsertDialog {
                    source,
                    source_name: file_name(&path),
                    ranges: if n == 1 { "1".into() } else { format!("1-{n}") },
                    position: InsertPos::AfterCurrent,
                    error: None,
                });
            }
            Err(e) => {
                self.error = Some(format!(
                    "„{}“ kann nicht geöffnet werden:\n{e}",
                    path.display()
                ))
            }
        }
    }

    pub(crate) fn insert_dialog_ui(&mut self, ctx: &egui::Context) {
        let (Some(dlg), Some(d)) = (&mut self.insert_dialog, &mut self.document) else {
            return;
        };
        let mut open = true;
        let mut confirmed = false;
        egui::Window::new("Seiten einfügen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Aus „{}“ ({} Seiten)",
                    dlg.source_name,
                    dlg.source.page_count()
                ));
                ui.horizontal(|ui| {
                    ui.label("Seiten:");
                    ui.text_edit_singleline(&mut dlg.ranges)
                        .on_hover_text("z. B. 1-3, 5, 8- (8- = ab Seite 8 bis zum Ende)");
                });
                ui.label("Einfügen:");
                let cur = d.viewer.current_page + 1;
                ui.radio_value(
                    &mut dlg.position,
                    InsertPos::BeforeCurrent,
                    format!("vor Seite {cur}"),
                );
                ui.radio_value(
                    &mut dlg.position,
                    InsertPos::AfterCurrent,
                    format!("nach Seite {cur}"),
                );
                ui.radio_value(&mut dlg.position, InsertPos::Start, "am Anfang");
                ui.radio_value(&mut dlg.position, InsertPos::End, "am Ende");
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.add_space(6.0);
                confirmed = ui.button("Einfügen").clicked();
            });

        if confirmed {
            let at = match dlg.position {
                InsertPos::BeforeCurrent => d.viewer.current_page,
                InsertPos::AfterCurrent => d.viewer.current_page + 1,
                InsertPos::Start => 0,
                InsertPos::End => d.doc.page_count(),
            };
            let result = parse_page_ranges(&dlg.ranges, dlg.source.page_count())
                .and_then(|pages| d.doc.prepare_insert(&dlg.source, &pages, at));
            match result {
                Ok(cmd) => {
                    d.execute(Box::new(cmd));
                    open = false;
                }
                Err(e) => dlg.error = Some(e.to_string()),
            }
        }
        if !open {
            self.insert_dialog = None;
        }
    }

    /// Leere Seite nach der aktuellen einfügen (gleiche Größe wie die aktuelle Seite).
    pub(crate) fn insert_blank_page(&mut self) {
        let Some(d) = &mut self.document else { return };
        let cur = d.viewer.current_page;
        let size = d
            .doc
            .page(cur)
            .map(|p| (p.crop_box.width(), p.crop_box.height()))
            .unwrap_or((595.0, 842.0));
        let cmd = d.doc.prepare_blank_page(cur + 1, size);
        d.execute(Box::new(cmd));
    }

    pub(crate) fn merge_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.merge_dialog else {
            return;
        };
        let mut open = true;
        let mut confirmed = false;
        egui::Window::new("Dateien zusammenführen")
            .open(&mut open)
            .collapsible(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label("Die Dateien werden in dieser Reihenfolge zu einem neuen Dokument zusammengefügt.");
                ui.add_space(4.0);
                let mut action: Option<(usize, i32)> = None; // (Index, -1 hoch / +1 runter / 0 entfernen)
                egui::Grid::new("merge_files").striped(true).show(ui, |ui| {
                    let n = dlg.files.len();
                    for (i, p) in dlg.files.iter().enumerate() {
                        ui.label(format!("{}.", i + 1));
                        ui.label(file_name(p)).on_hover_text(p.display().to_string());
                        if ui.add_enabled(i > 0, egui::Button::new("↑")).clicked() {
                            action = Some((i, -1));
                        }
                        if ui.add_enabled(i + 1 < n, egui::Button::new("↓")).clicked() {
                            action = Some((i, 1));
                        }
                        if ui.button("Entfernen").clicked() {
                            action = Some((i, 0));
                        }
                        ui.end_row();
                    }
                });
                match action {
                    Some((i, 0)) => {
                        dlg.files.remove(i);
                    }
                    Some((i, dir)) => dlg.files.swap(i, (i as i32 + dir) as usize),
                    None => {}
                }
                if dlg.files.is_empty() {
                    ui.label(egui::RichText::new("Noch keine Dateien.").weak());
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Dateien hinzufügen…").clicked() {
                        crate::platform::FileDialog::new()
                            .add_filter("PDF", &["pdf"])
                            .pick(ctx, crate::platform::Pick::MergeFiles, true);
                    }
                    let ok = dlg.files.len() >= 2;
                    confirmed = ui
                        .add_enabled(ok, egui::Button::new("Zusammenführen"))
                        .on_disabled_hover_text("Mindestens zwei Dateien")
                        .clicked();
                });
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            });

        if confirmed {
            match merge_files(&dlg.files) {
                Ok(doc) => {
                    self.merge_dialog = None;
                    self.request_adopt(doc);
                    return;
                }
                Err(e) => dlg.error = Some(e),
            }
        }
        if !open {
            self.merge_dialog = None;
        }
    }
}

/// Erste Datei öffnen, alle weiteren hinten anfügen. Das Ergebnis hat keinen Dateipfad
/// (Speichern fragt nach einem Namen) und lässt sich per Rückgängig wieder aufteilen.
fn merge_files(files: &[PathBuf]) -> Result<Document, String> {
    let err = |p: &PathBuf, e: pdftool_core::CoreError| format!("„{}“: {e}", file_name(p));
    let (first, rest) = files.split_first().ok_or("Keine Dateien ausgewählt")?;
    let mut doc = crate::platform::open_document(first, None).map_err(|e| err(first, e))?;
    doc.forget_path();
    for p in rest {
        let source = crate::platform::open_document(p, None).map_err(|e| err(p, e))?;
        let all: Vec<usize> = (0..source.page_count()).collect();
        let at = doc.page_count();
        let cmd = doc
            .prepare_insert(&source, &all, at)
            .map_err(|e| err(p, e))?;
        doc.execute(Box::new(cmd)).map_err(|e| err(p, e))?;
    }
    Ok(doc)
}

impl PdfToolApp {
    pub(crate) fn open_merge_dialog(&mut self) {
        let mut dlg = MergeDialog::default();
        // Ist schon ein Dokument mit Dateipfad offen, steht es vorne in der Liste.
        if let Some(p) = self.document.as_ref().and_then(|d| d.doc.path()) {
            dlg.files.push(p.to_path_buf());
        }
        self.merge_dialog = Some(dlg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn merge_two_files() {
        let doc =
            merge_files(&[fixture("text_two_pages.pdf"), fixture("mixed_sizes.pdf")]).unwrap();
        assert_eq!(doc.page_count(), 5);
        assert!(doc.path().is_none(), "Ergebnis hat noch keinen Dateinamen");
        assert!(doc.is_dirty());
        // Der Renderer bekommt eine Momentaufnahme mit allen Seiten.
        let snap = doc.render_snapshot().unwrap();
        assert_eq!(Document::from_bytes(snap.bytes).unwrap().page_count(), 5);
    }

    #[test]
    fn merge_reports_broken_file() {
        let err = merge_files(&[fixture("text_two_pages.pdf"), fixture("gibt-es-nicht.pdf")])
            .err()
            .expect("muss fehlschlagen");
        assert!(err.contains("gibt-es-nicht.pdf"), "{err}");
    }
}

// ---------- Extrahieren & Teilen ----------

/// Dialog "Seiten extrahieren".
pub struct ExtractDialog {
    ranges: String,
    delete_after: bool,
    error: Option<String>,
}

/// Dialog "Dokument teilen".
pub struct SplitDialog {
    by_ranges: bool,
    every: usize,
    ranges: String,
    error: Option<String>,
}

/// Indizes (0-basiert) -> "1-3, 5" (1-basiert), für Vorgabewerte in Eingabefeldern.
pub fn format_ranges(pages: &[usize]) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            i += 1;
            end = pages[i];
        }
        parts.push(if start == end {
            format!("{}", start + 1)
        } else {
            format!("{}-{}", start + 1, end + 1)
        });
        i += 1;
    }
    parts.join(", ")
}

impl PdfToolApp {
    fn document_stem(&self) -> String {
        self.document
            .as_ref()
            .and_then(|d| d.doc.path())
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Dokument".into())
    }

    pub(crate) fn open_extract_dialog(&mut self) {
        let Some(d) = &self.document else { return };
        self.extract_dialog = Some(ExtractDialog {
            ranges: format_ranges(&d.target_pages()),
            delete_after: false,
            error: None,
        });
    }

    pub(crate) fn open_split_dialog(&mut self) {
        self.split_dialog = Some(SplitDialog {
            by_ranges: false,
            every: 1,
            ranges: String::new(),
            error: None,
        });
    }

    pub(crate) fn extract_dialog_ui(&mut self, ctx: &egui::Context) {
        let (Some(dlg), Some(d)) = (&mut self.extract_dialog, &self.document) else {
            return;
        };
        let n = d.doc.page_count();
        let mut open = true;
        let mut confirmed = false;
        egui::Window::new("Seiten extrahieren")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Seiten:");
                    ui.text_edit_singleline(&mut dlg.ranges)
                        .on_hover_text(format!("z. B. 1-3, 5 (Dokument hat {n} Seiten)"));
                });
                ui.checkbox(&mut dlg.delete_after, "Danach aus diesem Dokument löschen");
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.add_space(6.0);
                confirmed = ui.button("Extrahieren…").clicked();
            });
        if !open {
            self.extract_dialog = None;
            return;
        }
        if !confirmed {
            return;
        }

        let pages = match parse_page_ranges(&dlg.ranges, n) {
            Ok(p) => p,
            Err(e) => {
                dlg.error = Some(e.to_string());
                return;
            }
        };
        if dlg.delete_after && pages.len() >= n {
            dlg.error = Some("Es können nicht alle Seiten gelöscht werden.".into());
            return;
        }
        let delete_after = dlg.delete_after;
        let Some(target) = crate::platform::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(format!("{}_Auszug.pdf", self.document_stem()))
            .save_file()
        else {
            return; // abgebrochen – Dialog bleibt offen
        };
        let Some(d) = &mut self.document else { return };
        let snapshot = match d.doc.snapshot_pdf() {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(format!("Extrahieren nicht möglich:\n{e}"));
                return;
            }
        };
        if delete_after {
            // Die Momentaufnahme oben enthält die Seiten noch – Löschen ist ein normaler,
            // rückgängig machbarer Schritt im aktuellen Dokument.
            d.apply_page_action(crate::pages::PageAction::Delete(pages.clone()));
        }
        let count = pages.len();
        self.extract_dialog = None;
        self.job = Some(crate::jobs::Job::spawn(
            "Seiten extrahieren",
            ctx,
            move |job| {
                job.progress(0.1, "Seiten kopieren …");
                let bytes =
                    pdftool_core::extract_pages(&snapshot, &pages).map_err(|e| e.to_string())?;
                job.progress(0.8, "Datei schreiben …");
                crate::platform::write(&target, &bytes)
                    .map_err(|e| format!("„{}“: {e}", target.display()))?;
                Ok(format!(
                    "{count} Seite(n) nach „{}“ extrahiert.",
                    target.display()
                ))
            },
        ));
    }

    pub(crate) fn split_dialog_ui(&mut self, ctx: &egui::Context) {
        let (Some(dlg), Some(d)) = (&mut self.split_dialog, &self.document) else {
            return;
        };
        let n = d.doc.page_count();
        let mut open = true;
        let mut confirmed = false;
        egui::Window::new("Dokument teilen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.radio_value(&mut dlg.by_ranges, false, "Alle N Seiten eine neue Datei");
                ui.add_enabled_ui(!dlg.by_ranges, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.label("N =");
                        ui.add(egui::DragValue::new(&mut dlg.every).range(1..=n.max(1)));
                    });
                });
                ui.radio_value(&mut dlg.by_ranges, true, "Nach Bereichen");
                ui.add_enabled_ui(dlg.by_ranges, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut dlg.ranges).hint_text("1-3; 4-6; 7-"),
                        )
                        .on_hover_text(
                            "Eine Datei pro Bereich, getrennt durch Semikolon: 1-3; 4-6; 7-",
                        );
                    });
                });
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.add_space(6.0);
                confirmed = ui.button("Teilen…").clicked();
            });
        if !open {
            self.split_dialog = None;
            return;
        }
        if !confirmed {
            return;
        }

        let groups = if dlg.by_ranges {
            match pdftool_core::split_by_ranges(&dlg.ranges, n) {
                Ok(g) => g,
                Err(e) => {
                    dlg.error = Some(e.to_string());
                    return;
                }
            }
        } else {
            pdftool_core::split_every(n, dlg.every)
        };
        let Some(folder) = crate::platform::FileDialog::new().pick_folder() else {
            return;
        };
        let stem = self.document_stem();
        let width = groups.len().to_string().len();
        let targets: Vec<PathBuf> = (1..=groups.len())
            .map(|i| folder.join(format!("{stem}_Teil{i:0width$}.pdf")))
            .collect();
        // Im Browser landen die Teile als Downloads – dort gibt es nichts zu überschreiben.
        if let Some(existing) = targets
            .iter()
            .find(|p| !crate::platform::is_web() && p.exists())
        {
            if let Some(dlg) = &mut self.split_dialog {
                dlg.error = Some(format!(
                    "„{}“ gibt es schon – bitte anderen Ordner wählen.",
                    existing.display()
                ));
            }
            return;
        }
        let Some(d) = &self.document else { return };
        let snapshot = match d.doc.snapshot_pdf() {
            Ok(s) => s,
            Err(e) => {
                self.error = Some(format!("Teilen nicht möglich:\n{e}"));
                return;
            }
        };
        self.split_dialog = None;
        let total = groups.len();
        self.job = Some(crate::jobs::Job::spawn(
            "Dokument teilen",
            ctx,
            move |job| {
                for (i, (pages, target)) in groups.iter().zip(&targets).enumerate() {
                    if job.cancelled() {
                        return Err(format!(
                            "Abgebrochen – {i} von {total} Dateien geschrieben."
                        ));
                    }
                    job.progress(
                        i as f32 / total as f32,
                        format!("Datei {} von {total} …", i + 1),
                    );
                    let bytes =
                        pdftool_core::extract_pages(&snapshot, pages).map_err(|e| e.to_string())?;
                    crate::platform::write(target, &bytes)
                        .map_err(|e| format!("„{}“: {e}", target.display()))?;
                }
                Ok(format!(
                    "{total} Dateien in „{}“ geschrieben.",
                    folder.display()
                ))
            },
        ));
    }
}

#[cfg(test)]
mod range_tests {
    use super::format_ranges;

    #[test]
    fn formats_ranges() {
        assert_eq!(format_ranges(&[0, 1, 2, 4, 6, 7]), "1-3, 5, 7-8");
        assert_eq!(format_ranges(&[3]), "4");
        assert_eq!(format_ranges(&[]), "");
    }
}
