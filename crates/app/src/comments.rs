//! Kommentar-Panel: Liste aller Annotationen, nach Seite gruppiert.

use crate::annot_draw::color32;
use crate::app::OpenDocument;
use crate::viewer::transform;
use eframe::egui::{self, RichText, Sense, Stroke, StrokeKind, vec2};
use pdftool_core::{AnnotId, Annotation, Color};

/// "D:20261022103045Z" -> "22.10.2026 10:30" (Zeitzone wird nicht umgerechnet).
pub fn format_pdf_date(d: &str) -> String {
    let digits: String = d.trim_start_matches("D:").chars().take(12).collect();
    if digits.len() < 8 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return String::new();
    }
    let part = |a: usize, b: usize| digits.get(a..b).unwrap_or("");
    let mut out = format!("{}.{}.{}", part(6, 8), part(4, 6), part(0, 4));
    if digits.len() >= 12 {
        out += &format!(" {}:{}", part(8, 10), part(10, 12));
    }
    out
}

impl OpenDocument {
    /// Springt zur Annotation und wählt sie aus.
    pub fn focus_annotation(&mut self, id: AnnotId) {
        self.selected = Some(id);
        let Some(a) = self.doc.annotations().get(id) else {
            return;
        };
        if let Ok(info) = self.doc.page(a.page) {
            let unit = transform::pdf_rect_to_unit(info, a.bounds());
            self.viewer.reveal(a.page, unit);
        }
    }

    pub fn comments_panel(&mut self, ui: &mut egui::Ui) {
        // Lesereihenfolge: Seite, dann von oben nach unten, dann von links nach rechts.
        // (Kopien, damit wir beim Zeichnen das Modell ändern dürfen.)
        let mut items: Vec<Annotation> = self.doc.annotations().iter().cloned().collect();
        items.sort_by(|a, b| {
            let (ra, rb) = (a.bounds(), b.bounds());
            a.page
                .cmp(&b.page)
                .then(rb.y1.total_cmp(&ra.y1))
                .then(ra.x0.total_cmp(&rb.x0))
        });

        ui.horizontal(|ui| {
            ui.heading("Kommentare");
            ui.label(format!("({})", items.len()));
        });
        ui.separator();
        if items.is_empty() {
            ui.label("Noch keine Kommentare. Werkzeuge findest du in der Leiste \"Kommentieren\".");
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                let mut last_page = None;
                for a in &items {
                    if last_page != Some(a.page) {
                        last_page = Some(a.page);
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("Seite {}", a.page + 1)).strong());
                    }
                    self.comment_entry(ui, a);
                }
            });
    }

    fn comment_entry(&mut self, ui: &mut egui::Ui, a: &Annotation) {
        let selected = self.selected == Some(a.id);
        let frame = egui::Frame::group(ui.style()).fill(if selected {
            ui.visuals().selection.bg_fill.gamma_multiply(0.35)
        } else {
            ui.visuals().faint_bg_color
        });
        let inner = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // Farbfeld
                let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, color32(a.color, 1.0));
                ui.painter().rect_stroke(
                    r,
                    2.0,
                    Stroke::new(1.0, ui.visuals().text_color()),
                    StrokeKind::Inside,
                );
                ui.label(RichText::new(a.kind.label()).strong());
                if !a.author.is_empty() {
                    ui.label(RichText::new(&a.author).weak());
                }
            });
            let date = format_pdf_date(&a.modified);
            if !date.is_empty() {
                ui.label(RichText::new(date).small().weak());
            }

            if selected {
                // Direkt bearbeiten: jede Änderung wird sofort (zusammengeführt) übernommen.
                let mut text = a.contents.clone();
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text("Kommentar hinzufügen …"),
                );
                if resp.changed() {
                    self.update_selected(|x| x.contents = text);
                }
                ui.horizontal(|ui| {
                    let mut rgb = a.color.to_rgb8();
                    if ui
                        .color_edit_button_srgb(&mut rgb)
                        .on_hover_text("Farbe")
                        .changed()
                    {
                        self.set_selected_color(Color::from_rgb8(rgb));
                    }
                    if ui.button("Löschen").on_hover_text("Entf").clicked() {
                        self.delete_selected();
                    }
                });
            } else if !a.contents.is_empty() {
                let preview: String = a.contents.chars().take(120).collect();
                let more = if a.contents.chars().count() > 120 {
                    " …"
                } else {
                    ""
                };
                ui.label(format!("{preview}{more}"));
            }
        });
        // Klick auf einen nicht ausgewählten Eintrag wählt ihn aus und springt hin.
        // (Nur dann: Beim ausgewählten Eintrag würde diese Klickfläche – sie liegt "oben" –
        // dem Textfeld und den Knöpfen die Klicks wegnehmen.)
        if !selected {
            let id = ui.scope_id().with(("comment", a.id.0));
            if ui
                .interact(inner.response.rect, id, Sense::click())
                .clicked()
            {
                self.focus_annotation(a.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_dates() {
        assert_eq!(format_pdf_date("D:20261022103045Z"), "22.10.2026 10:30");
        assert_eq!(format_pdf_date("D:20261022"), "22.10.2026");
        assert_eq!(format_pdf_date("kaputt"), "");
    }
}
