//! Die Dokumentansicht: scrollbare Seitenliste, Zoom, Render-Aufträge, Texturen.

pub mod layout;
pub mod textures;
pub mod thumbnails;

use eframe::egui::{self, Color32, Pos2, Rect, ScrollArea, Sense, Vec2, pos2, vec2};
use foliocraft_render::{
    DocId, Priority, RenderKey, RenderRequest, RenderService, needs_tiles, page_pixel_size,
    tile_grid, tile_rect,
};
use layout::{GAP, Layout};
use std::collections::HashSet;
use textures::PageTextures;

/// Wie weit über/unter dem sichtbaren Bereich vorausgerendert wird (in Bildschirmhöhen).
const PREFETCH_SCREENS: f32 = 0.5;

/// Maximale Kantenlänge der Vorschau, die bei gekachelten Seiten als Platzhalter dient.
const PREVIEW_MAX_PX: f32 = 1024.0;

pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 16.0;

/// Vorgegebene Zoomstufen für Strg+Plus/Minus und das Auswahlmenü.
pub const ZOOM_STEPS: [f32; 13] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0,
];

/// Wie der Zoom bestimmt wird.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomMode {
    /// Fester Wert (Prozentfeld, Strg+Mausrad, …).
    Custom,
    /// Breiteste Seite füllt die Fensterbreite.
    FitWidth,
    /// Aktuelle Seite passt komplett ins Fenster.
    FitPage,
}

/// Seiten, die in diesem Frame sichtbar sind – für Overlays (Suche, Auswahl, Annotationen).
#[allow(dead_code)] // TODO(M3): wird von Suche/Textauswahl genutzt
pub struct ViewOutput {
    /// Interaktionsfläche des gesamten Inhalts (Klicks, Ziehen).
    pub response: egui::Response,
    /// Sichtbarer Bildschirmbereich des Scrollfelds (zum Zuschneiden von Overlays).
    pub clip: Rect,
    /// (Seitenindex, Bildschirmrechteck) aller gezeichneten Seiten.
    pub pages: Vec<(usize, Rect)>,
}

/// Render-Aufträge, die während eines Frames gesammelt und am Ende verschickt werden.
#[derive(Default)]
pub(crate) struct FrameRequests {
    wanted: HashSet<RenderKey>,
    requests: Vec<RenderRequest>,
}

impl FrameRequests {
    /// Merkt `key` als gebraucht vor und erzeugt einen Auftrag, falls noch nichts da/unterwegs ist.
    pub(crate) fn need(
        &mut self,
        textures: &PageTextures,
        key: RenderKey,
        page_pt: (f32, f32),
        priority: Priority,
    ) {
        self.wanted.insert(key);
        if !textures.has(&key) && !textures.is_pending(&key) {
            self.requests.push(RenderRequest {
                key,
                page_size_pt: page_pt,
                priority,
            });
        }
    }
}

pub struct Viewer {
    pub doc_id: DocId,
    pub layout: Layout,
    /// 1.0 = 100 % (1 PDF-Punkt = 1 egui-Punkt).
    zoom: f32,
    zoom_mode: ZoomMode,
    pub current_page: usize,
    pub(crate) textures: PageTextures,
    pub(crate) frame: FrameRequests,
    /// Vom Code gewünschte Scrollposition (wird im nächsten Frame angewendet).
    scroll_to: Option<Vec2>,
    /// Sichtbarer Ausschnitt im letzten Frame, in Inhaltskoordinaten (min = Scroll-Offset).
    last_viewport: Rect,
    /// Bildschirmrechteck des Scrollfelds im letzten Frame.
    last_screen_rect: Rect,
}

impl Viewer {
    pub fn new(doc_id: DocId, page_sizes: Vec<(f32, f32)>) -> Self {
        Self {
            doc_id,
            layout: Layout::new(page_sizes),
            zoom: 1.0,
            zoom_mode: ZoomMode::FitWidth,
            current_page: 0,
            textures: PageTextures::default(),
            frame: FrameRequests::default(),
            scroll_to: None,
            last_viewport: Rect::NOTHING,
            last_screen_rect: Rect::NOTHING,
        }
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn zoom_mode(&self) -> ZoomMode {
        self.zoom_mode
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

    // ---------- Navigation ----------

    pub fn scroll_to_page(&mut self, page: usize) {
        let page = page.min(self.layout.page_count().saturating_sub(1));
        let x = self.scroll_to.map_or(self.last_viewport.min.x, |v| v.x);
        self.scroll_to = Some(vec2(x, self.layout.page_top(page, self.zoom) - GAP));
        self.current_page = page;
    }

    /// Scrollt so, dass `rect_pt` (Anzeige-Punkte relativ zur Seite, Ursprung oben links)
    /// sichtbar ist – z. B. für Suchtreffer.
    #[allow(dead_code)] // TODO(M3): wird von der Suche genutzt
    pub fn reveal(&mut self, page: usize, rect_in_page: Rect) {
        let content_w = self.content_width();
        let page_rect = self.layout.page_rect(page, self.zoom, content_w);
        let target = rect_in_page.translate(page_rect.min.to_vec2());
        let view = self.last_viewport;
        if view.is_positive() && view.contains_rect(target) {
            return;
        }
        // Treffer ungefähr ins obere Drittel holen.
        let y = target.min.y - view.height().max(100.0) / 3.0;
        let x = if target.min.x < view.min.x || target.max.x > view.max.x {
            target.center().x - view.width() / 2.0
        } else {
            view.min.x
        };
        self.scroll_to = Some(vec2(x.max(0.0), y.max(0.0)));
    }

    /// Scrollt relativ (z. B. Pfeiltasten).
    pub fn scroll_by(&mut self, delta: Vec2) {
        let base = self.scroll_to.unwrap_or(self.last_viewport.min.to_vec2());
        self.scroll_to = Some((base + delta).max(Vec2::ZERO));
    }

    // ---------- Zoom ----------

    fn content_width(&self) -> f32 {
        self.layout
            .content_size(self.zoom, self.last_viewport.width().max(0.0))
            .x
    }

    /// Setzt den Zoom so, dass der Inhaltspunkt unter `anchor` (Bildschirmkoordinate)
    /// an derselben Stelle bleibt. Ohne Anker wird die Fenstermitte festgehalten.
    pub fn set_zoom(&mut self, new_zoom: f32, anchor: Option<Pos2>) {
        let new_zoom = new_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if (new_zoom - self.zoom).abs() < 1e-4 {
            return;
        }
        let screen = self.last_screen_rect;
        if !screen.is_positive() || self.layout.page_count() == 0 {
            self.zoom = new_zoom; // noch nie gezeichnet: nichts festzuhalten
            return;
        }
        let anchor = anchor.unwrap_or(screen.center());
        let in_view = anchor - screen.min; // Abstand des Ankers vom linken oberen Fensterrand
        let viewport = self.scroll_to.map_or(self.last_viewport, |off| {
            Rect::from_min_size(off.to_pos2(), self.last_viewport.size())
        });

        // Inhaltspunkt unter dem Anker, ausgedrückt relativ zur Seite darunter.
        let c = viewport.min + in_view;
        let page = self.layout.page_at(c.y, self.zoom);
        let old_rect = self.layout.page_rect(page, self.zoom, self.content_width());
        let rel = (c - old_rect.min) / old_rect.size();

        self.zoom = new_zoom;
        let new_rect = self.layout.page_rect(page, self.zoom, self.content_width());
        let new_c = new_rect.min + rel * new_rect.size();
        self.scroll_to = Some((new_c - in_view).to_vec2().max(Vec2::ZERO));
    }

    pub fn set_zoom_mode(&mut self, mode: ZoomMode) {
        self.zoom_mode = mode;
        if mode == ZoomMode::FitPage {
            // "Ganze Seite" zeigt die aktuelle Seite von oben.
            let page = self.current_page;
            self.apply_fit_zoom();
            self.scroll_to_page(page);
        }
    }

    /// Zoom per Benutzereingabe: beendet automatische Anpassung.
    pub fn zoom_custom(&mut self, zoom: f32, anchor: Option<Pos2>) {
        self.zoom_mode = ZoomMode::Custom;
        self.set_zoom(zoom, anchor);
    }

    /// Nächste/vorherige Stufe aus [`ZOOM_STEPS`].
    pub fn zoom_step(&mut self, up: bool) {
        let z = self.zoom;
        let next = if up {
            ZOOM_STEPS
                .iter()
                .copied()
                .find(|s| *s > z * 1.01)
                .unwrap_or(MAX_ZOOM)
        } else {
            ZOOM_STEPS
                .iter()
                .rev()
                .copied()
                .find(|s| *s < z * 0.99)
                .unwrap_or(MIN_ZOOM)
        };
        self.zoom_custom(next, None);
    }

    /// Berechnet bei "Seitenbreite"/"Ganze Seite" den Zoom aus der Fenstergröße neu.
    fn apply_fit_zoom(&mut self) {
        let view = self.last_viewport.size();
        if view.x <= 0.0 || self.layout.page_count() == 0 {
            return;
        }
        let fit_w = (view.x - 2.0 * GAP) / self.layout.max_width_pt();
        let target = match self.zoom_mode {
            ZoomMode::Custom => return,
            ZoomMode::FitWidth => fit_w,
            ZoomMode::FitPage => {
                let (w, h) = self.layout.page_size_pt(self.current_page);
                ((view.x - 2.0 * GAP) / w).min((view.y - 2.0 * GAP) / h)
            }
        };
        // Kleine Schwankungen ignorieren (z. B. ein- und ausgeblendete Scrollbalken).
        if (target - self.zoom).abs() / self.zoom > 0.005 {
            let page = self.current_page;
            let keep_page = self.zoom_mode == ZoomMode::FitPage;
            self.set_zoom(target, Some(self.last_screen_rect.left_top()));
            if keep_page {
                self.scroll_to_page(page);
            }
        }
    }

    // ---------- Zeichnen ----------

    pub fn show(&mut self, ui: &mut egui::Ui) -> ViewOutput {
        self.apply_fit_zoom();

        // Strg+Mausrad / Touchpad-Pinch: egui liefert das als Zoomfaktor.
        let (zoom_delta, hover) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
        if zoom_delta != 1.0
            && let Some(pos) = hover.filter(|p| self.last_screen_rect.contains(*p))
        {
            // Gedämpft: Eine Mausrad-Raste liefert sonst ca. Faktor 1,6 – zu grob zum Lesen.
            self.zoom_custom(self.zoom * zoom_delta.powf(0.4), Some(pos));
        }

        let ppp = ui.ctx().pixels_per_point();
        let mut area = ScrollArea::both().auto_shrink(false);
        if let Some(off) = self.scroll_to.take() {
            area = area.scroll_offset(off);
        }

        let mut pages = Vec::new();
        let mut response = None;
        // `show_viewport` gibt uns den sichtbaren Ausschnitt (in Inhaltskoordinaten),
        // sodass wir nur sichtbare Seiten zeichnen müssen.
        let out = area.show_viewport(ui, |ui, viewport| {
            self.last_viewport = viewport;
            let content = self.layout.content_size(self.zoom, viewport.width());
            let (resp, painter) = ui.allocate_painter(content, Sense::click_and_drag());
            let origin = resp.rect.min.to_vec2();
            let visible_screen = viewport.translate(origin);

            let margin = viewport.height() * PREFETCH_SCREENS;
            let range = self.layout.visible_range(
                viewport.min.y - margin,
                viewport.max.y + margin,
                self.zoom,
            );
            // "Aktuelle Seite" = die Seite in der oberen Fensterhälfte.
            self.current_page = self
                .layout
                .page_at(viewport.min.y + viewport.height() * 0.3, self.zoom);

            let scale_milli = render_scale_milli(self.zoom * ppp);
            for i in range {
                let rect = self
                    .layout
                    .page_rect(i, self.zoom, content.x)
                    .translate(origin);
                let on_screen = rect.intersects(visible_screen);
                self.paint_page(&painter, i, rect, scale_milli, on_screen, visible_screen);
                if on_screen {
                    pages.push((i, rect));
                }
            }
            response = Some(resp);
        });
        self.last_screen_rect = out.inner_rect;

        ViewOutput {
            // `show_viewport` ruft das Closure immer genau einmal auf – `response` ist also gesetzt.
            response: response.expect("Viewport-Closure wurde ausgeführt"),
            clip: out.inner_rect,
            pages,
        }
    }

    fn paint_page(
        &mut self,
        painter: &egui::Painter,
        i: usize,
        rect: Rect,
        scale_milli: u32,
        on_screen: bool,
        visible_screen: Rect,
    ) {
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
        let key_for = |scale_milli: u32, tile| RenderKey {
            doc: self.doc_id,
            page: i as u32,
            scale_milli,
            tile,
        };

        if !needs_tiles(page_px) {
            let key = key_for(scale_milli, None);
            self.frame.need(&self.textures, key, page_pt, priority);
            if let Some(tex) = self.textures.get(&key) {
                paint_texture(painter, tex.id(), rect);
            } else if let Some(tex) = self.textures.best_fallback(self.doc_id, i as u32) {
                paint_texture(painter, tex.id(), rect);
            }
            return;
        }

        // Gekachelt: zuerst eine Vorschau als Platzhalter, darüber die scharfen Kacheln.
        let preview_scale = PREVIEW_MAX_PX / page_pt.0.max(page_pt.1);
        let preview = key_for(render_scale_milli(preview_scale), None);
        self.frame.need(&self.textures, preview, page_pt, priority);
        if let Some(tex) = self.textures.best_fallback(self.doc_id, i as u32) {
            paint_texture(painter, tex.id(), rect);
        }
        if !on_screen {
            return;
        }
        let pt_per_px = rect.width() / page_px.0 as f32;
        let (cols, rows) = tile_grid(page_px);
        for row in 0..rows {
            for col in 0..cols {
                let (x, y, w, h) = tile_rect(page_px, (col, row));
                let tile_screen = Rect::from_min_size(
                    rect.min + vec2(x as f32, y as f32) * pt_per_px,
                    vec2(w as f32, h as f32) * pt_per_px,
                );
                if !tile_screen.intersects(visible_screen) {
                    continue;
                }
                let key = key_for(scale_milli, Some((col, row)));
                self.frame
                    .need(&self.textures, key, page_pt, Priority::Visible);
                if let Some(tex) = self.textures.get(&key) {
                    paint_texture(painter, tex.id(), tile_screen);
                }
            }
        }
    }

    /// Schickt die in diesem Frame gesammelten Aufträge an den Worker. Einmal pro Frame
    /// aufrufen – NACH Hauptansicht und Thumbnails.
    pub fn flush_requests(&mut self, render: &RenderService) {
        // `std::mem::take` nimmt den Inhalt heraus und lässt einen leeren Default zurück.
        let frame = std::mem::take(&mut self.frame);
        // Erst veraltete Aufträge verwerfen, dann neue schicken.
        if let Some(w) = self.textures.update_wanted(frame.wanted) {
            render.retain(w);
        }
        for req in frame.requests {
            self.textures.mark_pending(req.key);
            render.request(req);
        }
    }
}

pub(crate) fn paint_texture(painter: &egui::Painter, id: egui::TextureId, rect: Rect) {
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

    fn viewer_with_frame(n: usize) -> Viewer {
        let mut v = Viewer::new(1, vec![(600.0, 800.0); n]);
        v.zoom_mode = ZoomMode::Custom;
        // So tun, als wäre schon ein Frame gezeichnet worden (Fenster 1000x700).
        v.last_screen_rect = Rect::from_min_size(pos2(0.0, 50.0), vec2(1000.0, 700.0));
        v.last_viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0));
        v
    }

    #[test]
    fn zoom_keeps_anchor_point() {
        let mut v = viewer_with_frame(10);
        v.last_viewport = v.last_viewport.translate(vec2(0.0, 2000.0)); // auf Seite 2 gescrollt
        let anchor = pos2(500.0, 400.0);
        // Inhaltspunkt unter dem Anker vor dem Zoom (relativ zu seiner Seite):
        let c = v.last_viewport.min + (anchor - v.last_screen_rect.min);
        let page = v.layout.page_at(c.y, 1.0);
        let r = v.layout.page_rect(page, 1.0, 1000.0);
        let rel = (c - r.min) / r.size();

        v.set_zoom(2.0, Some(anchor));
        let off = v.scroll_to.unwrap();
        let c2 = off.to_pos2() + (anchor - v.last_screen_rect.min);
        let r2 = v.layout.page_rect(page, 2.0, v.content_width());
        let rel2 = (c2 - r2.min) / r2.size();
        assert!((rel - rel2).length() < 1e-3, "{rel:?} vs {rel2:?}");
    }

    #[test]
    fn zoom_steps_move_through_presets() {
        let mut v = viewer_with_frame(1);
        v.zoom_step(true);
        assert_eq!(v.zoom(), 1.25);
        v.zoom_step(false);
        v.zoom_step(false);
        assert_eq!(v.zoom(), 0.75);
        assert_eq!(v.zoom_mode(), ZoomMode::Custom);
    }

    #[test]
    fn fit_width_computes_zoom() {
        let mut v = viewer_with_frame(1);
        v.zoom_mode = ZoomMode::FitWidth;
        v.apply_fit_zoom();
        assert!((v.zoom() - (1000.0 - 2.0 * GAP) / 600.0).abs() < 1e-4);
    }
}
