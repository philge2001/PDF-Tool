//! Zeichnet Annotationen aus dem Modell als egui-Overlay über der gerenderten Seite.
//!
//! Spiegelt `core::annotation::appearance` (was in die PDF geschrieben wird) so genau
//! wie mit egui möglich. Unterschiede: Hervorhebungen sind halbtransparent statt
//! "Multiply", Freitext nutzt die egui-Schrift (Zeilenumbrüche aber identisch).

use crate::viewer::transform;
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2,
};
use pdftool_core::annotation::appearance::arrow_head;
use pdftool_core::annotation::{NOTE_SIZE, arrow_size};
use pdftool_core::{AnnotKind, Annotation, Color, PageInfo, Point};

pub fn color32(c: Color, alpha: f32) -> Color32 {
    let [r, g, b] = c.to_rgb8();
    Color32::from_rgba_unmultiplied(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Zeichnet eine Annotation. `page_rect` = Bildschirmrechteck der Seite.
pub fn paint(painter: &egui::Painter, info: &PageInfo, page_rect: Rect, a: &Annotation) {
    // Bildschirmpunkte pro PDF-Punkt (für Linienbreiten und Schriftgrößen).
    let scale = page_rect.width() / info.display_size().0;
    let to_screen = |p: Point| transform::pdf_to_screen(info, page_rect, p);
    let rect_to_screen = |r: pdftool_core::Rect| transform::pdf_rect_to_screen(info, page_rect, r);
    let stroke = |w: f32, alpha: f32| Stroke::new((w * scale).max(1.0), color32(a.color, alpha));

    match &a.kind {
        AnnotKind::Highlight { quads } => {
            // egui kann kein "Multiply" (Text bleibt dabei voll lesbar). Annäherung: höchstens
            // halbe Deckkraft, damit der Text unter der Markierung lesbar bleibt.
            let fill = color32(a.color, 0.2 + 0.3 * a.opacity);
            for q in quads {
                painter.rect_filled(rect_to_screen(q.bounds()), 0.0, fill);
            }
        }
        AnnotKind::Redact { quads } => {
            // Vorschau der Schwärzung: rote Kontur, leicht abgedunkelt.
            for q in quads {
                let r = rect_to_screen(q.bounds());
                painter.rect_filled(r, 0.0, Color32::from_black_alpha(60));
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.5, color32(a.color, 1.0)),
                    StrokeKind::Inside,
                );
            }
        }
        AnnotKind::Underline { quads } | AnnotKind::StrikeOut { quads } => {
            let strike = matches!(a.kind, AnnotKind::StrikeOut { .. });
            for q in quads {
                let r = q.bounds();
                let w = (r.height() * 0.07).max(0.5);
                let y = if strike {
                    r.y0 + r.height() * 0.52
                } else {
                    r.y0 + w
                };
                painter.line_segment(
                    [
                        to_screen(Point::new(r.x0, y)),
                        to_screen(Point::new(r.x1, y)),
                    ],
                    stroke(w, a.opacity),
                );
            }
        }
        AnnotKind::Note { pos } => paint_note_icon(painter, to_screen(*pos), NOTE_SIZE * scale, a),
        AnnotKind::Ink { strokes, width } => {
            for s in strokes {
                let pts: Vec<Pos2> = s.iter().map(|p| to_screen(*p)).collect();
                if pts.len() == 1 {
                    painter.circle_filled(
                        pts[0],
                        (width * scale / 2.0).max(1.0),
                        color32(a.color, a.opacity),
                    );
                } else {
                    painter.add(Shape::line(pts, stroke(*width, a.opacity)));
                }
            }
        }
        AnnotKind::Square { rect, width } => {
            painter.rect_stroke(
                rect_to_screen(rect.expand(-width / 2.0)),
                0.0,
                stroke(*width, a.opacity),
                StrokeKind::Middle,
            );
        }
        AnnotKind::Circle { rect, width } => {
            let r = rect_to_screen(rect.expand(-width / 2.0));
            painter.add(Shape::ellipse_stroke(
                r.center(),
                r.size() / 2.0,
                stroke(*width, a.opacity),
            ));
        }
        AnnotKind::Line {
            start,
            end,
            width,
            arrow_end,
        } => {
            let st = stroke(*width, a.opacity);
            painter.line_segment([to_screen(*start), to_screen(*end)], st);
            if *arrow_end {
                let (l, r) = arrow_head(*start, *end, arrow_size(*width));
                painter.add(Shape::line(
                    vec![to_screen(l), to_screen(*end), to_screen(r)],
                    st,
                ));
            }
        }
        AnnotKind::FreeText { rect, font_size } => {
            let sr = rect_to_screen(*rect);
            let pad = 2.0;
            // Gleicher Umbruch wie in der Datei (Breiten der eingebetteten Schrift).
            let lines = pdftool_core::embedded_font::wrap(
                &a.contents,
                *font_size,
                rect.width() - 2.0 * pad,
            );
            let fid = FontId::new(
                (font_size * scale).max(1.0),
                egui::FontFamily::Name(crate::app::NOTO_FAMILY.into()),
            );
            let leading = font_size * 1.2 * scale;
            let painter = painter.with_clip_rect(sr.intersect(painter.clip_rect()));
            for (i, line) in lines.iter().enumerate() {
                // Oberkante der Zeile; egui positioniert Text an seiner Oberkante.
                let y = sr.min.y + pad * scale + i as f32 * leading;
                painter.text(
                    pos2(sr.min.x + pad * scale, y),
                    Align2::LEFT_TOP,
                    line,
                    fid.clone(),
                    color32(a.color, a.opacity),
                );
            }
        }
    }
}

/// Sprechblasen-Symbol einer Notiz; `top_left` in Bildschirmkoordinaten.
fn paint_note_icon(painter: &egui::Painter, top_left: Pos2, size: f32, a: &Annotation) {
    let u = size / NOTE_SIZE; // Bildschirmpunkte pro "Symbol-Punkt"
    let p = |x: f32, y: f32| top_left + vec2(x * u, y * u); // y nach unten
    let body = vec![
        p(1.0, 1.0),
        p(19.0, 1.0),
        p(19.0, 15.0),
        p(9.0, 15.0),
        p(4.0, 19.0),
        p(5.0, 15.0),
        p(1.0, 15.0),
    ];
    let outline = Stroke::new((0.8 * u).max(1.0), Color32::from_gray(64));
    // Nicht konvex (Spitze) -> erst Körper als Rechteck füllen, dann Spitze, dann Umriss.
    painter.rect_filled(
        Rect::from_min_max(p(1.0, 1.0), p(19.0, 15.0)),
        0.0,
        color32(a.color, a.opacity),
    );
    painter.add(Shape::convex_polygon(
        vec![p(9.0, 15.0), p(4.0, 19.0), p(5.0, 15.0)],
        color32(a.color, a.opacity),
        Stroke::NONE,
    ));
    painter.add(Shape::closed_line(body, outline));
    for y in [5.0, 8.0, 11.0] {
        painter.line_segment([p(4.0, y), p(16.0, y)], outline);
    }
}

/// Auswahlrahmen um eine Annotation (für das Auswahlwerkzeug, M7).
pub fn paint_selection_frame(
    painter: &egui::Painter,
    info: &PageInfo,
    page_rect: Rect,
    a: &Annotation,
) {
    let r = transform::pdf_rect_to_screen(info, page_rect, a.bounds()).expand(3.0);
    let blue = Color32::from_rgb(30, 120, 255);
    painter.rect_stroke(r, 2.0, Stroke::new(1.5, blue), StrokeKind::Outside);
    for c in [
        r.left_top(),
        r.right_top(),
        r.left_bottom(),
        r.right_bottom(),
    ] {
        painter.rect_filled(Rect::from_center_size(c, vec2(7.0, 7.0)), 1.0, blue);
    }
}
