//! Obere Werkzeugleiste: Werkzeuge, Rückgängig/Wiederholen, Seitennavigation, Zoom.

use crate::app::FolioApp;
use crate::tools::Tool;
use crate::viewer::{ZOOM_STEPS, ZoomMode};
use eframe::egui;

impl FolioApp {
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for tool in Tool::NAVIGATION {
                ui.selectable_value(&mut self.tool, tool, tool.label());
            }

            let Some(d) = &mut self.document else { return };
            ui.separator();

            // ----- Rückgängig / Wiederholen -----
            let undo_label = d.doc.history().undo_label();
            let resp = ui.add_enabled(undo_label.is_some(), egui::Button::new("Rückgängig"));
            if resp
                .on_hover_text(format!("{} (Strg+Z)", undo_label.unwrap_or_default()))
                .clicked()
            {
                d.undo();
            }
            let redo_label = d.doc.history().redo_label();
            let resp = ui.add_enabled(redo_label.is_some(), egui::Button::new("Wiederholen"));
            if resp
                .on_hover_text(format!("{} (Strg+Y)", redo_label.unwrap_or_default()))
                .clicked()
            {
                d.redo();
            }
            ui.separator();

            // ----- Seitennavigation -----
            let page_count = d.doc.page_count();
            if ui
                .button("‹")
                .on_hover_text("Vorherige Seite (Bild ↑)")
                .clicked()
            {
                d.viewer
                    .scroll_to_page(d.viewer.current_page.saturating_sub(1));
            }
            // Das Feld zeigt die aktuelle Seite, solange der Nutzer nicht hineintippt.
            // `get_or_insert_with` legt den Text beim ersten Fokus an.
            let current = (d.viewer.current_page + 1).to_string();
            let mut text = d.page_input.clone().unwrap_or(current);
            let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(40.0));
            if resp.has_focus() || resp.gained_focus() {
                d.page_input = Some(text.clone());
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Ok(n) = text.trim().parse::<usize>()
                {
                    d.viewer.scroll_to_page(n.clamp(1, page_count) - 1);
                }
                d.page_input = None;
            }
            ui.label(format!("/ {page_count}"));
            if ui
                .button("›")
                .on_hover_text("Nächste Seite (Bild ↓)")
                .clicked()
            {
                d.viewer.scroll_to_page(d.viewer.current_page + 1);
            }

            ui.separator();

            // ----- Zoom -----
            if ui
                .button("−")
                .on_hover_text("Verkleinern (Strg+−)")
                .clicked()
            {
                d.viewer.zoom_step(false);
            }
            let current = format!("{:.0}", d.viewer.zoom() * 100.0);
            let mut text = d.zoom_input.clone().unwrap_or(current);
            let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(48.0));
            if resp.has_focus() || resp.gained_focus() {
                d.zoom_input = Some(text.clone());
            }
            if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && let Ok(p) = text.trim().trim_end_matches('%').trim().parse::<f32>()
                {
                    d.viewer.zoom_custom(p / 100.0, None);
                }
                d.zoom_input = None;
            }
            ui.label("%");
            if ui
                .button("+")
                .on_hover_text("Vergrößern (Strg++)")
                .clicked()
            {
                d.viewer.zoom_step(true);
            }

            let mode_label = match d.viewer.zoom_mode() {
                ZoomMode::Custom => "Zoom",
                ZoomMode::FitWidth => "Seitenbreite",
                ZoomMode::FitPage => "Ganze Seite",
            };
            egui::ComboBox::from_id_salt("zoom_mode")
                .selected_text(mode_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            d.viewer.zoom_mode() == ZoomMode::FitWidth,
                            "Seitenbreite (Strg+2)",
                        )
                        .clicked()
                    {
                        d.viewer.set_zoom_mode(ZoomMode::FitWidth);
                    }
                    if ui
                        .selectable_label(
                            d.viewer.zoom_mode() == ZoomMode::FitPage,
                            "Ganze Seite (Strg+0)",
                        )
                        .clicked()
                    {
                        d.viewer.set_zoom_mode(ZoomMode::FitPage);
                    }
                    ui.separator();
                    for step in ZOOM_STEPS {
                        let label = format!("{:.0} %", step * 100.0);
                        if ui.selectable_label(false, label).clicked() {
                            d.viewer.zoom_custom(step, None);
                        }
                    }
                });
        });
    }

    /// Zweite Leiste: Annotationswerkzeuge und deren Stil.
    pub(crate) fn annotation_bar(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &mut self.document else { return };
        ui.horizontal(|ui| {
            ui.label("Kommentieren:");
            for tool in Tool::ANNOTATION {
                if matches!(tool, Tool::Note | Tool::Ink) {
                    ui.separator(); // Gruppen: Text-Markup | Notiz/Freitext | Zeichnen
                }
                let selected = self.tool == tool;
                if ui.selectable_label(selected, tool.label()).clicked() {
                    // Markup-Werkzeug bei vorhandener Textauswahl: sofort anwenden.
                    let applied = tool.is_text_markup()
                        && d.create_markup_from_selection(tool, &self.tool_settings);
                    if !applied {
                        // Erneuter Klick auf das aktive Werkzeug schaltet zurück auf Textauswahl.
                        self.tool = if selected { Tool::TextSelect } else { tool };
                    }
                }
            }

            // Ausgewählte Annotation: deren Farbe ändern / löschen.
            if let Some(a) = d
                .selected
                .and_then(|id| d.doc.annotations().get(id))
                .cloned()
            {
                ui.separator();
                ui.label(format!("Auswahl: {}", a.kind.label()));
                let mut rgb = a.color.to_rgb8();
                if ui
                    .color_edit_button_srgb(&mut rgb)
                    .on_hover_text("Farbe der Auswahl")
                    .changed()
                {
                    d.set_selected_color(foliocraft_core::Color::from_rgb8(rgb));
                }
                if ui.button("Löschen").on_hover_text("Entf").clicked() {
                    d.delete_selected();
                }
                return;
            }

            // Stil des aktiven Werkzeugs
            if !Tool::ANNOTATION.contains(&self.tool) {
                return;
            }
            ui.separator();
            let s = &mut self.tool_settings;
            let mut rgb = s.color(self.tool).to_rgb8();
            if ui
                .color_edit_button_srgb(&mut rgb)
                .on_hover_text("Farbe")
                .changed()
            {
                s.set_color(self.tool, foliocraft_core::Color::from_rgb8(rgb));
            }
            if self.tool.uses_stroke_width() {
                ui.label("Linie");
                ui.add(
                    egui::DragValue::new(&mut s.stroke_width)
                        .range(0.5..=20.0)
                        .speed(0.1)
                        .suffix(" pt"),
                );
            }
            if self.tool == Tool::FreeText {
                ui.label("Schrift");
                ui.add(
                    egui::DragValue::new(&mut s.font_size)
                        .range(6.0..=72.0)
                        .speed(0.2)
                        .suffix(" pt"),
                );
            }
        });
    }
}
