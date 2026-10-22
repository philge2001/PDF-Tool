//! Obere Werkzeugleiste: Öffnen, Seitennavigation, Zoom.

use crate::app::FolioApp;
use crate::viewer::{ZOOM_STEPS, ZoomMode};
use eframe::egui;

impl FolioApp {
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            if ui.button("Öffnen…").on_hover_text("Strg+O").clicked() {
                self.open_dialog(&ctx);
            }
            ui.toggle_value(&mut self.show_thumbnails, "Seiten")
                .on_hover_text("Seitenleiste mit Miniaturen ein-/ausblenden");

            ui.separator();
            for tool in crate::tools::Tool::ALL {
                ui.selectable_value(&mut self.tool, tool, tool.label());
            }

            let Some(d) = &mut self.document else { return };
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
}
