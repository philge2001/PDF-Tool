//! Dialoge für Seitenorganisation: Seiten aus Datei einfügen, Dateien zusammenführen.

use crate::app::FolioApp;
use eframe::egui;
use foliocraft_core::{Document, parse_page_ranges};
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

fn pick_pdfs() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("PDF", &["pdf"])
        .pick_files()
        .unwrap_or_default()
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl FolioApp {
    /// Datei wählen und den Einfüge-Dialog öffnen.
    pub(crate) fn start_insert_from_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .pick_file()
        else {
            return;
        };
        match Document::open(&path) {
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
                        dlg.files.extend(pick_pdfs());
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
    let err = |p: &PathBuf, e: foliocraft_core::CoreError| format!("„{}“: {e}", file_name(p));
    let (first, rest) = files.split_first().ok_or("Keine Dateien ausgewählt")?;
    let mut doc = Document::open(first).map_err(|e| err(first, e))?;
    doc.forget_path();
    for p in rest {
        let source = Document::open(p).map_err(|e| err(p, e))?;
        let all: Vec<usize> = (0..source.page_count()).collect();
        let at = doc.page_count();
        let cmd = doc
            .prepare_insert(&source, &all, at)
            .map_err(|e| err(p, e))?;
        doc.execute(Box::new(cmd)).map_err(|e| err(p, e))?;
    }
    Ok(doc)
}

impl FolioApp {
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
