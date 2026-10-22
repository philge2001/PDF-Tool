//! Speichern, "Speichern unter", Rückfrage bei ungespeicherten Änderungen, Fenstertitel.

use crate::app::{FolioApp, OpenDocument};
use eframe::egui;
use foliocraft_render::RenderService;
use std::path::PathBuf;

/// Was nach der Rückfrage "Änderungen speichern?" passieren soll.
pub enum PendingAction {
    Close,
    Open(PathBuf),
}

impl OpenDocument {
    /// Nach dem Speichern: Worker mit den neuen Bytes neu laden. Die schon gerenderten
    /// Seitenbilder bleiben gültig (die Seiten selbst haben sich nicht geändert).
    pub fn reload_renderer(&mut self, render: &RenderService) {
        render.open(
            self.viewer.doc_id,
            self.doc.bytes().clone(),
            crate::app::hidden_annots(&self.doc),
        );
        // Der Worker hat seine Warteschlange geleert -> offene Aufträge neu anfordern.
        self.viewer.reset_pending();
        self.texts.reset_requested();
    }

    pub fn file_name(&self) -> String {
        self.doc
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unbenannt.pdf".into())
    }
}

impl FolioApp {
    /// Speichert das Dokument. `force_dialog` = "Speichern unter". Gibt `true` bei Erfolg zurück.
    pub(crate) fn save_document(&mut self, force_dialog: bool) -> bool {
        let Some(d) = &mut self.document else {
            return false;
        };
        let target = match d.doc.path() {
            Some(p) if !force_dialog => p.to_path_buf(),
            current => {
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("PDF", &["pdf"])
                    .set_file_name(d.file_name());
                if let Some(dir) = current.and_then(|p| p.parent()) {
                    dialog = dialog.set_directory(dir);
                }
                match dialog.save_file() {
                    Some(p) => p,
                    None => return false, // abgebrochen
                }
            }
        };
        match d.doc.save_as(&target) {
            Ok(()) => {
                if let Ok(render) = &self.render {
                    d.reload_renderer(render);
                }
                self.settings.add_recent(&target);
                true
            }
            Err(e) => {
                self.error = Some(format!(
                    "Speichern nach „{}“ fehlgeschlagen:\n{e}",
                    target.display()
                ));
                false
            }
        }
    }

    /// Öffnet eine Datei – fragt vorher nach, wenn es ungespeicherte Änderungen gibt.
    pub(crate) fn request_open(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.document.as_ref().is_some_and(|d| d.doc.is_dirty()) {
            self.pending = Some(PendingAction::Open(path));
        } else {
            self.open_path(ctx, &path);
        }
    }

    /// Fängt das Schließen des Fensters ab, solange es ungespeicherte Änderungen gibt.
    pub(crate) fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if self.document.as_ref().is_some_and(|d| d.doc.is_dirty()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(PendingAction::Close);
        }
    }

    /// Dialog "Änderungen speichern?".
    pub(crate) fn unsaved_changes_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        let name = self
            .document
            .as_ref()
            .map(|d| d.file_name())
            .unwrap_or_default();
        #[derive(PartialEq)]
        enum Choice {
            None,
            Save,
            Discard,
            Cancel,
        }
        let mut choice = Choice::None;
        egui::Modal::new(egui::Id::unique("unsaved_changes")).show(ctx, |ui| {
            ui.heading("Ungespeicherte Änderungen");
            ui.label(format!(
                "Sollen die Änderungen an „{name}“ gespeichert werden?"
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Speichern").clicked() {
                    choice = Choice::Save;
                }
                if ui.button("Nicht speichern").clicked() {
                    choice = Choice::Discard;
                }
                if ui.button("Abbrechen").clicked() {
                    choice = Choice::Cancel;
                }
            });
        });
        let proceed = match choice {
            Choice::None => return,
            Choice::Cancel => false,
            Choice::Save => self.save_document(false), // bei Fehler/Abbruch im Dialog: nicht fortfahren
            Choice::Discard => true,
        };
        let action = self.pending.take();
        if !proceed {
            return;
        }
        match action {
            Some(PendingAction::Close) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(PendingAction::Open(path)) => self.open_path(ctx, &path),
            None => {}
        }
    }

    /// Fenstertitel mit "*" bei ungespeicherten Änderungen. Wird nur bei Änderung gesendet.
    pub(crate) fn update_title(&mut self, ctx: &egui::Context) {
        let title = match &self.document {
            Some(d) => {
                let dirty = if d.doc.is_dirty() { "*" } else { "" };
                format!("{}{dirty} – FolioCraft", d.file_name())
            }
            None => "FolioCraft".to_string(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }
}
