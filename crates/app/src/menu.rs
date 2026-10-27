//! Inhalt des Knopfs „Menü“ (Datei-Befehle, Untermenüs Bearbeiten, Seiten, … Ansicht).

use crate::app::PdfToolApp;
use crate::pages::PageAction;
use crate::shell::RightPane;
use crate::tools::Tool;
use crate::viewer::ZoomMode;
use eframe::egui::{self, Button, ThemePreference};

impl PdfToolApp {
    /// Inhalt des Knopfs "Menü": Datei-Befehle direkt, der Rest als Untermenüs.
    pub(crate) fn main_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let ctx = ctx.clone();
        {
            {
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
                if ui
                    .add_enabled(has_doc, Button::new("Exportieren…"))
                    .on_hover_text("Seiten als PNG/JPEG, Text (TXT) oder Word (DOCX)")
                    .clicked()
                {
                    self.open_export_dialog();
                }
                if ui
                    .add_enabled(has_doc, Button::new("Sicherheit…"))
                    .on_hover_text("Passwortschutz und Berechtigungen")
                    .clicked()
                {
                    self.open_security_dialog();
                }
                if ui
                    .add_enabled(has_doc, Button::new("Komprimieren…"))
                    .on_hover_text("Datei verkleinern (Bilder, alte Fassungen, Objekt-Streams)")
                    .clicked()
                {
                    self.open_compress_dialog();
                }
                if ui
                    .add_enabled(has_doc, Button::new("Nach PDF/A umwandeln"))
                    .on_hover_text(
                        "Für die Langzeitarchivierung: Schriften einbetten, Farbprofil, Metadaten",
                    )
                    .clicked()
                {
                    self.start_pdfa(ui.ctx());
                }
                if ui
                    .add_enabled(has_doc, Button::new("Vergleichen mit…"))
                    .on_hover_text("Textunterschiede zu einer anderen PDF anzeigen")
                    .clicked()
                {
                    self.start_compare(ui.ctx());
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
            }

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
                    self.start_insert_from_file(ui.ctx());
                }
                if ui.button("Leere Seite einfügen").clicked() {
                    self.insert_blank_page();
                }
                ui.separator();
                if ui.button("Seiten extrahieren…").clicked() {
                    self.open_extract_dialog();
                }
                if ui.button("Dokument teilen…").clicked() {
                    self.open_split_dialog();
                }
                ui.separator();
                #[cfg(has_ocr)]
                if crate::ocr_ui::available() {
                    if ui
                        .button("Texterkennung (OCR)…")
                        .on_hover_text("Gescannte Seiten durchsuchbar machen")
                        .clicked()
                    {
                        self.open_ocr_dialog();
                    }
                } else {
                    ui.add_enabled(false, Button::new("Texterkennung (OCR)…"))
                        .on_disabled_hover_text(
                            "Texterkennung ist auf dieser Seite nicht eingebunden",
                        );
                }
                #[cfg(not(has_ocr))]
                ui.add_enabled(false, Button::new("Texterkennung (OCR)…"))
                    .on_disabled_hover_text("Diese Version wurde ohne Texterkennung gebaut");
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

            ui.menu_button("Formular", |ui| {
                let Some(d) = &mut self.document else {
                    ui.label("Kein Dokument");
                    return;
                };
                let has_fields = d.doc.forms().iter().any(|f| f.kind.is_fillable());
                if !has_fields {
                    ui.label("Dieses Dokument enthält keine Formularfelder.");
                }
                ui.add_enabled_ui(has_fields, |ui| {
                    ui.checkbox(&mut self.settings.highlight_fields, "Felder hervorheben");
                    if ui.button("Formular zurücksetzen").clicked() {
                        d.reset_form();
                        ui.close();
                    }
                });
            });
            ui.menu_button("Schwärzen", |ui| {
                if self.document.is_none() {
                    ui.label("Kein Dokument");
                    return;
                }
                for tool in Tool::REDACT {
                    if ui
                        .selectable_label(self.tool == tool, tool.label())
                        .clicked()
                    {
                        self.tool = tool;
                        ui.close();
                    }
                }
                let pending = self.document.as_ref().map_or(0, |d| d.pending_redactions());
                ui.label(egui::RichText::new(format!("{pending} Schwärzung(en) markiert")).weak());
                ui.separator();
                if ui
                    .add_enabled(pending > 0, Button::new("Schwärzungen anwenden…"))
                    .clicked()
                {
                    self.redact_confirm = true;
                    ui.close();
                }
                ui.separator();
                if ui
                    .button("Suchtreffer schwärzen…")
                    .on_hover_text("Öffnet die Suche; dort „Alle Treffer schwärzen“")
                    .clicked()
                {
                    if let Some(d) = &mut self.document {
                        d.search.open();
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button("Dokument bereinigen…").clicked() {
                    self.open_sanitize_dialog();
                    ui.close();
                }
            });
            ui.menu_button("Signieren", |ui| {
                if ui
                    .add_enabled(self.document.is_some(), Button::new("Dokument signieren…"))
                    .clicked()
                {
                    self.open_sign_dialog();
                    ui.close();
                }
                if ui.button("Testzertifikat erstellen…").clicked() {
                    self.open_cert_dialog();
                    ui.close();
                }
                ui.separator();
                if ui.button("Signaturen anzeigen").clicked() {
                    self.right_pane = Some(RightPane::Signatures);
                }
            });
            ui.menu_button("Barrierefreiheit", |ui| {
                let has_doc = self.document.is_some();
                if ui.add_enabled(has_doc, Button::new("Prüfen…")).clicked() {
                    self.open_a11y_report();
                    ui.close();
                }
                if ui
                    .add_enabled(has_doc, Button::new("Automatisch taggen…"))
                    .clicked()
                {
                    self.open_autotag_dialog();
                    ui.close();
                }
                ui.separator();
                if ui.button("Panel „Tags“").clicked() {
                    self.right_pane = Some(RightPane::Tags);
                }
            });
            ui.menu_button("Ansicht", |ui| {
                ui.label("Panel rechts");
                ui.radio_value(&mut self.right_pane, None, "Keins");
                for pane in RightPane::ALL {
                    ui.radio_value(&mut self.right_pane, Some(pane), pane.label());
                }
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
        }
        // Im Browser schließt man den Tab; ein Fenster zum Beenden gibt es dort nicht.
        if !crate::platform::is_web() {
            ui.separator();
            if ui.button("Beenden").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
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
                ui.separator();
                self.trust_settings(ui);
            });
        self.show_settings_window = open;
    }
}
