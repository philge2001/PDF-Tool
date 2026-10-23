//! Formular bearbeiten: Felder aufziehen, auswählen, verschieben, Größe ändern, löschen.
//!
//! Während des Ziehens wird nur eine Vorschau gezeichnet; erst beim Loslassen entsteht
//! ein Command (ein Undo-Schritt pro Zug).

use crate::app::OpenDocument;
use crate::form_draw;
use crate::forms::FieldFocus;
use crate::tools::Tool;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, Align2, Color32, FontId, Rect, Stroke, StrokeKind, vec2};
use pdftool_core::form::ChoiceOption;
use pdftool_core::{
    AddField, FieldKind, FormField, Point, Rect as PdfRect, RemoveField, UpdateField,
};

/// Seitenlänge des Anfassers zum Größe-Ändern (Bildschirmpunkte).
const HANDLE: f32 = 8.0;
/// Kleinste Feldgröße in Punkt.
const MIN_SIZE: f32 = 6.0;

#[derive(Default)]
pub struct FormEdit {
    /// Ausgewähltes Widget.
    pub selected: Option<FieldFocus>,
    drag: Option<EditDrag>,
}

struct EditDrag {
    kind: DragKind,
    page: usize,
    /// Startpunkt im PDF-Raum.
    start: Point,
    /// Aktuelles Rechteck (Vorschau) im PDF-Raum.
    rect: PdfRect,
}

#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Create(Tool),
    Move(FieldFocus),
    Resize(FieldFocus),
}

/// Feldart und Namensvorlage zu einem Erstell-Werkzeug.
fn kind_for(tool: Tool) -> Option<(FieldKind, &'static str, (f32, f32))> {
    Some(match tool {
        Tool::FieldText => (
            FieldKind::Text {
                multiline: false,
                password: false,
                max_len: None,
            },
            "Textfeld",
            (150.0, 20.0),
        ),
        Tool::FieldCheckBox => (FieldKind::CheckBox, "Kontrollkästchen", (14.0, 14.0)),
        Tool::FieldRadio => (FieldKind::Radio, "Optionsfeld", (14.0, 14.0)),
        Tool::FieldCombo => (
            FieldKind::Combo { editable: false },
            "Kombinationsfeld",
            (150.0, 20.0),
        ),
        Tool::FieldList => (
            FieldKind::List { multi: false },
            "Listenfeld",
            (150.0, 60.0),
        ),
        _ => return None,
    })
}

impl OpenDocument {
    fn widget(&self, f: FieldFocus) -> Option<(&FormField, &pdftool_core::Widget)> {
        let field = self.doc.forms().get(f.field)?;
        Some((field, field.widgets.get(f.widget)?))
    }

    /// Anfasser unten rechts (im PDF-Sinn: x1/y0) des ausgewählten Widgets auf dem Bildschirm.
    fn handle_rect(&self, view: &ViewOutput, f: FieldFocus) -> Option<Rect> {
        let (_, w) = self.widget(f)?;
        let page = self.doc.page_index_of(w.page)?;
        let (_, page_rect) = view.pages.iter().find(|(i, _)| *i == page)?;
        let info = self.doc.page(page).ok()?;
        let corner = transform::pdf_to_screen(info, *page_rect, Point::new(w.rect.x1, w.rect.y0));
        Some(Rect::from_center_size(corner, vec2(HANDLE, HANDLE)))
    }

    /// Maus im Modus "Formular bearbeiten". Gibt ein neues Werkzeug zurück, wenn nach dem
    /// Anlegen eines Felds zurück zu "Auswählen" gewechselt werden soll.
    pub fn handle_form_edit(
        &mut self,
        ui: &egui::Ui,
        view: &ViewOutput,
        tool: Tool,
    ) -> Option<Tool> {
        let resp = &view.response;
        let mut new_tool = None;

        // Cursor
        if let Some(pos) = resp.hover_pos() {
            let on_handle = self
                .form_edit
                .selected
                .and_then(|s| self.handle_rect(view, s))
                .is_some_and(|h| h.contains(pos));
            let icon = if tool != Tool::FormEdit {
                egui::CursorIcon::Crosshair
            } else if on_handle {
                egui::CursorIcon::ResizeNwSe
            } else if self.field_at(view, pos).is_some() {
                egui::CursorIcon::Move
            } else {
                egui::CursorIcon::Default
            };
            ui.ctx().set_cursor_icon(icon);
        }

        if resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && let Some((page, page_rect)) = view.page_at(origin)
            && let Ok(info) = self.doc.page(page)
        {
            let start = transform::screen_to_pdf(info, page_rect, origin);
            let kind = if tool != Tool::FormEdit {
                Some(DragKind::Create(tool))
            } else if let Some(sel) = self.form_edit.selected
                && self
                    .handle_rect(view, sel)
                    .is_some_and(|h| h.contains(origin))
            {
                Some(DragKind::Resize(sel))
            } else if let Some(hit) = self.field_at(view, origin) {
                self.form_edit.selected = Some(hit);
                Some(DragKind::Move(hit))
            } else {
                self.form_edit.selected = None;
                None
            };
            let rect = match kind {
                Some(DragKind::Move(f) | DragKind::Resize(f)) => {
                    self.widget(f).map(|(_, w)| w.rect)
                }
                _ => Some(PdfRect::new(start.x, start.y, start.x, start.y)),
            };
            if let (Some(kind), Some(rect)) = (kind, rect) {
                self.form_edit.drag = Some(EditDrag {
                    kind,
                    page,
                    start,
                    rect,
                });
            }
        }

        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(drag) = &self.form_edit.drag
            && let Some((_, page_rect)) = view.pages.iter().find(|(i, _)| *i == drag.page).copied()
            && let Ok(info) = self.doc.page(drag.page)
        {
            let p = transform::screen_to_pdf(info, page_rect, pos);
            let original = match drag.kind {
                DragKind::Move(f) | DragKind::Resize(f) => self.widget(f).map(|(_, w)| w.rect),
                DragKind::Create(_) => None,
            };
            let rect = match (drag.kind, original) {
                (DragKind::Move(_), Some(r)) => r.translate(p.x - drag.start.x, p.y - drag.start.y),
                (DragKind::Resize(_), Some(r)) => PdfRect::new(
                    r.x0,
                    p.y.min(r.y1 - MIN_SIZE),
                    p.x.max(r.x0 + MIN_SIZE),
                    r.y1,
                ),
                _ => PdfRect::from_points(drag.start, p),
            };
            if let Some(drag) = &mut self.form_edit.drag {
                drag.rect = rect;
            }
        }

        if resp.drag_stopped()
            && let Some(drag) = self.form_edit.drag.take()
        {
            match drag.kind {
                DragKind::Create(t) => {
                    new_tool = self.create_field(t, drag.page, Some(drag.rect), drag.start);
                }
                DragKind::Move(f) | DragKind::Resize(f) => self.set_widget_rect(f, drag.rect),
            }
        }

        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            if tool == Tool::FormEdit {
                self.form_edit.selected = self.field_at(view, pos);
            } else if let Some((page, page_rect)) = view.page_at(pos)
                && let Ok(info) = self.doc.page(page)
            {
                let p = transform::screen_to_pdf(info, page_rect, pos);
                new_tool = self.create_field(tool, page, None, p);
            }
        }
        new_tool
    }

    /// Legt ein Feld an: aufgezogen (`rect`) oder per Klick (Standardgröße, linke obere Ecke `at`).
    fn create_field(
        &mut self,
        tool: Tool,
        page: usize,
        rect: Option<PdfRect>,
        at: Point,
    ) -> Option<Tool> {
        let (kind, prefix, (w, h)) = kind_for(tool)?;
        let info = self.doc.page(page).ok()?;
        let rect = rect
            .map(PdfRect::normalized)
            .filter(|r| r.width() >= MIN_SIZE && r.height() >= MIN_SIZE)
            .unwrap_or(PdfRect::new(at.x, at.y - h, at.x + w, at.y));
        let name = self.doc.forms().unique_name(prefix);
        let mut field = FormField::new(kind, name, info.object_id, rect);
        if matches!(field.kind, FieldKind::Combo { .. } | FieldKind::List { .. }) {
            field.options = (1..=3)
                .map(|i| ChoiceOption {
                    export: format!("Option {i}"),
                    display: format!("Option {i}"),
                })
                .collect();
        }
        field.id = self.doc.allocate_field_id();
        let id = field.id;
        self.execute(Box::new(AddField::new(field)));
        self.form_edit.selected = Some(FieldFocus {
            field: id,
            widget: 0,
        });
        Some(Tool::FormEdit)
    }

    fn set_widget_rect(&mut self, f: FieldFocus, rect: PdfRect) {
        let Some((field, w)) = self.widget(f) else {
            return;
        };
        if w.rect == rect {
            return;
        }
        let before = field.clone();
        let mut after = before.clone();
        after.widgets[f.widget].rect = rect;
        self.execute(Box::new(UpdateField::new(before, after)));
    }

    /// Ausgewähltes Feld löschen (Entf). `true`, wenn etwas gelöscht wurde.
    pub fn delete_selected_field(&mut self) -> bool {
        let Some(sel) = self.form_edit.selected.take() else {
            return false;
        };
        self.execute(Box::new(RemoveField::new(sel.field)));
        true
    }

    /// Entfernt eine nicht mehr existierende Auswahl (z. B. nach Rückgängig).
    pub fn validate_field_selection(&mut self) {
        if let Some(sel) = self.form_edit.selected
            && self.widget(sel).is_none()
        {
            self.form_edit.selected = None;
        }
    }

    /// Namen, Auswahlrahmen und Vorschau im Bearbeiten-Modus.
    pub fn paint_form_edit(&self, ui: &egui::Ui, view: &ViewOutput) {
        let painter = ui.painter().with_clip_rect(view.clip);
        let blue = form_draw::FOCUS;
        for &(page, page_rect) in &view.pages {
            let Ok(info) = self.doc.page(page) else {
                continue;
            };
            for (f, w) in self.doc.forms().widgets_on(info.object_id) {
                if !f.kind.is_fillable() {
                    continue;
                }
                let r = form_draw::widget_rect(info, page_rect, w);
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.0, blue.gamma_multiply(0.6)),
                    StrokeKind::Outside,
                );
                // Name des Felds klein in die linke obere Ecke – bei kleinen Feldern daneben.
                let (pos, align) = if r.height() < 16.0 {
                    (r.right_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER)
                } else {
                    (r.left_top() + vec2(2.0, 1.0), Align2::LEFT_TOP)
                };
                painter.text(
                    pos,
                    align,
                    f.short_name(),
                    FontId::proportional(9.0),
                    Color32::from_rgb(20, 60, 160),
                );
            }
        }

        if let Some(sel) = self.form_edit.selected
            && let Some((_, w)) = self.widget(sel)
            && let Some(page) = self.doc.page_index_of(w.page)
            && let Some((_, page_rect)) = view.pages.iter().find(|(i, _)| *i == page)
            && let Ok(info) = self.doc.page(page)
        {
            let r = form_draw::widget_rect(info, *page_rect, w);
            painter.rect_stroke(
                r.expand(2.0),
                0.0,
                Stroke::new(1.5, blue),
                StrokeKind::Outside,
            );
            if let Some(h) = self.handle_rect(view, sel) {
                painter.rect_filled(h, 1.0, blue);
            }
        }

        if let Some(drag) = &self.form_edit.drag
            && let Some((_, page_rect)) = view.pages.iter().find(|(i, _)| *i == drag.page)
            && let Ok(info) = self.doc.page(drag.page)
        {
            let r: Rect = transform::pdf_rect_to_screen(info, *page_rect, drag.rect.normalized());
            painter.rect_filled(r, 0.0, form_draw::HIGHLIGHT);
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, blue), StrokeKind::Middle);
        }
    }
}
