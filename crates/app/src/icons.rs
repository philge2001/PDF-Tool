//! Eigene Vektor-Icons, mit dem egui-Painter gezeichnet (keine Bilddateien, keine fremden
//! Icon-Sätze). Jedes Icon ist auf einem 24×24-Raster entworfen und wird auf das Zielrechteck
//! skaliert; dadurch ist es in jeder Größe scharf und passt sich der Textfarbe an (hell/dunkel).

use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Menu,
    Home,
    Close,
    Search,
    Save,
    Open,
    Undo,
    Redo,
    ChevronUp,
    ChevronDown,
    ZoomIn,
    ZoomOut,
    FitPage,
    FitWidth,
    Comment,
    Note,
    Thumbnails,
    Signature,
    Accessibility,
    Cursor,
    TextCursor,
    Hand,
    Highlight,
    Underline,
    Strike,
    Draw,
    AddText,
    Rectangle,
    Ellipse,
    Arrow,
    Tools,
    Edit,
    Export,
    Combine,
    Organize,
    Scan,
    Lock,
    Redact,
    Compress,
    Form,
    Certificate,
    Compare,
    Archive,
    More,
    Help,
    Settings,
    Back,
    Plus,
    Trash,
    Image,
    Sanitize,
    Insert,
    Split,
    RotateRight,
    Sun,
    Moon,
}

/// Zeichnet `icon` in `rect` mit Linienfarbe `color`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    // Umrechnung Raster (0..24) -> Bildschirm. `move` kopiert rect in die Closure.
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - Vec2::splat(12.0 * s);
    let p = move |x: f32, y: f32| Pos2::new(o.x + x * s, o.y + y * s);
    let w = (1.6 * s).max(1.0);
    let st = Stroke::new(w, color);
    let line = |pts: &[(f32, f32)]| {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| p(x, y)).collect();
        painter.add(egui::Shape::line(v, st));
    };
    let closed = |pts: &[(f32, f32)]| {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| p(x, y)).collect();
        painter.add(egui::Shape::closed_line(v, st));
    };
    let filled = |pts: &[(f32, f32)], fill: Color32| {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| p(x, y)).collect();
        painter.add(egui::Shape::convex_polygon(v, fill, Stroke::NONE));
    };
    let rect_r = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        painter.rect_stroke(
            Rect::from_min_max(p(x0, y0), p(x1, y1)),
            r * s,
            st,
            StrokeKind::Middle,
        );
    };
    let circle = |x: f32, y: f32, r: f32| painter.circle_stroke(p(x, y), r * s, st);
    let dot = |x: f32, y: f32, r: f32| painter.circle_filled(p(x, y), r * s, color);
    // Dokumentseite mit Eselsohr (für mehrere Werkzeug-Icons).
    let page = |x0: f32, y0: f32, x1: f32, y1: f32| {
        closed(&[(x0, y0), (x1 - 4.0, y0), (x1, y0 + 4.0), (x1, y1), (x0, y1)]);
        line(&[(x1 - 4.0, y0), (x1 - 4.0, y0 + 4.0), (x1, y0 + 4.0)]);
    };

    match icon {
        Icon::Menu => {
            line(&[(4.0, 7.0), (20.0, 7.0)]);
            line(&[(4.0, 12.0), (20.0, 12.0)]);
            line(&[(4.0, 17.0), (20.0, 17.0)]);
        }
        Icon::Home => {
            line(&[(3.5, 11.5), (12.0, 4.0), (20.5, 11.5)]);
            line(&[(6.0, 10.0), (6.0, 20.0), (18.0, 20.0), (18.0, 10.0)]);
            line(&[(10.0, 20.0), (10.0, 14.5), (14.0, 14.5), (14.0, 20.0)]);
        }
        Icon::Close => {
            line(&[(6.0, 6.0), (18.0, 18.0)]);
            line(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Icon::Search => {
            circle(10.5, 10.5, 6.0);
            line(&[(15.0, 15.0), (20.0, 20.0)]);
        }
        Icon::Save => {
            closed(&[
                (4.0, 4.0),
                (17.0, 4.0),
                (20.0, 7.0),
                (20.0, 20.0),
                (4.0, 20.0),
            ]);
            rect_r(8.0, 4.0, 15.0, 9.0, 0.5);
            rect_r(7.0, 13.0, 17.0, 20.0, 0.5);
        }
        Icon::Open => {
            line(&[
                (3.5, 18.5),
                (3.5, 5.5),
                (9.0, 5.5),
                (11.0, 7.5),
                (18.5, 7.5),
                (18.5, 10.0),
            ]);
            closed(&[(3.5, 18.5), (6.5, 10.5), (21.0, 10.5), (18.0, 18.5)]);
        }
        Icon::Undo | Icon::Redo => {
            let m = |x: f32| if icon == Icon::Undo { x } else { 24.0 - x };
            line(&[(m(5.0), 9.0), (m(14.0), 9.0)]);
            // Bogen nach unten zurück
            let arc: Vec<(f32, f32)> = (0..=10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 10.0;
                    (m(14.0 + 5.0 * a.cos()), 14.0 + 5.0 * a.sin())
                })
                .collect();
            line(&arc);
            line(&[(m(14.0), 19.0), (m(8.0), 19.0)]);
            line(&[(m(8.5), 5.5), (m(5.0), 9.0), (m(8.5), 12.5)]);
        }
        Icon::ChevronUp => line(&[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)]),
        Icon::ChevronDown => line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Icon::Back => {
            line(&[(19.0, 12.0), (5.0, 12.0)]);
            line(&[(11.0, 6.0), (5.0, 12.0), (11.0, 18.0)]);
        }
        Icon::ZoomIn | Icon::Plus => {
            if icon == Icon::ZoomIn {
                circle(12.0, 12.0, 8.5);
            }
            line(&[(12.0, 7.0), (12.0, 17.0)]);
            line(&[(7.0, 12.0), (17.0, 12.0)]);
        }
        Icon::ZoomOut => {
            circle(12.0, 12.0, 8.5);
            line(&[(7.0, 12.0), (17.0, 12.0)]);
        }
        Icon::FitPage => {
            rect_r(7.0, 4.0, 17.0, 20.0, 1.0);
            line(&[(3.0, 8.0), (3.0, 3.0), (5.0, 3.0)]);
            line(&[(21.0, 16.0), (21.0, 21.0), (19.0, 21.0)]);
        }
        Icon::FitWidth => {
            rect_r(4.0, 6.0, 20.0, 18.0, 1.0);
            line(&[(7.0, 12.0), (17.0, 12.0)]);
            line(&[(9.0, 10.0), (7.0, 12.0), (9.0, 14.0)]);
            line(&[(15.0, 10.0), (17.0, 12.0), (15.0, 14.0)]);
        }
        Icon::Comment | Icon::Note => {
            closed(&[
                (4.0, 5.0),
                (20.0, 5.0),
                (20.0, 16.0),
                (11.0, 16.0),
                (6.5, 20.0),
                (7.0, 16.0),
                (4.0, 16.0),
            ]);
            if icon == Icon::Comment {
                line(&[(8.0, 9.0), (16.0, 9.0)]);
                line(&[(8.0, 12.5), (13.0, 12.5)]);
            }
        }
        Icon::Thumbnails => {
            rect_r(4.0, 3.5, 11.0, 11.0, 1.0);
            rect_r(13.0, 3.5, 20.0, 11.0, 1.0);
            rect_r(4.0, 13.0, 11.0, 20.5, 1.0);
            rect_r(13.0, 13.0, 20.0, 20.5, 1.0);
        }
        Icon::Signature => {
            let wave: Vec<(f32, f32)> = (0..=16)
                .map(|i| {
                    let x = 3.0 + i as f32;
                    (x, 12.0 + 3.0 * (x * 0.9).sin() - (i as f32) * 0.15)
                })
                .collect();
            line(&wave);
            line(&[(3.0, 19.5), (21.0, 19.5)]);
        }
        Icon::Accessibility => {
            circle(12.0, 12.0, 9.0);
            dot(12.0, 7.3, 1.4);
            line(&[(7.5, 10.0), (12.0, 11.0), (16.5, 10.0)]);
            line(&[(12.0, 11.0), (12.0, 14.0), (9.5, 18.0)]);
            line(&[(12.0, 14.0), (14.5, 18.0)]);
        }
        Icon::Cursor => {
            closed(&[
                (6.0, 3.5),
                (6.0, 18.5),
                (9.5, 15.0),
                (12.0, 20.5),
                (14.5, 19.5),
                (12.0, 14.0),
                (17.0, 14.0),
            ]);
        }
        Icon::TextCursor => {
            line(&[(9.0, 4.0), (12.0, 5.5), (15.0, 4.0)]);
            line(&[(12.0, 5.5), (12.0, 18.5)]);
            line(&[(9.0, 20.0), (12.0, 18.5), (15.0, 20.0)]);
        }
        Icon::Hand => {
            line(&[(8.0, 13.0), (8.0, 6.0)]);
            line(&[(11.0, 12.0), (11.0, 4.5)]);
            line(&[(14.0, 12.0), (14.0, 5.5)]);
            line(&[(17.0, 13.0), (17.0, 8.0)]);
            line(&[
                (8.0, 13.0),
                (5.5, 11.0),
                (4.5, 12.5),
                (8.5, 18.5),
                (11.0, 20.5),
                (15.5, 20.5),
                (17.0, 17.5),
                (17.0, 13.0),
            ]);
        }
        Icon::Highlight => {
            // Textmarker (schräg) mit Kappe und Spitze, darunter ein markierter Streifen.
            closed(&[(15.0, 2.5), (20.5, 8.0), (13.0, 15.5), (7.5, 10.0)]);
            line(&[(12.5, 5.0), (18.0, 10.5)]);
            closed(&[(7.5, 10.0), (13.0, 15.5), (9.5, 17.0), (6.0, 13.5)]);
            line(&[(6.0, 13.5), (4.0, 17.5), (9.5, 17.0)]);
            filled(
                &[(3.0, 19.5), (21.0, 19.5), (21.0, 22.0), (3.0, 22.0)],
                color.gamma_multiply(0.6),
            );
        }
        Icon::Underline | Icon::Strike => {
            // "T"-artiges Zeichen mit Linie darunter bzw. hindurch.
            line(&[(7.0, 4.0), (7.0, 12.0)]);
            line(&[
                (7.0, 12.0),
                (8.5, 15.0),
                (12.0, 16.0),
                (15.5, 15.0),
                (17.0, 12.0),
            ]);
            line(&[(17.0, 4.0), (17.0, 12.0)]);
            if icon == Icon::Underline {
                line(&[(5.0, 20.0), (19.0, 20.0)]);
            } else {
                line(&[(4.0, 10.0), (20.0, 10.0)]);
            }
        }
        Icon::Draw | Icon::Edit => {
            closed(&[
                (15.5, 4.5),
                (19.5, 8.5),
                (9.0, 19.0),
                (4.5, 19.5),
                (5.0, 15.0),
            ]);
            line(&[(13.5, 6.5), (17.5, 10.5)]);
            if icon == Icon::Draw {
                line(&[(12.0, 21.0), (20.0, 21.0)]);
            }
        }
        Icon::AddText => {
            rect_r(3.5, 3.5, 20.5, 20.5, 2.0);
            line(&[(8.0, 8.0), (16.0, 8.0)]);
            line(&[(12.0, 8.0), (12.0, 17.0)]);
        }
        Icon::Rectangle => rect_r(4.0, 6.0, 20.0, 18.0, 1.0),
        Icon::Ellipse => {
            let pts: Vec<(f32, f32)> = (0..32)
                .map(|i| {
                    let a = std::f32::consts::TAU * i as f32 / 32.0;
                    (12.0 + 8.5 * a.cos(), 12.0 + 6.0 * a.sin())
                })
                .collect();
            closed(&pts);
        }
        Icon::Arrow => {
            line(&[(5.0, 19.0), (19.0, 5.0)]);
            line(&[(11.0, 5.0), (19.0, 5.0), (19.0, 13.0)]);
        }
        Icon::Tools => {
            rect_r(4.0, 4.0, 10.5, 10.5, 1.5);
            rect_r(13.5, 4.0, 20.0, 10.5, 1.5);
            rect_r(4.0, 13.5, 10.5, 20.0, 1.5);
            line(&[(16.75, 13.5), (16.75, 20.0)]);
            line(&[(13.5, 16.75), (20.0, 16.75)]);
        }
        Icon::Export => {
            page(5.0, 3.0, 17.0, 21.0);
            line(&[(11.0, 13.0), (21.0, 13.0)]);
            line(&[(18.0, 10.0), (21.0, 13.0), (18.0, 16.0)]);
        }
        Icon::Combine => {
            rect_r(3.5, 5.0, 11.0, 15.0, 1.0);
            rect_r(13.0, 5.0, 20.5, 15.0, 1.0);
            line(&[(7.25, 15.0), (7.25, 18.0), (16.75, 18.0), (16.75, 15.0)]);
            line(&[(12.0, 18.0), (12.0, 21.0)]);
        }
        Icon::Organize => {
            rect_r(3.5, 4.0, 10.0, 13.0, 1.0);
            rect_r(14.0, 11.0, 20.5, 20.0, 1.0);
            line(&[(12.0, 6.0), (17.0, 6.0), (17.0, 9.0)]);
            line(&[(15.5, 7.5), (17.0, 9.0), (18.5, 7.5)]);
        }
        Icon::Scan => {
            line(&[(4.0, 8.0), (4.0, 4.0), (8.0, 4.0)]);
            line(&[(16.0, 4.0), (20.0, 4.0), (20.0, 8.0)]);
            line(&[(20.0, 16.0), (20.0, 20.0), (16.0, 20.0)]);
            line(&[(8.0, 20.0), (4.0, 20.0), (4.0, 16.0)]);
            line(&[(8.0, 9.0), (16.0, 9.0)]);
            line(&[(12.0, 9.0), (12.0, 16.0)]);
        }
        Icon::Lock => {
            rect_r(5.0, 11.0, 19.0, 20.5, 2.0);
            line(&[
                (8.0, 11.0),
                (8.0, 8.0),
                (9.5, 4.8),
                (12.0, 4.0),
                (14.5, 4.8),
                (16.0, 8.0),
                (16.0, 11.0),
            ]);
            dot(12.0, 15.5, 1.3);
        }
        Icon::Redact => {
            page(5.0, 3.0, 19.0, 21.0);
            filled(&[(8.0, 9.0), (16.0, 9.0), (16.0, 12.0), (8.0, 12.0)], color);
            line(&[(8.0, 16.0), (14.0, 16.0)]);
        }
        Icon::Compress => {
            line(&[(12.0, 3.0), (12.0, 9.0)]);
            line(&[(9.0, 6.5), (12.0, 9.5), (15.0, 6.5)]);
            line(&[(12.0, 21.0), (12.0, 15.0)]);
            line(&[(9.0, 17.5), (12.0, 14.5), (15.0, 17.5)]);
            line(&[(4.0, 12.0), (20.0, 12.0)]);
        }
        Icon::Form => {
            rect_r(3.5, 4.5, 20.5, 9.5, 1.0);
            rect_r(3.5, 14.5, 9.0, 20.0, 1.0);
            line(&[(4.8, 17.2), (6.2, 18.6), (8.0, 15.8)]);
            line(&[(12.0, 17.0), (20.0, 17.0)]);
        }
        Icon::Certificate => {
            rect_r(3.5, 4.0, 20.5, 15.0, 1.5);
            line(&[(7.0, 8.0), (13.0, 8.0)]);
            line(&[(7.0, 11.0), (11.0, 11.0)]);
            circle(16.5, 15.0, 2.8);
            line(&[
                (15.0, 17.5),
                (14.5, 21.0),
                (16.5, 20.0),
                (18.5, 21.0),
                (18.0, 17.5),
            ]);
        }
        Icon::Compare => {
            rect_r(3.5, 4.0, 10.5, 20.0, 1.0);
            rect_r(13.5, 4.0, 20.5, 20.0, 1.0);
            line(&[(6.0, 9.0), (8.0, 9.0)]);
            line(&[(16.0, 9.0), (18.0, 9.0)]);
            line(&[(6.0, 13.0), (8.0, 13.0)]);
            line(&[(16.0, 14.0), (18.0, 14.0)]);
        }
        Icon::Archive => {
            rect_r(3.5, 4.5, 20.5, 9.0, 1.0);
            line(&[(5.0, 9.0), (5.0, 19.5), (19.0, 19.5), (19.0, 9.0)]);
            line(&[(10.0, 13.0), (14.0, 13.0)]);
        }
        Icon::More => {
            dot(6.0, 12.0, 1.6);
            dot(12.0, 12.0, 1.6);
            dot(18.0, 12.0, 1.6);
        }
        Icon::Help => {
            circle(12.0, 12.0, 9.0);
            line(&[
                (9.3, 9.5),
                (10.0, 7.8),
                (12.0, 7.0),
                (14.0, 7.8),
                (14.7, 9.5),
                (14.0, 11.0),
                (12.0, 12.3),
                (12.0, 14.0),
            ]);
            dot(12.0, 17.0, 1.2);
        }
        Icon::Settings => {
            circle(12.0, 12.0, 3.0);
            for i in 0..8 {
                let a = std::f32::consts::TAU * i as f32 / 8.0;
                line(&[
                    (12.0 + 6.0 * a.cos(), 12.0 + 6.0 * a.sin()),
                    (12.0 + 8.5 * a.cos(), 12.0 + 8.5 * a.sin()),
                ]);
            }
            circle(12.0, 12.0, 6.0);
        }
        Icon::Trash => {
            line(&[(4.0, 6.5), (20.0, 6.5)]);
            line(&[(9.0, 6.5), (9.5, 4.0), (14.5, 4.0), (15.0, 6.5)]);
            line(&[(6.0, 6.5), (7.0, 20.5), (17.0, 20.5), (18.0, 6.5)]);
            line(&[(10.0, 10.0), (10.0, 17.0)]);
            line(&[(14.0, 10.0), (14.0, 17.0)]);
        }
        Icon::Image => {
            rect_r(3.5, 5.0, 20.5, 19.0, 1.5);
            dot(8.5, 9.5, 1.6);
            line(&[
                (3.5, 17.0),
                (9.0, 12.0),
                (12.5, 15.0),
                (15.5, 12.5),
                (20.5, 16.5),
            ]);
        }
        Icon::Sanitize => {
            page(5.0, 3.0, 19.0, 21.0);
            line(&[(8.0, 13.0), (11.0, 16.0), (16.0, 10.0)]);
        }
        Icon::Insert => {
            page(5.0, 3.0, 19.0, 21.0);
            line(&[(12.0, 9.0), (12.0, 17.0)]);
            line(&[(8.0, 13.0), (16.0, 13.0)]);
        }
        Icon::Split => {
            rect_r(4.0, 4.0, 20.0, 9.5, 1.0);
            rect_r(4.0, 14.5, 20.0, 20.0, 1.0);
            line(&[(2.5, 12.0), (5.0, 12.0)]);
            line(&[(8.0, 12.0), (10.5, 12.0)]);
            line(&[(13.5, 12.0), (16.0, 12.0)]);
            line(&[(19.0, 12.0), (21.5, 12.0)]);
        }
        Icon::RotateRight => {
            let arc: Vec<(f32, f32)> = (0..=14)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + 1.4 * i as f32 / 14.0);
                    (12.0 + 7.5 * a.cos(), 13.0 + 7.5 * a.sin())
                })
                .collect();
            line(&arc);
            line(&[(15.5, 3.5), (18.0, 7.0), (14.0, 8.5)]);
        }
        Icon::Sun => {
            circle(12.0, 12.0, 4.0);
            for i in 0..8 {
                let a = std::f32::consts::TAU * i as f32 / 8.0;
                line(&[
                    (12.0 + 6.5 * a.cos(), 12.0 + 6.5 * a.sin()),
                    (12.0 + 9.0 * a.cos(), 12.0 + 9.0 * a.sin()),
                ]);
            }
        }
        Icon::Moon => {
            let outer: Vec<(f32, f32)> = (0..=20)
                .map(|i| {
                    let a =
                        0.35 * std::f32::consts::PI + 1.3 * std::f32::consts::PI * i as f32 / 20.0;
                    (12.0 + 8.0 * a.cos(), 12.0 + 8.0 * a.sin())
                })
                .collect();
            let inner: Vec<(f32, f32)> = (0..=20)
                .rev()
                .map(|i| {
                    let a =
                        0.45 * std::f32::consts::PI + 1.1 * std::f32::consts::PI * i as f32 / 20.0;
                    (15.0 + 6.5 * a.cos(), 9.5 + 6.5 * a.sin())
                })
                .collect();
            let mut all = outer;
            all.extend(inner);
            closed(&all);
        }
    }
}

/// Größe der normalen Symbol-Schaltflächen (quadratisch).
pub const BUTTON: f32 = 32.0;

/// Quadratische Schaltfläche mit Icon; `selected` hinterlegt sie in der Akzentfarbe.
pub fn button(ui: &mut Ui, icon: Icon, tooltip: &str, selected: bool) -> Response {
    sized_button(ui, icon, tooltip, selected, BUTTON)
}

pub fn sized_button(ui: &mut Ui, icon: Icon, tooltip: &str, selected: bool, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let enabled = ui.is_enabled();
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let fg = if !enabled {
            v.weak_text_color()
        } else if selected {
            v.selection.stroke.color
        } else {
            v.text_color()
        };
        if selected {
            ui.painter()
                .rect_filled(rect, 6.0, v.selection.bg_fill.gamma_multiply(0.35));
        } else if resp.hovered() && enabled {
            ui.painter()
                .rect_filled(rect, 6.0, v.widgets.hovered.weak_bg_fill);
        }
        paint(ui.painter(), rect.shrink(size * 0.2), icon, fg);
    }
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

/// Icon + Text in einer Zeile (für Listen wie "Alle Werkzeuge").
pub fn row_button(ui: &mut Ui, icon: Icon, text: &str, selected: bool) -> Response {
    let height = 34.0;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        if selected {
            ui.painter()
                .rect_filled(rect, 6.0, v.selection.bg_fill.gamma_multiply(0.35));
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, 6.0, v.widgets.hovered.weak_bg_fill);
        }
        let icon_rect = Rect::from_min_size(
            rect.left_center() + Vec2::new(8.0, -10.0),
            Vec2::splat(20.0),
        );
        let fg = if selected {
            v.selection.stroke.color
        } else {
            v.text_color()
        };
        paint(
            ui.painter(),
            icon_rect,
            icon,
            crate::style::icon_tint(icon, fg),
        );
        ui.painter().text(
            rect.left_center() + Vec2::new(38.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::TextStyle::Body.resolve(ui.style()),
            v.text_color(),
        );
    }
    resp
}

/// Flache Schaltfläche mit Icon und Text (Kopf-/Dokumentleiste).
pub fn text_button(ui: &mut Ui, icon: Icon, text: &str, selected: bool) -> Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, ui.visuals().text_color());
    let size = Vec2::new(10.0 + 18.0 + 6.0 + galley.size().x + 12.0, BUTTON);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        if selected {
            ui.painter()
                .rect_filled(rect, 6.0, v.selection.bg_fill.gamma_multiply(0.35));
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, 6.0, v.widgets.hovered.weak_bg_fill);
        }
        let fg = if selected {
            v.selection.stroke.color
        } else {
            v.text_color()
        };
        let icon_rect = Rect::from_min_size(
            rect.left_center() + Vec2::new(10.0, -9.0),
            Vec2::splat(18.0),
        );
        paint(ui.painter(), icon_rect, icon, fg);
        ui.painter().galley(
            rect.left_center() + Vec2::new(34.0, -galley.size().y / 2.0),
            galley,
            fg,
        );
    }
    resp
}

/// Ergebnis eines Tabs: Klick auf den Tab bzw. auf sein Schließen-Kreuz.
pub struct TabResponse {
    pub response: Response,
    pub close_clicked: bool,
}

impl TabResponse {
    pub fn clicked(&self) -> bool {
        self.response.clicked() && !self.close_clicked
    }
}

/// Tab in der Kopfleiste (Startseite, Dokumente); `closable` zeigt ein Kreuz.
pub fn tab(ui: &mut Ui, icon: Icon, text: &str, selected: bool, closable: bool) -> TabResponse {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, ui.visuals().text_color());
    let text_w = galley.size().x.min(220.0);
    let width = 12.0 + 16.0 + 8.0 + text_w + if closable { 30.0 } else { 14.0 };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, BUTTON + 2.0), Sense::click());
    let mut close_clicked = false;
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let fill = if selected {
            v.window_fill
        } else if response.hovered() {
            v.widgets.hovered.weak_bg_fill
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 8.0, fill);
        if selected {
            ui.painter().rect_stroke(
                rect,
                8.0,
                Stroke::new(1.0, crate::style::border(v.dark_mode)),
                StrokeKind::Inside,
            );
        }
        let fg = v.text_color();
        paint(
            ui.painter(),
            Rect::from_min_size(
                rect.left_center() + Vec2::new(12.0, -8.0),
                Vec2::splat(16.0),
            ),
            icon,
            fg,
        );
        let clip = Rect::from_min_size(
            rect.left_center() + Vec2::new(36.0, -galley.size().y / 2.0),
            Vec2::new(text_w, galley.size().y),
        );
        ui.painter()
            .with_clip_rect(clip)
            .galley(clip.min, galley, fg);
        if closable {
            let close = Rect::from_center_size(
                rect.right_center() - Vec2::new(16.0, 0.0),
                Vec2::splat(20.0),
            );
            let hovered = ui.rect_contains_pointer(close);
            if hovered {
                ui.painter()
                    .rect_filled(close, 4.0, v.widgets.active.weak_bg_fill);
            }
            paint(ui.painter(), close.shrink(5.0), Icon::Close, fg);
            close_clicked = hovered && response.clicked();
        }
    }
    let response = response.on_hover_text(text);
    TabResponse {
        response,
        close_clicked,
    }
}
