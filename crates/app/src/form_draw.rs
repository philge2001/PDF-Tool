//! Zeichnet Formularfelder als egui-Overlay über der gerenderten Seite.
//!
//! Die Anordnung des Texts (Schriftgröße, Zeilen, Ausrichtung) kommt aus
//! `core::form::appearance::layout` – derselben Funktion, die beim Speichern die
//! Darstellung in der Datei erzeugt. Nur die Schrift selbst ist die von egui.

use crate::annot_draw::color32;
use crate::viewer::transform;
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use foliocraft_core::form::appearance::{self, SELECTION_COLOR};
use foliocraft_core::{FieldKind, FormField, PageInfo, Point, Widget};

/// Hintergrund hervorgehobener Felder (wie man es von Formular-Readern kennt).
pub const HIGHLIGHT: Color32 = Color32::from_rgba_unmultiplied_const(150, 170, 230, 70);
/// Rahmen um Pflichtfelder, wenn Felder hervorgehoben werden.
const REQUIRED: Color32 = Color32::from_rgb(220, 40, 40);
/// Rahmen des Felds mit Tastaturfokus.
pub const FOCUS: Color32 = Color32::from_rgb(30, 120, 255);

/// Wie ein Widget gezeichnet wird.
#[derive(Clone, Copy, Default)]
pub struct Style {
    /// Ausfüllbare Felder farbig hinterlegen.
    pub highlight: bool,
    /// Den Text weglassen (das Feld wird gerade in einem Eingabefeld bearbeitet).
    pub skip_text: bool,
    /// Fokusrahmen zeichnen.
    pub focused: bool,
}

/// Bildschirmrechteck eines Widgets.
pub fn widget_rect(info: &PageInfo, page_rect: Rect, w: &Widget) -> Rect {
    transform::pdf_rect_to_screen(info, page_rect, w.rect)
}

/// Bildschirmpunkte pro PDF-Punkt.
pub fn scale(info: &PageInfo, page_rect: Rect) -> f32 {
    page_rect.width() / info.display_size().0
}

pub fn paint(
    painter: &egui::Painter,
    info: &PageInfo,
    page_rect: Rect,
    field: &FormField,
    w: &Widget,
    style: Style,
) {
    let r = widget_rect(info, page_rect, w);
    let scale = scale(info, page_rect);
    let painter = painter.with_clip_rect(r.expand(3.0).intersect(painter.clip_rect()));
    let round = field.kind == FieldKind::Radio;
    let toggle = matches!(field.kind, FieldKind::CheckBox | FieldKind::Radio);

    let fill = |c: Color32| {
        if round {
            painter.add(Shape::ellipse_filled(r.center(), r.size() / 2.0, c));
        } else {
            painter.rect_filled(r, 0.0, c);
        }
    };
    let outline = |stroke: Stroke| {
        if round {
            painter.add(Shape::ellipse_stroke(r.center(), r.size() / 2.0, stroke));
        } else {
            painter.rect_stroke(r, 0.0, stroke, StrokeKind::Inside);
        }
    };

    if let Some(bg) = w.background {
        fill(color32(bg, 1.0));
    }
    if style.highlight && !field.read_only {
        fill(HIGHLIGHT);
    }
    match w.border {
        Some(bc) if w.border_width > 0.0 => {
            outline(Stroke::new(
                (w.border_width * scale).max(1.0),
                color32(bc, 1.0),
            ));
        }
        // Schalter ohne eigenen Rahmen: dünner grauer Rand, sonst sieht man das Kästchen nicht.
        _ if toggle => outline(Stroke::new(
            scale.max(1.0),
            color32(appearance::FALLBACK_BORDER, 1.0),
        )),
        _ => {}
    }
    if style.highlight && field.required {
        outline(Stroke::new(1.0, REQUIRED));
    }

    let ink = color32(field.text_color, 1.0);
    match field.kind {
        FieldKind::CheckBox | FieldKind::Radio => {
            if field.widget_is_on(w) {
                paint_symbol(&painter, r, round, ink);
            }
        }
        FieldKind::Text { .. } | FieldKind::Combo { .. } | FieldKind::List { .. } => {
            if !style.skip_text {
                paint_text(&painter, info, page_rect, field, w, scale, ink);
            }
            if style.highlight && matches!(field.kind, FieldKind::Combo { .. }) {
                // Kleiner Pfeil rechts: zeigt, dass sich eine Liste aufklappen lässt.
                let s = (r.height() * 0.3).clamp(4.0, 8.0);
                let c = pos2(r.right() - s - 2.0, r.center().y);
                painter.add(Shape::convex_polygon(
                    vec![
                        c + vec2(-s / 2.0, -s / 4.0),
                        c + vec2(s / 2.0, -s / 4.0),
                        c + vec2(0.0, s / 3.0),
                    ],
                    Color32::from_gray(80),
                    Stroke::NONE,
                ));
            }
        }
        FieldKind::PushButton | FieldKind::Signature => {}
    }
    if style.focused {
        painter.rect_stroke(
            r.expand(1.5),
            2.0,
            Stroke::new(1.5, FOCUS),
            StrokeKind::Outside,
        );
    }
}

/// Häkchen bzw. Punkt – gleiche Proportionen wie im Appearance Stream.
fn paint_symbol(painter: &egui::Painter, r: Rect, round: bool, ink: Color32) {
    let side = r.width().min(r.height());
    let origin = r.center() - vec2(side / 2.0, side / 2.0);
    if round {
        painter.circle_filled(r.center(), side * 0.25, ink);
    } else {
        // Proportionen im PDF gelten von unten; auf dem Bildschirm wächst y nach unten.
        let p = |fx: f32, fy: f32| origin + vec2(fx * side, (1.0 - fy) * side);
        painter.add(Shape::line(
            vec![p(0.22, 0.52), p(0.42, 0.28), p(0.8, 0.76)],
            Stroke::new((side * 0.12).max(1.0), ink),
        ));
    }
}

fn paint_text(
    painter: &egui::Painter,
    info: &PageInfo,
    page_rect: Rect,
    field: &FormField,
    w: &Widget,
    scale: f32,
    ink: Color32,
) {
    let Some(layout) = appearance::layout(field, w) else {
        return;
    };
    let font = FontId::proportional((layout.font_size * scale).max(1.0));
    // Lokale Widget-Koordinaten -> PDF -> Bildschirm.
    let to_screen = |x: f32, y: f32| {
        transform::pdf_to_screen(info, page_rect, Point::new(w.rect.x0 + x, w.rect.y0 + y))
    };
    let lh = layout.line_height();
    for line in &layout.lines {
        if line.selected {
            let a = to_screen(0.0, line.baseline - layout.font_size * 0.25);
            let b = to_screen(w.rect.width(), line.baseline - layout.font_size * 0.25 + lh);
            painter.rect_filled(Rect::from_two_pos(a, b), 0.0, color32(SELECTION_COLOR, 1.0));
        }
        let galley = painter.layout_no_wrap(line.text.clone(), font.clone(), ink);
        // egui setzt Text an seine Oberkante – wir kennen aber die Grundlinie. Der Abstand
        // Oberkante -> Grundlinie steht in der ersten Glyphe.
        let baseline_offset = galley
            .rows
            .first()
            .and_then(|row| row.glyphs.first())
            .map_or(font.size * 0.8, |g| g.pos.y);
        let at: Pos2 = to_screen(line.x, line.baseline) - vec2(0.0, baseline_offset);
        painter.galley(at, galley, ink);
    }
}
