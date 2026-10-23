//! Werkzeug "Bilder" im Modus "Inhalt bearbeiten": auswählen, verschieben, skalieren
//! (Anfasser unten rechts, Seitenverhältnis bleibt), ersetzen, löschen, einfügen.
//!
//! Während des Ziehens wird nur ein Umriss gezeichnet; erst beim Loslassen entsteht ein
//! Command (ein Undo-Schritt), danach rendert der Worker die Seite neu.

use crate::app::OpenDocument;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, vec2};
use pdftool_core::content::Matrix;
use pdftool_core::{NewImage, PageImage, Point};

/// Seitenlänge des Anfassers (Bildschirmpunkte).
const HANDLE: f32 = 9.0;

pub(crate) struct ImageDrag {
    page: usize,
    image: PageImage,
    resize: bool,
    /// Startpunkt (Bildschirm) und Ankerpunkt beim Skalieren (PDF-Raum, gegenüberliegende Ecke).
    start: Pos2,
    anchor: Point,
    /// Bisherige Abbildung (PDF-Raum), wird beim Loslassen angewendet.
    by: Matrix,
}

/// Bildschirmrechteck eines PDF-Rechtecks.
fn screen_rect(
    d: &OpenDocument,
    view: &ViewOutput,
    page: usize,
    r: pdftool_core::Rect,
) -> Option<Rect> {
    let (_, page_rect) = view.pages.iter().find(|(p, _)| *p == page)?;
    let info = d.doc.page(page).ok()?;
    Some(transform::pdf_rect_to_screen(info, *page_rect, r))
}

/// Umfassendes Rechteck des Einheitsquadrats unter einer Matrix.
fn bounds_of(m: &Matrix) -> pdftool_core::Rect {
    m.apply_rect(pdftool_core::Rect::new(0.0, 0.0, 1.0, 1.0))
}

impl OpenDocument {
    /// Bilder einer Seite (aus dem Zwischenspeicher oder neu gelesen).
    pub fn page_images(&mut self, page: usize) -> Vec<PageImage> {
        let Ok(info) = self.doc.page(page) else {
            return Vec::new();
        };
        let (id, stamp) = (info.id, self.doc.content_stamp(info.object_id));
        let cache = &mut self.content_edit.images;
        if cache.get(&id).is_none_or(|(s, _)| *s != stamp) {
            let images = self.doc.page_images(page).unwrap_or_default();
            cache.insert(id, (stamp, images));
        }
        cache.get(&id).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    /// Oberstes Bild unter dem Mauszeiger (zuletzt gezeichnetes zuerst).
    pub(crate) fn image_at(&mut self, view: &ViewOutput, pos: Pos2) -> Option<(usize, PageImage)> {
        let (page, page_rect) = view.page_at(pos)?;
        let info = self.doc.page(page).ok()?.clone();
        let p = transform::screen_to_pdf(&info, page_rect, pos);
        self.page_images(page)
            .into_iter()
            .rev()
            .find(|i| i.bounds.contains(p))
            .map(|i| (page, i))
    }

    pub(crate) fn image_handle_rect(&self, view: &ViewOutput) -> Option<Rect> {
        let (page, img) = self.content_edit.selected_image.as_ref()?;
        let r = screen_rect(self, view, *page, img.bounds)?;
        Some(Rect::from_center_size(
            r.right_bottom(),
            vec2(HANDLE, HANDLE),
        ))
    }

    /// Maus mit dem Werkzeug "Bilder".
    pub fn handle_image_edit(&mut self, ui: &egui::Ui, view: &ViewOutput) {
        let resp = &view.response;
        if let Some(pos) = resp.hover_pos() {
            let icon = if self.content_edit.placing_image.is_some() {
                egui::CursorIcon::Crosshair
            } else if self
                .image_handle_rect(view)
                .is_some_and(|h| h.contains(pos))
            {
                egui::CursorIcon::ResizeNwSe
            } else if self.image_at(view, pos).is_some() {
                egui::CursorIcon::Move
            } else {
                egui::CursorIcon::Default
            };
            ui.ctx().set_cursor_icon(icon);
        }

        // Neues Bild platzieren: Klick = Mitte.
        if self.content_edit.placing_image.is_some() {
            if resp.clicked()
                && let Some(pos) = resp.interact_pointer_pos()
                && let Some((page, page_rect)) = view.page_at(pos)
                && let Ok(info) = self.doc.page(page)
                && let Some(img) = self.content_edit.placing_image.take()
            {
                let center = transform::screen_to_pdf(info, page_rect, pos);
                // Natürliche Größe bei 96 dpi, höchstens halb so breit wie die Seite.
                let (page_w, _) = info.display_size();
                let width = (img.width as f32 * 0.75).min(page_w / 2.0);
                let result = self.doc.insert_image(page, img, center, width);
                self.after_image_change(page, result, Some(center));
            }
            return;
        }

        if resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            let on_handle = self
                .image_handle_rect(view)
                .is_some_and(|h| h.contains(origin));
            let hit = if on_handle {
                self.content_edit.selected_image.clone()
            } else {
                self.image_at(view, origin)
            };
            if let Some((page, image)) = hit
                && let Some((_, page_rect)) = view.pages.iter().find(|(p, _)| *p == page).copied()
                && let Ok(info) = self.doc.page(page)
            {
                // Anker beim Skalieren: die Ecke gegenüber dem Anfasser (oben links in der Anzeige).
                let r = transform::pdf_rect_to_screen(info, page_rect, image.bounds);
                let anchor = transform::screen_to_pdf(info, page_rect, r.left_top());
                self.content_edit.selected_image = Some((page, image.clone()));
                self.content_edit.image_drag = Some(ImageDrag {
                    page,
                    image,
                    resize: on_handle,
                    start: origin,
                    anchor,
                    by: Matrix::IDENTITY,
                });
            }
        }

        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(drag) = &self.content_edit.image_drag
            && let Some((_, page_rect)) = view.pages.iter().find(|(p, _)| *p == drag.page).copied()
            && let Ok(info) = self.doc.page(drag.page)
        {
            let to_pdf = |p| transform::screen_to_pdf(info, page_rect, p);
            let by = if drag.resize {
                // Gleichmäßig skalieren: Verhältnis der Diagonalen (Bildschirm) vom Anker aus.
                let r = transform::pdf_rect_to_screen(info, page_rect, drag.image.bounds);
                let old = r.right_bottom() - r.left_top();
                let new = pos - r.left_top();
                let k = (new.dot(old) / old.length_sq().max(1.0)).max(0.05);
                let a = drag.anchor;
                Matrix::translate(-a.x, -a.y)
                    .then(&Matrix([k, 0.0, 0.0, k, 0.0, 0.0]))
                    .then(&Matrix::translate(a.x, a.y))
            } else {
                let (s, p) = (to_pdf(drag.start), to_pdf(pos));
                Matrix::translate(p.x - s.x, p.y - s.y)
            };
            if let Some(drag) = &mut self.content_edit.image_drag {
                drag.by = by;
            }
        }

        if resp.drag_stopped()
            && let Some(drag) = self.content_edit.image_drag.take()
        {
            if drag.by != Matrix::IDENTITY {
                let target = bounds_of(&drag.image.matrix.then(&drag.by)).center();
                let result = self.doc.transform_image(drag.page, &drag.image, drag.by);
                self.after_image_change(drag.page, result, Some(target));
            }
        } else if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            self.content_edit.selected_image = self.image_at(view, pos);
        }
    }

    /// Nach einer Bildänderung: Fehler melden, das betroffene Bild wieder auswählen
    /// (das mit der Mitte am nächsten an `near`).
    fn after_image_change(
        &mut self,
        page: usize,
        result: pdftool_core::Result<()>,
        near: Option<Point>,
    ) {
        self.content_edit.selected_image = None;
        if let Err(e) = result {
            self.content_edit.error = Some(e.to_string());
            return;
        }
        let Some(near) = near else { return };
        self.content_edit.selected_image = self
            .page_images(page)
            .into_iter()
            .min_by(|a, b| {
                a.bounds
                    .center()
                    .distance(near)
                    .total_cmp(&b.bounds.center().distance(near))
            })
            .map(|i| (page, i));
    }

    /// Ausgewähltes Bild löschen (Entf). `true`, wenn etwas gelöscht wurde.
    pub fn delete_selected_image(&mut self) -> bool {
        let Some((page, img)) = self.content_edit.selected_image.take() else {
            return false;
        };
        let result = self.doc.delete_image(page, &img);
        self.after_image_change(page, result, None);
        true
    }

    /// Ausgewähltes Bild durch eine Datei ersetzen.
    pub fn replace_selected_image(&mut self, bytes: &[u8]) {
        let Some((page, img)) = self.content_edit.selected_image.clone() else {
            return;
        };
        let result =
            NewImage::from_bytes(bytes).and_then(|new| self.doc.replace_image(page, &img, new));
        self.after_image_change(page, result, Some(img.bounds.center()));
    }

    /// Bild aus einer Datei vorbereiten; der nächste Klick auf eine Seite platziert es.
    pub fn start_image_insert(&mut self, bytes: &[u8]) {
        match NewImage::from_bytes(bytes) {
            Ok(img) => {
                self.content_edit.selected_image = None;
                self.content_edit.placing_image = Some(img);
            }
            Err(e) => self.content_edit.error = Some(e.to_string()),
        }
    }

    /// Umrisse der Bilder, Auswahl mit Anfasser, Vorschau beim Ziehen.
    pub fn paint_image_edit(&mut self, ui: &egui::Ui, view: &ViewOutput) {
        let painter = ui.painter().with_clip_rect(view.clip);
        let accent = Color32::from_rgb(40, 120, 230);
        for &(page, _) in &view.pages {
            for img in self.page_images(page) {
                if let Some(r) = screen_rect(self, view, page, img.bounds) {
                    painter.rect_stroke(
                        r,
                        0.0,
                        Stroke::new(0.5, accent.gamma_multiply(0.5)),
                        StrokeKind::Outside,
                    );
                }
            }
        }
        if let Some((page, img)) = &self.content_edit.selected_image
            && let Some(r) = screen_rect(self, view, *page, img.bounds)
        {
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, accent), StrokeKind::Outside);
            if let Some(h) = self.image_handle_rect(view) {
                painter.rect_filled(h, 1.0, accent);
            }
        }
        self.paint_image_drag(&painter, view);
    }

    /// Vorschau beim Ziehen eines Bilds.
    pub(crate) fn paint_image_drag(&self, painter: &egui::Painter, view: &ViewOutput) {
        let accent = Color32::from_rgb(40, 120, 230);
        if let Some(drag) = &self.content_edit.image_drag
            && let Some(r) = screen_rect(
                self,
                view,
                drag.page,
                bounds_of(&drag.image.matrix.then(&drag.by)),
            )
        {
            painter.rect_filled(r, 0.0, accent.gamma_multiply(0.15));
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, accent), StrokeKind::Inside);
        }
    }
}
