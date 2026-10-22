//! Menüleiste: Datei, Bearbeiten, Ansicht.

use crate::app::FolioApp;
use crate::pages::PageAction;
use crate::viewer::ZoomMode;
use eframe::egui::{self, Button, ThemePreference};

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
                    self.save_document(false);
                }
                if ui
                    .add_enabled(
                        has_doc,
                        Button::new("Speichern unter…").shortcut_text("Strg+Umschalt+S"),
                    )
                    .clicked()
                {
                    self.save_document(true);
                }
                if ui.button("Dateien zusammenführen…").clicked() {
                    self.open_merge_dialog();
                }
                ui.separator();
                ui.menu_button("Zuletzt geöffnet", |ui| {
                    if self.settings.recent_files.is_empty() {
                        ui.label("(leer)");
                    }
                    let mut open = None;
                    for path in &self.settings.recent_files {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default();
                        if ui
                            .button(name)
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            open = Some(path.clone());
                        }
                    }
                    if let Some(path) = open {
                        self.request_open(&ctx, path);
                    }
                    if !self.settings.recent_files.is_empty() {
                        ui.separator();
                        if ui.button("Liste leeren").clicked() {
                            self.settings.recent_files.clear();
                        }
                    }
                });
                if ui.button("Einstellungen…").clicked() {
                    self.show_settings_window = true;
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

            ui.menu_button("Seiten", |ui| {
                if self.document.is_none() {
                    ui.label("Kein Dokument");
                    return;
                }
                if ui.button("Seiten aus Datei einfügen…").clicked() {
                    self.start_insert_from_file();
                }
                if ui.button("Leere Seite einfügen").clicked() {
                    self.insert_blank_page();
                }
                ui.separator();
                let Some(d) = &mut self.document else { return };
                let target = d.target_pages();
                let n = d.doc.page_count();
                let what = if target.len() == 1 {
                    format!("Seite {}", target[0] + 1)
                } else {
                    format!("{} ausgewählte Seiten", target.len())
                };
                ui.label(egui::RichText::new(what).weak());
                if ui.button("Im Uhrzeigersinn drehen").clicked() {
                    d.apply_page_action(PageAction::Rotate(target.clone(), 1));
                }
                if ui.button("Gegen den Uhrzeigersinn drehen").clicked() {
                    d.apply_page_action(PageAction::Rotate(target.clone(), -1));
                }
                ui.separator();
                if ui
                    .add_enabled(target.len() < n, Button::new("Löschen"))
                    .on_disabled_hover_text("Mindestens eine Seite muss bleiben")
                    .clicked()
                {
                    d.apply_page_action(PageAction::Delete(target));
                }
            });

            ui.menu_button("Ansicht", |ui| {
                ui.checkbox(&mut self.show_thumbnails, "Seitenleiste");
                ui.checkbox(&mut self.show_comments, "Kommentare");
                ui.separator();
                ui.label("Design");
                self.theme_radios(ui);
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

    fn theme_radios(&mut self, ui: &mut egui::Ui) {
        let before = self.settings.theme;
        ui.radio_value(
            &mut self.settings.theme,
            ThemePreference::System,
            "Wie System",
        );
        ui.radio_value(&mut self.settings.theme, ThemePreference::Light, "Hell");
        ui.radio_value(&mut self.settings.theme, ThemePreference::Dark, "Dunkel");
        if self.settings.theme != before {
            ui.ctx().set_theme(self.settings.theme);
        }
    }

    /// Fenster "Einstellungen".
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings_window;
        egui::Window::new("Einstellungen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Autorname");
                        ui.text_edit_singleline(&mut self.tool_settings.author)
                            .on_hover_text("Wird in neuen Kommentaren als Autor (/T) gespeichert");
                        ui.end_row();
                        ui.label("Design");
                        ui.vertical(|ui| self.theme_radios(ui));
                        ui.end_row();
                    });
            });
        self.show_settings_window = open;
    }
}
