//! Umrechnung PDF-Benutzerraum <-> Bildschirm. Die EINZIGE Stelle, an der das passiert.
//!
//! `page_rect` ist das Bildschirmrechteck der Seite (aus `ViewOutput::pages`); Rotation
//! steckt in `PageInfo`, Zoom und Scrollposition stecken in `page_rect`.

use eframe::egui::{Pos2, Rect, pos2};
use foliocraft_core::{PageInfo, Point};

pub fn pdf_to_screen(info: &PageInfo, page_rect: Rect, p: Point) -> Pos2 {
    let u = info.pdf_to_display_unit(p);
    pos2(
        page_rect.min.x + u.x * page_rect.width(),
        page_rect.min.y + u.y * page_rect.height(),
    )
}

pub fn screen_to_pdf(info: &PageInfo, page_rect: Rect, p: Pos2) -> Point {
    let u = Point::new(
        (p.x - page_rect.min.x) / page_rect.width(),
        (p.y - page_rect.min.y) / page_rect.height(),
    );
    info.display_unit_to_pdf(u)
}

/// Achsparalleles PDF-Rechteck -> Bildschirmrechteck (bei 90°-Rotationen bleibt es achsparallel).
pub fn pdf_rect_to_screen(info: &PageInfo, page_rect: Rect, r: foliocraft_core::Rect) -> Rect {
    let a = pdf_to_screen(info, page_rect, Point::new(r.x0, r.y0));
    let b = pdf_to_screen(info, page_rect, Point::new(r.x1, r.y1));
    Rect::from_two_pos(a, b)
}

/// PDF-Rechteck -> normierte Anzeige-Koordinaten (0..1, oben links) – zoomunabhängig.
pub fn pdf_rect_to_unit(info: &PageInfo, r: foliocraft_core::Rect) -> Rect {
    pdf_rect_to_screen(info, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use foliocraft_core::Rect as PdfRect;

    fn page(rotation: u16) -> PageInfo {
        PageInfo {
            id: foliocraft_core::PageId(1),
            object_id: (1, 0),
            crop_box: PdfRect::new(0.0, 0.0, 600.0, 800.0),
            rotation,
        }
    }

    #[test]
    fn roundtrip_screen_pdf() {
        for rot in [0, 90, 180, 270] {
            let info = page(rot);
            let (w, h) = info.display_size();
            let screen =
                Rect::from_min_size(pos2(100.0, 50.0), eframe::egui::vec2(w * 1.5, h * 1.5));
            let p = Point::new(123.0, 456.0);
            let back = screen_to_pdf(&info, screen, pdf_to_screen(&info, screen, p));
            assert!(
                (back.x - p.x).abs() < 0.01 && (back.y - p.y).abs() < 0.01,
                "rot {rot}"
            );
        }
    }

    #[test]
    fn top_of_pdf_is_top_of_screen() {
        let info = page(0);
        let screen = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(600.0, 800.0));
        let r = pdf_rect_to_screen(&info, screen, PdfRect::new(0.0, 790.0, 10.0, 800.0));
        let expected = Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0));
        // Gleitkomma: nur bis auf Rundungsfehler gleich.
        assert!(
            (r.min - expected.min).length() < 1e-3 && (r.max - expected.max).length() < 1e-3,
            "{r:?}"
        );
    }
}
