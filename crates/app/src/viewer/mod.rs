//! Die Dokumentansicht: scrollbare Seitenliste, Render-Aufträge, Texturen.

pub mod layout;
pub mod textures;

use eframe::egui::{self, Color32, Rect, ScrollArea, Sense, pos2, vec2};
use foliocraft_render::{
    DocId, Priority, RenderKey, RenderRequest, RenderService, needs_tiles, page_pixel_size,
    tile_grid, tile_rect,
};
use layout::Layout;
use std::collections::HashSet;
use textures::PageTextures;

/// Wie weit über/unter dem sichtbaren Bereich vorausgerendert wird (in Bildschirmhöhen).
const PREFETCH_SCREENS: f32 = 0.5;

/// Maximale Kantenlänge der Vorschau, die bei gekachelten Seiten als Platzhalter dient.
const PREVIEW_MAX_PX: f32 = 1024.0;

pub struct Viewer {
    pub doc_id: DocId,
    pub layout: Layout,
    /// 1.0 = 100 % (1 PDF-Punkt = 1 egui-Punkt).
    pub zoom: f32,
    pub current_page: usize,
    textures: PageTextures,
    /// Vom Code gewünschte Scrollposition (wird im nächsten Frame angewendet).
    scroll_to: Option<f32>,
}

impl Viewer {
    pub fn new(doc_id: DocId, page_sizes: Vec<(f32, f32)>) -> Self {
        Self {
            doc_id,
            layout: Layout::new(page_sizes),
            zoom: 1.0,
            current_page: 0,
            textures: PageTextures::default(),
            scroll_to: None,
        }
    }

    /// Verarbeitet ein Ergebnis des Render-Workers.
    pub fn on_render_event(&mut self, ctx: &egui::Context, event: foliocraft_render::Event) {
        use foliocraft_render::Event;
        match event {
            Event::Rendered { key, image } if key.doc == self.doc_id => {
                self.textures.insert(ctx, key, image)
            }
            Event::RenderFailed { key, error } if key.doc == self.doc_id => {
                tracing::warn!(page = key.page, %error, "Rendern fehlgeschlagen");
                self.textures.mark_failed(&key);
            }
            _ => {}
        }
    }

    pub fn scroll_to_page(&mut self, page: usize) {
        let page = page.min(self.layout.page_count().saturating_sub(1));
        self.scroll_to = Some(self.layout.page_top(page, self.zoom) - layout::GAP);
    }

    pub fn show(&mut self, ui: &mut egui::Ui, render: &RenderService) {
        let ppp = ui.ctx().pixels_per_point();
        let mut area = ScrollArea::both().auto_shrink(false);
        if let Some(y) = self.scroll_to.take() {
            area = area.vertical_scroll_offset(y.max(0.0));
        }

        let mut wanted = HashSet::new();
        let mut requests = Vec::new();

        // `show_viewport` gibt uns den sichtbaren Ausschnitt (in Inhaltskoordinaten),
        // sodass wir nur sichtbare Seiten zeichnen müssen.
        area.show_viewport(ui, |ui, viewport| {
            let content = self.layout.content_size(self.zoom, ui.available_width());
            let (response, painter) = ui.allocate_painter(content, Sense::click_and_drag());
            let origin = response.rect.min.to_vec2();
            let visible_screen = viewport.translate(origin);

            let margin = viewport.height() * PREFETCH_SCREENS;
            let range = self.layout.visible_range(
                viewport.min.y - margin,
                viewport.max.y + margin,
                self.zoom,
            );
            self.current_page = self.layout.page_at(viewport.center().y, self.zoom);

            let scale_milli = render_scale_milli(self.zoom * ppp);
            for i in range {
                let rect = self
                    .layout
                    .page_rect(i, self.zoom, content.x)
                    .translate(origin);
                let on_screen = rect.intersects(visible_screen);
                let priority = if on_screen {
                    Priority::Visible
                } else {
                    Priority::Prefetch
                };

                // Seitenhintergrund mit leichtem Schatten.
                painter.rect_filled(
                    rect.translate(vec2(2.0, 3.0)),
                    0.0,
                    Color32::from_black_alpha(60),
                );
                painter.rect_filled(rect, 0.0, Color32::WHITE);

                let page_pt = self.layout.page_size_pt(i);
                let page_px = page_pixel_size(page_pt, scale_milli as f32 / 1000.0);
                // Das Closure bekommt `textures` als Parameter, statt `self.textures` einzufangen:
                // Eingefangen wäre es für die ganze Lebensdauer des Closures ausgeliehen und die
                // späteren `self.textures.get(..)`-Aufrufe würden am Borrow-Checker scheitern.
                let mut need = |textures: &PageTextures, key: RenderKey, prio: Priority| {
                    wanted.insert(key);
                    if !textures.has(&key) && !textures.is_pending(&key) {
                        requests.push(RenderRequest {
                            key,
                            page_size_pt: page_pt,
                            priority: prio,
                        });
                    }
                };
                let key_for = |scale_milli: u32, tile| RenderKey {
                    doc: self.doc_id,
                    page: i as u32,
                    scale_milli,
                    tile,
                };

                if !needs_tiles(page_px) {
                    let key = key_for(scale_milli, None);
                    need(&self.textures, key, priority);
                    if let Some(tex) = self.textures.get(&key) {
                        paint_texture(&painter, tex.id(), rect);
                    } else if let Some(tex) = self.textures.best_fallback(self.doc_id, i as u32) {
                        paint_texture(&painter, tex.id(), rect);
                    }
                    continue;
                }

                // Gekachelt: zuerst eine Vorschau als Platzhalter, darüber die scharfen Kacheln.
                let preview_scale = PREVIEW_MAX_PX / page_pt.0.max(page_pt.1);
                need(
                    &self.textures,
                    key_for((preview_scale * 1000.0) as u32, None),
                    priority,
                );
                if let Some(tex) = self.textures.best_fallback(self.doc_id, i as u32) {
                    paint_texture(&painter, tex.id(), rect);
                }
                if !on_screen {
                    continue;
                }
                let pt_per_px = rect.width() / page_px.0 as f32;
                let (cols, rows) = tile_grid(page_px);
                for row in 0..rows {
                    for col in 0..cols {
                        let (x, y, w, h) = tile_rect(page_px, (col, row));
                        let tile_rect = Rect::from_min_size(
                            rect.min + vec2(x as f32, y as f32) * pt_per_px,
                            vec2(w as f32, h as f32) * pt_per_px,
                        );
                        if !tile_rect.intersects(visible_screen) {
                            continue;
                        }
                        let key = key_for(scale_milli, Some((col, row)));
                        need(&self.textures, key, Priority::Visible);
                        if let Some(tex) = self.textures.get(&key) {
                            paint_texture(&painter, tex.id(), tile_rect);
                        }
                    }
                }
            }
        });

        // Erst veraltete Aufträge verwerfen, dann neue schicken.
        if let Some(w) = self.textures.update_wanted(wanted) {
            render.retain(w);
        }
        for req in requests {
            self.textures.mark_pending(req.key);
            render.request(req);
        }
    }
}

fn paint_texture(painter: &egui::Painter, id: egui::TextureId, rect: Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.image(id, rect, uv, Color32::WHITE);
}

/// Rundet die gewünschte Skalierung auf eine feste Stufe (Viertel-Oktaven: 2^(k/4)) auf.
/// So erzeugt stufenloses Zoomen nicht für jeden Zwischenwert ein neues Bild, und
/// das Bild ist nie unschärfer als die Anzeige (höchstens ~19 % zu groß).
pub fn render_scale_milli(px_per_pt: f32) -> u32 {
    let steps = (px_per_pt.max(0.05).log2() * 4.0).ceil();
    (2f32.powf(steps / 4.0) * 1000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_buckets_round_up() {
        assert_eq!(render_scale_milli(1.0), 1000);
        assert_eq!(render_scale_milli(2.0), 2000);
        let s = render_scale_milli(1.1);
        assert!((1100..=1200).contains(&s), "{s}");
        // Benachbarte Zoomwerte landen in derselben Stufe.
        assert_eq!(render_scale_milli(1.05), render_scale_milli(1.15));
    }
}
