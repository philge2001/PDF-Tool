//! Annotationen auswählen, verschieben, löschen, Farbe ändern, Text bearbeiten.

use super::editor::TextEditor;
use crate::annot_draw;
use crate::app::OpenDocument;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, CursorIcon, Pos2};
use foliocraft_core::{
    AnnotId, AnnotKind, Annotation, Color, Point, RemoveAnnotation, UpdateAnnotation,
};

/// Treffertoleranz in Bildschirmpunkten (wird je nach Zoom in PDF-Punkte umgerechnet).
const HIT_TOLERANCE_PX: f32 = 4.0;

/// Laufendes Verschieben einer Annotation.
pub struct AnnotDrag {
    pub before: Annotation,
    /// Startpunkt im PDF-Raum.
    start: Point,
    /// Aktuelle (verschobene) Fassung – wird statt des Originals gezeichnet.
    pub preview: Annotation,
}

impl OpenDocument {
    /// Oberste Annotation unter dem Mauszeiger.
    pub fn annotation_at(&self, view: &ViewOutput, pos: Pos2) -> Option<AnnotId> {
        let (page, rect) = view.pages.iter().copied().find(|(_, r)| r.contains(pos))?;
        let info = self.doc.page(page).ok()?;
        let p = transform::screen_to_pdf(info, rect, pos);
        let scale = rect.width() / info.display_size().0; // Bildschirmpunkte pro PDF-Punkt
        self.doc
            .annotations()
            .hit_test(page, p, HIT_TOLERANCE_PX / scale)
            .map(|a| a.id)
    }

    fn pdf_point(&self, view: &ViewOutput, page: usize, pos: Pos2) -> Option<Point> {
        let (_, rect) = view.pages.iter().find(|(p, _)| *p == page)?;
        Some(transform::screen_to_pdf(
            self.doc.page(page).ok()?,
            *rect,
            pos,
        ))
    }

    /// Maus-Interaktion des Auswahlwerkzeugs. Gibt `true` zurück, wenn die Eingabe
    /// verbraucht wurde (z. B. weil eine Annotation angeklickt wurde).
    pub fn handle_annotation_select(
        &mut self,
        ui: &egui::Ui,
        view: &ViewOutput,
        allow_move: bool,
    ) -> bool {
        let resp = &view.response;
        let hovered = resp.hover_pos().and_then(|p| self.annotation_at(view, p));
        if hovered.is_some() {
            ui.ctx().set_cursor_icon(if allow_move {
                CursorIcon::Move
            } else {
                CursorIcon::PointingHand
            });
        }

        // Doppelklick auf Notiz/Freitext: Text bearbeiten.
        if resp.double_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(id) = self.annotation_at(view, pos)
            && let Some(a) = self.doc.annotations().get(id)
            && matches!(a.kind, AnnotKind::Note { .. } | AnnotKind::FreeText { .. })
        {
            self.selected = Some(id);
            self.editor = Some(TextEditor::edit(a.clone(), pos));
            return true;
        }

        if resp.clicked() {
            let hit = resp
                .interact_pointer_pos()
                .and_then(|p| self.annotation_at(view, p));
            self.selected = hit;
            return hit.is_some();
        }

        if !allow_move {
            return false;
        }

        // Ziehen: Annotation unter dem Startpunkt verschieben.
        if resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && let Some(id) = self.annotation_at(view, origin)
            && let Some(a) = self.doc.annotations().get(id).cloned()
            && let Some(start) = self.pdf_point(view, a.page, origin)
        {
            self.selected = Some(id);
            self.annot_drag = Some(AnnotDrag {
                preview: a.clone(),
                before: a,
                start,
            });
        }
        if let Some(drag) = &self.annot_drag {
            let page = drag.before.page;
            if resp.dragged()
                && let Some(pos) = resp.interact_pointer_pos()
                && let Some(p) = self.pdf_point(view, page, pos)
                && let Some(drag) = &mut self.annot_drag
            {
                let mut moved = drag.before.clone();
                moved.translate(p.x - drag.start.x, p.y - drag.start.y);
                drag.preview = moved;
            }
            if resp.drag_stopped()
                && let Some(drag) = self.annot_drag.take()
                && drag.preview != drag.before
            {
                let mut after = drag.preview;
                after.modified = foliocraft_core::date::pdf_date_now();
                self.execute(Box::new(UpdateAnnotation::new(drag.before, after)));
            }
            return true;
        }
        false
    }

    pub fn delete_selected(&mut self) {
        if let Some(id) = self.selected.take() {
            self.execute(Box::new(RemoveAnnotation::new(id)));
        }
    }

    /// Ändert eine Eigenschaft der ausgewählten Annotation. Mehrere direkt aufeinander
    /// folgende Änderungen (Farbwähler ziehen, Tippen) ergeben EINEN Undo-Schritt.
    pub fn update_selected(&mut self, change: impl FnOnce(&mut Annotation)) {
        let Some(before) = self
            .selected
            .and_then(|id| self.doc.annotations().get(id))
            .cloned()
        else {
            return;
        };
        let mut after = before.clone();
        change(&mut after);
        if after == before {
            return;
        }
        after.modified = foliocraft_core::date::pdf_date_now();
        let cmd = UpdateAnnotation::new(before, after).mergeable();
        if let Err(e) = self.doc.execute_merging(Box::new(cmd)) {
            tracing::error!(error = %e, "Änderung fehlgeschlagen");
        }
    }

    pub fn set_selected_color(&mut self, color: Color) {
        self.update_selected(|a| a.color = color);
    }

    /// Entfernt eine Auswahl, deren Annotation nicht mehr existiert (z. B. nach Undo).
    pub fn validate_selection(&mut self) {
        if self
            .selected
            .is_some_and(|id| self.doc.annotations().get(id).is_none())
        {
            self.selected = None;
        }
    }

    /// Auswahlrahmen zeichnen.
    pub fn paint_annotation_selection(&self, ui: &egui::Ui, view: &ViewOutput) {
        let Some(id) = self.selected else { return };
        let a = match &self.annot_drag {
            Some(d) if d.preview.id == id => &d.preview,
            _ => match self.doc.annotations().get(id) {
                Some(a) => a,
                None => return,
            },
        };
        let (Some((_, rect)), Ok(info)) = (
            view.pages.iter().find(|(p, _)| *p == a.page),
            self.doc.page(a.page),
        ) else {
            return;
        };
        annot_draw::paint_selection_frame(&ui.painter_at(view.clip), info, *rect, a);
    }
}
