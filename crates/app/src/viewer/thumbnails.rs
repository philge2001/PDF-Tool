//! Seitenleiste mit Miniaturansichten: Navigation, Mehrfachauswahl, Kontextmenü und
//! Neuanordnen per Drag & Drop. Nutzt denselben Texturcache wie die Hauptansicht,
//! aber mit niedrigster Render-Priorität.

use super::layout::Layout;
use super::{Viewer, paint_texture, render_scale_milli};
use crate::pages::PageAction;
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Rect, ScrollArea, Sense, Stroke, StrokeKind, pos2,
    vec2,
};
use pdftool_core::PageId;
use pdftool_render::{Priority, RenderKey};
use std::collections::BTreeSet;

/// Breite der breitesten Miniatur in egui-Punkten.
const THUMB_WIDTH: f32 = 120.0;
/// Platz für die Seitennummer unter jeder Miniatur.
const THUMB_GAP: f32 = 24.0;

/// Was der Nutzer in der Seitenleiste ausgelöst hat.
pub enum ThumbAction {
    /// Zu dieser Seite springen.
    Navigate(usize),
    Page(PageAction),
}

/// Zustand der Seitenleiste (eigenes Layout mit größerem Abstand).
pub struct Thumbnails {
    layout: Layout,
    /// Für welche Seite wir zuletzt automatisch mitgescrollt haben.
    followed_page: Option<usize>,
    /// Ausgewählte Seiten – als PageId, damit die Auswahl Umsortieren übersteht.
    pub selected: BTreeSet<PageId>,
    /// Ausgangspunkt für Umschalt+Klick.
    anchor: Option<PageId>,
    /// Laufendes Ziehen: gezogene Seiten (Indizes) und Einfügeposition.
    drag: Option<(Vec<usize>, usize)>,
}

impl Thumbnails {
    pub fn new(page_sizes: &[(f32, f32)]) -> Self {
        Self {
            layout: Layout::with_gap(page_sizes.to_vec(), THUMB_GAP),
            followed_page: None,
            selected: BTreeSet::new(),
            anchor: None,
            drag: None,
        }
    }

    /// Neue Seitengrößen nach einer Strukturänderung; die Auswahl bleibt (per PageId) erhalten.
    pub fn restructure(&mut self, page_sizes: &[(f32, f32)]) {
        self.layout = Layout::with_gap(page_sizes.to_vec(), THUMB_GAP);
        self.followed_page = None;
        self.drag = None;
    }

    /// Indizes der ausgewählten Seiten (aufsteigend). Gelöschte Seiten fallen dabei weg.
    pub fn selected_indices(&self, page_ids: &[PageId]) -> Vec<usize> {
        page_ids
            .iter()
            .enumerate()
            .filter(|(_, id)| self.selected.contains(id))
            .map(|(i, _)| i)
            .collect()
    }

    fn select_only(&mut self, id: PageId) {
        self.selected.clear();
        self.selected.insert(id);
        self.anchor = Some(id);
    }

    /// Klick mit Strg (umschalten), Umschalt (Bereich) oder ohne (nur diese Seite).
    fn click(&mut self, i: usize, page_ids: &[PageId], modifiers: egui::Modifiers) {
        let id = page_ids[i];
        if modifiers.command {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
            self.anchor = Some(id);
        } else if modifiers.shift
            && let Some(a) = self
                .anchor
                .and_then(|a| page_ids.iter().position(|&p| p == a))
        {
            let (lo, hi) = (a.min(i), a.max(i));
            self.selected = page_ids[lo..=hi].iter().copied().collect();
        } else {
            self.select_only(id);
        }
    }

    /// Seite unter einer Inhaltskoordinate (nur, wenn wirklich auf einer Miniatur).
    fn page_at(&self, y: f32, x: f32, zoom: f32, content_w: f32) -> Option<usize> {
        let i = self.layout.page_at(y, zoom);
        let r = self.layout.page_rect(i, zoom, content_w);
        let hit = r.expand2(vec2(8.0, 0.0));
        (y >= hit.min.y && y <= hit.max.y + THUMB_GAP && x >= hit.min.x && x <= hit.max.x)
            .then_some(i)
    }

    /// Einfügeposition beim Ziehen: vor der ersten Seite, deren Mitte unter `y` liegt.
    fn insertion_index(&self, y: f32, zoom: f32, content_w: f32) -> usize {
        (0..self.layout.page_count())
            .find(|&i| self.layout.page_rect(i, zoom, content_w).center().y > y)
            .unwrap_or(self.layout.page_count())
    }
}

impl Viewer {
    /// Zeichnet die Miniaturen und verarbeitet Klicks, Kontextmenü und Ziehen.
    /// `overlay` wird pro Miniatur aufgerufen (Painter, Seite, Rechteck) – z. B. für Annotationen.
    pub fn show_thumbnails(
        &mut self,
        ui: &mut egui::Ui,
        thumbs: &mut Thumbnails,
        page_ids: &[PageId],
        overlay: impl Fn(&egui::Painter, usize, Rect),
    ) -> Option<ThumbAction> {
        let zoom = THUMB_WIDTH / thumbs.layout.max_width_pt().max(1.0);
        let ppp = ui.ctx().pixels_per_point();
        let scale_milli = render_scale_milli(zoom * ppp);
        let n = thumbs.layout.page_count();
        let mut action = None;

        let area = ScrollArea::vertical()
            .id_salt("thumbnails")
            .auto_shrink(false);
        // Hauptansicht hat die Seite gewechselt -> Miniatur in Sicht bringen (nicht beim Ziehen).
        let follow = thumbs.followed_page != Some(self.current_page) && thumbs.drag.is_none();
        if follow {
            thumbs.followed_page = Some(self.current_page);
        }

        area.show_viewport(ui, |ui, viewport| {
            let content = thumbs.layout.content_size(zoom, viewport.width());
            let (resp, painter) = ui.allocate_painter(content, Sense::click_and_drag());
            let origin = resp.rect.min.to_vec2();
            let selection_fill = ui.visuals().selection.bg_fill.gamma_multiply(0.45);

            for i in
                thumbs
                    .layout
                    .visible_range(viewport.min.y - 200.0, viewport.max.y + 200.0, zoom)
            {
                let rect = thumbs
                    .layout
                    .page_rect(i, zoom, content.x)
                    .translate(origin);
                if page_ids
                    .get(i)
                    .is_some_and(|id| thumbs.selected.contains(id))
                {
                    let bg = Rect::from_min_max(
                        rect.min - vec2(6.0, 6.0),
                        rect.max + vec2(6.0, THUMB_GAP - 2.0),
                    );
                    painter.rect_filled(bg, 4.0, selection_fill);
                }
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

                overlay(&painter, i, rect);

                let stroke = if i == self.current_page {
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

            // Umrechnung Bildschirm -> Inhaltskoordinaten der Miniaturen.
            let to_content = |p: egui::Pos2| p - origin;
            // `t` als Parameter statt `thumbs` einzufangen: Sonst bliebe `thumbs` ausgeliehen,
            // solange das Closure lebt, und wir könnten die Auswahl unten nicht ändern.
            let page_under = |t: &Thumbnails, p: egui::Pos2| {
                let c = to_content(p);
                t.page_at(c.y, c.x, zoom, content.x)
            };

            // ----- Klick: auswählen und hinspringen -----
            if resp.clicked()
                && let Some(i) = resp
                    .interact_pointer_pos()
                    .and_then(|p| page_under(thumbs, p))
            {
                let modifiers = ui.input(|inp| inp.modifiers);
                thumbs.click(i, page_ids, modifiers);
                action = Some(ThumbAction::Navigate(i));
            }

            // ----- Rechtsklick: Seite mit in die Auswahl nehmen, Kontextmenü -----
            if resp.secondary_clicked()
                && let Some(i) = resp
                    .interact_pointer_pos()
                    .and_then(|p| page_under(thumbs, p))
                && !thumbs.selected.contains(&page_ids[i])
            {
                thumbs.select_only(page_ids[i]);
            }
            resp.context_menu(|ui| {
                let sel = thumbs.selected_indices(page_ids);
                if sel.is_empty() {
                    ui.label("Keine Seite ausgewählt");
                    return;
                }
                let what = if sel.len() == 1 {
                    "Seite".to_string()
                } else {
                    format!("{} Seiten", sel.len())
                };
                ui.label(egui::RichText::new(what).weak());
                if ui.button("Im Uhrzeigersinn drehen").clicked() {
                    action = Some(ThumbAction::Page(PageAction::Rotate(sel.clone(), 1)));
                    ui.close();
                }
                if ui.button("Gegen den Uhrzeigersinn drehen").clicked() {
                    action = Some(ThumbAction::Page(PageAction::Rotate(sel.clone(), -1)));
                    ui.close();
                }
                ui.separator();
                let can_delete = sel.len() < n;
                let delete = ui.add_enabled(can_delete, egui::Button::new("Löschen"));
                if delete
                    .on_disabled_hover_text("Mindestens eine Seite muss bleiben")
                    .clicked()
                {
                    action = Some(ThumbAction::Page(PageAction::Delete(sel)));
                    ui.close();
                }
            });

            // ----- Ziehen: Seiten neu anordnen -----
            if resp.drag_started()
                && let Some(i) = ui
                    .input(|inp| inp.pointer.press_origin())
                    .and_then(|p| page_under(thumbs, p))
            {
                if !thumbs.selected.contains(&page_ids[i]) {
                    thumbs.select_only(page_ids[i]);
                }
                thumbs.drag = Some((thumbs.selected_indices(page_ids), i));
            }
            if let Some((dragged, _)) = thumbs.drag.take() {
                let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
                if let Some(p) = pointer {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    let before = thumbs.insertion_index(to_content(p).y, zoom, content.x);
                    // Einfügelinie in der Lücke vor `before` (bzw. nach der letzten Seite).
                    let gap_y = if before < n {
                        thumbs.layout.page_rect(before, zoom, content.x).min.y - THUMB_GAP / 2.0
                            + 4.0
                    } else {
                        thumbs.layout.page_rect(n - 1, zoom, content.x).max.y + THUMB_GAP
                    };
                    let y = gap_y + origin.y;
                    painter.line_segment(
                        [
                            pos2(resp.rect.min.x + 6.0, y),
                            pos2(resp.rect.max.x - 6.0, y),
                        ],
                        Stroke::new(3.0, ui.visuals().selection.stroke.color),
                    );
                    // Am Rand automatisch scrollen.
                    let edge = 40.0;
                    let clip = ui.clip_rect();
                    if p.y < clip.min.y + edge {
                        ui.scroll_with_delta(vec2(0.0, 8.0));
                    } else if p.y > clip.max.y - edge {
                        ui.scroll_with_delta(vec2(0.0, -8.0));
                    }
                    if resp.drag_stopped() {
                        action = Some(ThumbAction::Page(PageAction::Move(dragged, before)));
                    } else {
                        thumbs.drag = Some((dragged, before));
                    }
                }
            }

            if follow && n > 0 {
                let r = thumbs
                    .layout
                    .page_rect(self.current_page.min(n - 1), zoom, content.x)
                    .translate(origin);
                let r = Rect::from_min_max(r.min, r.max + vec2(0.0, THUMB_GAP));
                ui.scroll_to_rect(r, None);
            }
        });
        action
    }
}
