//! Seitenleiste mit Miniaturansichten. Nutzt denselben Texturcache wie die Hauptansicht,
//! aber mit niedrigster Render-Priorität.

use super::layout::Layout;
use super::{Viewer, paint_texture, render_scale_milli};
use eframe::egui::{
    self, Align2, Color32, FontId, Rect, ScrollArea, Sense, Stroke, StrokeKind, vec2,
};
use foliocraft_render::{Priority, RenderKey};

/// Breite der breitesten Miniatur in egui-Punkten.
const THUMB_WIDTH: f32 = 120.0;
/// Platz für die Seitennummer unter jeder Miniatur.
const THUMB_GAP: f32 = 24.0;

/// Zustand der Seitenleiste (eigenes Layout mit größerem Abstand).
pub struct Thumbnails {
    layout: Layout,
    /// Für welche Seite wir zuletzt automatisch mitgescrollt haben.
    followed_page: Option<usize>,
}

impl Thumbnails {
    pub fn new(page_sizes: &[(f32, f32)]) -> Self {
        Self {
            layout: Layout::with_gap(page_sizes.to_vec(), THUMB_GAP),
            followed_page: None,
        }
    }
}

impl Viewer {
    /// Zeichnet die Miniaturen. Gibt die angeklickte Seite zurück.
    pub fn show_thumbnails(&mut self, ui: &mut egui::Ui, thumbs: &mut Thumbnails) -> Option<usize> {
        let zoom = THUMB_WIDTH / thumbs.layout.max_width_pt().max(1.0);
        let ppp = ui.ctx().pixels_per_point();
        let scale_milli = render_scale_milli(zoom * ppp);
        let mut clicked = None;

        let area = ScrollArea::vertical()
            .id_salt("thumbnails")
            .auto_shrink(false);
        // Hauptansicht hat die Seite gewechselt -> Miniatur in Sicht bringen.
        let follow = thumbs.followed_page != Some(self.current_page);
        if follow {
            thumbs.followed_page = Some(self.current_page);
        }

        area.show_viewport(ui, |ui, viewport| {
            let content = thumbs.layout.content_size(zoom, viewport.width());
            let (resp, painter) = ui.allocate_painter(content, Sense::click());
            let origin = resp.rect.min.to_vec2();

            for i in
                thumbs
                    .layout
                    .visible_range(viewport.min.y - 200.0, viewport.max.y + 200.0, zoom)
            {
                let rect = thumbs
                    .layout
                    .page_rect(i, zoom, content.x)
                    .translate(origin);
                painter.rect_filled(rect, 0.0, Color32::WHITE);

                let page_pt = self.layout.page_size_pt(i);
                let key = RenderKey {
                    doc: self.doc_id,
                    page: i as u32,
                    scale_milli,
                    tile: None,
                };
                self.frame
                    .need(&self.textures, key, page_pt, Priority::Thumbnail);
                if let Some(tex) = self.textures.get(&key) {
                    paint_texture(&painter, tex.id(), rect);
                } else if let Some(tex) = self.textures.best_fallback(self.doc_id, i as u32) {
                    paint_texture(&painter, tex.id(), rect);
                }

                let is_current = i == self.current_page;
                let stroke = if is_current {
                    Stroke::new(3.0, ui.visuals().selection.bg_fill)
                } else {
                    Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color)
                };
                painter.rect_stroke(rect, 0.0, stroke, StrokeKind::Outside);
                painter.text(
                    rect.center_bottom() + vec2(0.0, 4.0),
                    Align2::CENTER_TOP,
                    (i + 1).to_string(),
                    FontId::proportional(12.0),
                    ui.visuals().text_color(),
                );
            }

            if resp.clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                clicked = Some(thumbs.layout.page_at(pos.y - origin.y, zoom));
            }

            if follow {
                let r = thumbs
                    .layout
                    .page_rect(self.current_page, zoom, content.x)
                    .translate(origin);
                let r = Rect::from_min_max(r.min, r.max + vec2(0.0, THUMB_GAP));
                ui.scroll_to_rect(r, None);
            }
        });
        clicked
    }
}
