//! Erzeugt Appearance Streams (/AP /N): die PDF-Zeichenbefehle, mit denen jeder
//! Reader die Annotation genau so darstellt wie FolioCraft.
//!
//! Die Form-XObjects benutzen dieselben Koordinaten wie die Seite (BBox = /Rect,
//! Matrix = Identität) – dadurch sind keine Umrechnungen nötig.

use super::{AnnotKind, Annotation, NOTE_SIZE, arrow_size};
use crate::font;
use crate::geometry::{Point, Rect};
use lopdf::{Dictionary, dictionary};
use std::fmt::Write as _;

pub struct Appearance {
    pub bbox: Rect,
    pub content: Vec<u8>,
    pub resources: Dictionary,
}

/// Zahl kompakt formatieren (max. 3 Nachkommastellen, ohne überflüssige Nullen).
fn n(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

pub fn build(a: &Annotation) -> Appearance {
    let c = a.color;
    let rgb = format!("{} {} {}", n(c.r), n(c.g), n(c.b));
    // `String` als Puffer; `write!` hängt formatierten Text an (wie `format!`, nur ohne neue Allokation).
    let mut s = String::new();
    let mut gs = dictionary! { "Type" => "ExtGState", "CA" => a.opacity, "ca" => a.opacity };
    let mut resources = Dictionary::new();

    s.push_str("/GS0 gs\n");
    match &a.kind {
        AnnotKind::Highlight { quads } => {
            // Multiply: Text unter der Markierung bleibt gut lesbar.
            gs.set("BM", "Multiply");
            let _ = writeln!(s, "{rgb} rg");
            for q in quads {
                let r = q.bounds();
                let _ = writeln!(
                    s,
                    "{} {} {} {} re f",
                    n(r.x0),
                    n(r.y0),
                    n(r.width()),
                    n(r.height())
                );
            }
        }
        AnnotKind::Underline { quads } | AnnotKind::StrikeOut { quads } => {
            let strike = matches!(a.kind, AnnotKind::StrikeOut { .. });
            let _ = writeln!(s, "{rgb} RG");
            for q in quads {
                let r = q.bounds();
                let w = (r.height() * 0.07).max(0.5);
                let y = if strike {
                    r.y0 + r.height() * 0.52
                } else {
                    r.y0 + w
                };
                let _ = writeln!(
                    s,
                    "{} w {} {} m {} {} l S",
                    n(w),
                    n(r.x0),
                    n(y),
                    n(r.x1),
                    n(y)
                );
            }
        }
        AnnotKind::Note { pos } => {
            // Sprechblase: gefülltes Rechteck mit Spitze unten links, drei Textzeilen.
            let (x, y) = (pos.x, pos.y - NOTE_SIZE);
            let _ = writeln!(s, "{rgb} rg 0.25 0.25 0.25 RG 0.8 w 1 j");
            let _ = writeln!(
                s,
                "{} {} m {} {} l {} {} l {} {} l {} {} l {} {} l {} {} l h B",
                n(x + 1.0),
                n(y + 5.0),
                n(x + 1.0),
                n(y + 19.0),
                n(x + 19.0),
                n(y + 19.0),
                n(x + 19.0),
                n(y + 5.0),
                n(x + 9.0),
                n(y + 5.0),
                n(x + 4.0),
                n(y + 1.0),
                n(x + 5.0),
                n(y + 5.0),
            );
            for dy in [15.0, 12.0, 9.0] {
                let _ = writeln!(
                    s,
                    "{} {} m {} {} l S",
                    n(x + 4.0),
                    n(y + dy),
                    n(x + 16.0),
                    n(y + dy)
                );
            }
        }
        AnnotKind::Ink { strokes, width } => {
            let _ = writeln!(s, "{rgb} RG {} w 1 J 1 j", n(*width));
            for stroke in strokes {
                let Some(first) = stroke.first() else {
                    continue;
                };
                let _ = write!(s, "{} {} m", n(first.x), n(first.y));
                if stroke.len() == 1 {
                    // Einzelner Punkt: winzige Linie, damit die runde Kappe einen Punkt malt.
                    let _ = write!(s, " {} {} l", n(first.x + 0.01), n(first.y));
                }
                for p in &stroke[1..] {
                    let _ = write!(s, " {} {} l", n(p.x), n(p.y));
                }
                s.push_str(" S\n");
            }
        }
        AnnotKind::Square { rect, width } => {
            let r = rect.expand(-width / 2.0);
            let _ = writeln!(
                s,
                "{rgb} RG {} w {} {} {} {} re S",
                n(*width),
                n(r.x0),
                n(r.y0),
                n(r.width()),
                n(r.height())
            );
        }
        AnnotKind::Circle { rect, width } => {
            let r = rect.expand(-width / 2.0);
            let _ = writeln!(s, "{rgb} RG {} w", n(*width));
            ellipse(&mut s, r);
            s.push_str("S\n");
        }
        AnnotKind::Line {
            start,
            end,
            width,
            arrow_end,
        } => {
            let _ = writeln!(s, "{rgb} RG {} w 1 J 1 j", n(*width));
            let _ = writeln!(
                s,
                "{} {} m {} {} l S",
                n(start.x),
                n(start.y),
                n(end.x),
                n(end.y)
            );
            if *arrow_end {
                let (l, r) = arrow_head(*start, *end, arrow_size(*width));
                let _ = writeln!(
                    s,
                    "{} {} m {} {} l {} {} l S",
                    n(l.x),
                    n(l.y),
                    n(end.x),
                    n(end.y),
                    n(r.x),
                    n(r.y)
                );
            }
        }
        AnnotKind::FreeText { rect, font_size } => {
            resources.set("Font", dictionary! { "Helv" => helvetica_dict() });
            let pad = 2.0;
            let lines = font::wrap(&a.contents, *font_size, rect.width() - 2.0 * pad);
            let leading = font_size * 1.2;
            // Ausschneiden auf das Rechteck, damit überlanger Text nicht herausragt.
            let _ = writeln!(
                s,
                "q {} {} {} {} re W n",
                n(rect.x0),
                n(rect.y0),
                n(rect.width()),
                n(rect.height())
            );
            let _ = writeln!(
                s,
                "BT /Helv {} Tf {rgb} rg {} TL {} {} Td",
                n(*font_size),
                n(leading),
                n(rect.x0 + pad),
                n(rect.y1 - pad - font_size * 0.8),
            );
            for (i, line) in lines.iter().enumerate() {
                if i > 0 {
                    s.push_str("T* ");
                }
                s.push_str(&pdf_literal(&font::winansi_encode(line)));
                s.push_str(" Tj\n");
            }
            s.push_str("ET Q\n");
        }
    }
    resources.set("ExtGState", dictionary! { "GS0" => gs });

    Appearance {
        bbox: a.bounds(),
        content: s.into_bytes(),
        resources,
    }
}

pub fn helvetica_dict() -> Dictionary {
    dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    }
}

/// Die beiden Endpunkte der offenen Pfeilspitze an `end` (Schenkel ±25° zur Linie).
pub fn arrow_head(start: Point, end: Point, size: f32) -> (Point, Point) {
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / len, dy / len);
    let (sin, cos) = 25f32.to_radians().sin_cos();
    let rot = |sx: f32| {
        Point::new(
            end.x - size * (ux * cos - sx * uy * sin),
            end.y - size * (uy * cos + sx * ux * sin),
        )
    };
    (rot(1.0), rot(-1.0))
}

/// Ellipse aus vier Bézierkurven (Standard-Annäherung mit k ≈ 0,5523).
fn ellipse(s: &mut String, r: Rect) {
    const K: f32 = 0.552_284_8;
    let (cx, cy) = (r.center().x, r.center().y);
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    let (kx, ky) = (rx * K, ry * K);
    let _ = writeln!(s, "{} {} m", n(cx + rx), n(cy));
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        n(cx + rx),
        n(cy + ky),
        n(cx + kx),
        n(cy + ry),
        n(cx),
        n(cy + ry)
    );
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        n(cx - kx),
        n(cy + ry),
        n(cx - rx),
        n(cy + ky),
        n(cx - rx),
        n(cy)
    );
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        n(cx - rx),
        n(cy - ky),
        n(cx - kx),
        n(cy - ry),
        n(cx),
        n(cy - ry)
    );
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        n(cx + kx),
        n(cy - ry),
        n(cx + rx),
        n(cy - ky),
        n(cx + rx),
        n(cy)
    );
}

/// Bytes als PDF-Literal-String `( … )` mit maskierten Sonderzeichen.
fn pdf_literal(bytes: &[u8]) -> String {
    let mut out = String::from("(");
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(b as char);
            }
            32..=126 => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}"); // Oktal-Escape, z. B. ü -> \374
            }
        }
    }
    out.push(')');
    out
}

/// Das fertige Form-XObject als lopdf-Stream.
pub fn to_stream(ap: Appearance) -> lopdf::Stream {
    let b = ap.bbox;
    let dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => vec![b.x0.into(), b.y0.into(), b.x1.into(), b.y1.into()],
        "Resources" => ap.resources,
    };
    lopdf::Stream::new(dict, ap.content)
}

/// Inhalt als Text (für Tests).
#[cfg(test)]
pub fn content_text(a: &Annotation) -> String {
    String::from_utf8_lossy(&build(a).content).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Color, Quad};

    #[test]
    fn number_format() {
        assert_eq!(n(1.0), "1");
        assert_eq!(n(0.5), "0.5");
        assert_eq!(n(-0.0001), "0");
        assert_eq!(n(12.3456), "12.346");
    }

    #[test]
    fn highlight_uses_multiply() {
        let a = Annotation::new(
            0,
            AnnotKind::Highlight {
                quads: vec![Quad::from_rect(Rect::new(10.0, 20.0, 110.0, 32.0))],
            },
            Color::YELLOW,
        );
        let ap = build(&a);
        assert_eq!(ap.bbox, Rect::new(10.0, 20.0, 110.0, 32.0));
        assert!(content_text(&a).contains("10 20 100 12 re f"));
        let gs = ap.resources.get(b"ExtGState").unwrap().as_dict().unwrap();
        let gs0 = gs.get(b"GS0").unwrap().as_dict().unwrap();
        assert_eq!(gs0.get(b"BM").unwrap().as_name().unwrap(), b"Multiply");
    }

    #[test]
    fn freetext_escapes_and_encodes() {
        let mut a = Annotation::new(
            0,
            AnnotKind::FreeText {
                rect: Rect::new(0.0, 0.0, 300.0, 50.0),
                font_size: 12.0,
            },
            Color::BLACK,
        );
        a.contents = "Grüße (Test)".into();
        let text = content_text(&a);
        assert!(text.contains(r"(Gr\374\337e \(Test\)) Tj"), "{text}");
    }

    #[test]
    fn arrow_head_points_back() {
        let (l, r) = arrow_head(Point::new(0.0, 0.0), Point::new(100.0, 0.0), 10.0);
        assert!(l.x < 100.0 && r.x < 100.0);
        assert!((l.y + r.y).abs() < 1e-4); // symmetrisch
    }
}
