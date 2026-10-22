//! Menüleiste: Datei, Bearbeiten, Ansicht.

use crate::app::FolioApp;
use crate::viewer::ZoomMode;
use eframe::egui::{self, Button};

impl FolioApp {
    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Datei", |ui| {
                if ui
                    .add(Button::new("Öffnen…").shortcut_text("Strg+O"))
                    .clicked()
                {
                    self.open_dialog(&ctx);
                }
                let has_doc = self.document.is_some();
                let dirty = self.document.as_ref().is_some_and(|d| d.doc.is_dirty());
                if ui
                    .add_enabled(dirty, Button::new("Speichern").shortcut_text("Strg+S"))
                    .clicked()
                {
                    self.save(false);
                }
                if ui
                    .add_enabled(
                        has_doc,
                        Button::new("Speichern unter…").shortcut_text("Strg+Umschalt+S"),
                    )
                    .clicked()
                {
                    self.save(true);
                }
                ui.separator();
                if ui.button("Beenden").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.menu_button("Bearbeiten", |ui| {
                let Some(d) = &mut self.document else {
                    ui.label("Kein Dokument");
                    return;
                };
                let undo = d.doc.history().undo_label();
                let text = format!("Rückgängig: {}", undo.clone().unwrap_or_default());
                if ui
                    .add_enabled(undo.is_some(), Button::new(text).shortcut_text("Strg+Z"))
                    .clicked()
                {
                    d.undo();
                }
                let redo = d.doc.history().redo_label();
                let text = format!("Wiederholen: {}", redo.clone().unwrap_or_default());
                if ui
                    .add_enabled(redo.is_some(), Button::new(text).shortcut_text("Strg+Y"))
                    .clicked()
                {
                    d.redo();
                }
                ui.separator();
                let has_text = d.selected_text().is_some_and(|t| !t.is_empty());
                if ui
                    .add_enabled(has_text, Button::new("Kopieren").shortcut_text("Strg+C"))
                    .clicked()
                {
                    d.copy_selection(&ctx);
                }
                if ui
                    .add_enabled(
                        d.selected.is_some(),
                        Button::new("Annotation löschen").shortcut_text("Entf"),
                    )
                    .clicked()
                {
                    d.delete_selected();
                }
                ui.separator();
                if ui
                    .add(Button::new("Suchen…").shortcut_text("Strg+F"))
                    .clicked()
                {
                    d.search.open();
                }
            });

            ui.menu_button("Ansicht", |ui| {
                ui.checkbox(&mut self.show_thumbnails, "Seitenleiste");
                ui.checkbox(&mut self.show_comments, "Kommentare");
                if let Some(d) = &mut self.document {
                    ui.separator();
                    if ui
                        .add(Button::new("Originalgröße").shortcut_text("Strg+1"))
                        .clicked()
                    {
                        d.viewer.zoom_custom(1.0, None);
                    }
                    if ui
                        .add(Button::new("Seitenbreite").shortcut_text("Strg+2"))
                        .clicked()
                    {
                        d.viewer.set_zoom_mode(ZoomMode::FitWidth);
                    }
                    if ui
                        .add(Button::new("Ganze Seite").shortcut_text("Strg+0"))
                        .clicked()
                    {
                        d.viewer.set_zoom_mode(ZoomMode::FitPage);
                    }
                }
            });
        });
    }
}
