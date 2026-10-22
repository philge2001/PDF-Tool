//! PDF -> Modell: liest die unterstützten Annotationen einer Seite.
//!
//! Grundsatz: Nur was wir vollständig verstehen, wird übernommen. Alles andere bleibt
//! "fremd" und wird beim Speichern unverändert durchgereicht.

use super::{AnnotKind, Annotation, MANAGED_SUBTYPES, Source};
use crate::geometry::{Color, Point, Quad, Rect};
use crate::pdf_string;
use lopdf::{Dictionary, Object, ObjectId};

/// Liefert die Einträge von /Annots einer Seite (Verweise oder direkte Dictionaries).
pub fn page_annots(pdf: &lopdf::Document, page_id: ObjectId) -> Vec<Object> {
    let Ok(page) = pdf.get_dictionary(page_id) else {
        return Vec::new();
    };
    let Ok(annots) = page.get(b"Annots") else {
        return Vec::new();
    };
    match pdf.dereference(annots) {
        Ok((_, Object::Array(arr))) => arr.clone(),
        _ => Vec::new(),
    }
}

/// Versucht eine Annotation zu importieren. `None` = nicht unterstützt / nicht verstanden.
pub fn parse(pdf: &lopdf::Document, page: usize, id: ObjectId) -> Option<Annotation> {
    let d = pdf.get_dictionary(id).ok()?;
    let subtype = d.get(b"Subtype").ok()?.as_name().ok()?;
    let subtype = std::str::from_utf8(subtype).ok()?;
    if !MANAGED_SUBTYPES.contains(&subtype) {
        return None;
    }
    let flags = int(pdf, d, b"F").unwrap_or(0);
    // Versteckte Annotationen (Bit 2) und Antworten auf andere Kommentare (/IRT) lassen wir in Ruhe.
    if flags & 2 != 0 || d.has(b"IRT") {
        return None;
    }
    let rect = rect(pdf, d, b"Rect")?;
    let width = border_width(pdf, d);

    let kind = match subtype {
        "Highlight" | "Underline" | "StrikeOut" => {
            let quads = quads(pdf, d).unwrap_or_else(|| vec![Quad::from_rect(rect)]);
            match subtype {
                "Highlight" => AnnotKind::Highlight { quads },
                "Underline" => AnnotKind::Underline { quads },
                _ => AnnotKind::StrikeOut { quads },
            }
        }
        "Text" => AnnotKind::Note {
            pos: Point::new(rect.x0, rect.y1),
        },
        "Ink" => {
            let list = array(pdf, d, b"InkList")?;
            let strokes: Vec<Vec<Point>> = list
                .iter()
                .filter_map(|s| numbers(pdf, s))
                .map(|v| v.chunks_exact(2).map(|c| Point::new(c[0], c[1])).collect())
                .filter(|s: &Vec<Point>| !s.is_empty())
                .collect();
            if strokes.is_empty() {
                return None;
            }
            AnnotKind::Ink {
                strokes,
                width: width.unwrap_or(1.0),
            }
        }
        "Square" => AnnotKind::Square {
            rect,
            width: width.unwrap_or(1.0),
        },
        "Circle" => AnnotKind::Circle {
            rect,
            width: width.unwrap_or(1.0),
        },
        "Line" => {
            let l = numbers(pdf, d.get(b"L").ok()?)?;
            if l.len() != 4 {
                return None;
            }
            let (mut start, mut end) = (Point::new(l[0], l[1]), Point::new(l[2], l[3]));
            let le: Vec<Vec<u8>> = array(pdf, d, b"LE")
                .map(|a| {
                    a.iter()
                        .filter_map(|o| o.as_name().ok().map(<[u8]>::to_vec))
                        .collect()
                })
                .unwrap_or_default();
            let is_arrow = |n: Option<&Vec<u8>>| n.is_some_and(|n| n.ends_with(b"Arrow"));
            let mut arrow_end = is_arrow(le.get(1));
            // Pfeil nur am Anfang? Dann Richtung umdrehen, damit er am Ende sitzt.
            if !arrow_end && is_arrow(le.first()) {
                std::mem::swap(&mut start, &mut end);
                arrow_end = true;
            }
            AnnotKind::Line {
                start,
                end,
                width: width.unwrap_or(1.0),
                arrow_end,
            }
        }
        "FreeText" => {
            let da = d
                .get(b"DA")
                .ok()
                .and_then(|o| o.as_str().ok())
                .unwrap_or_default();
            let (font_size, _) = parse_da(da);
            AnnotKind::FreeText {
                rect,
                font_size: font_size.unwrap_or(12.0),
            }
        }
        _ => return None,
    };

    let color = if matches!(kind, AnnotKind::FreeText { .. }) {
        let da = d
            .get(b"DA")
            .ok()
            .and_then(|o| o.as_str().ok())
            .unwrap_or_default();
        parse_da(da).1.unwrap_or(Color::BLACK)
    } else {
        color(pdf, d).unwrap_or(Color::YELLOW)
    };

    Some(Annotation {
        id: super::AnnotId(0),
        page,
        color,
        opacity: float(pdf, d, b"CA").unwrap_or(1.0).clamp(0.0, 1.0),
        contents: pdf_string::get(d, b"Contents"),
        author: pdf_string::get(d, b"T"),
        name: pdf_string::get(d, b"NM"),
        modified: pdf_string::get(d, b"M"),
        source: Some(Source {
            object_id: id,
            popup: d.get(b"Popup").ok().and_then(|o| o.as_reference().ok()),
        }),
        kind,
    })
}

// ---------- kleine Lesehelfer (folgen indirekten Verweisen) ----------

pub(crate) fn deref<'a>(pdf: &'a lopdf::Document, o: &'a Object) -> &'a Object {
    pdf.dereference(o).map(|(_, o)| o).unwrap_or(o)
}

pub(crate) fn float(pdf: &lopdf::Document, d: &Dictionary, key: &[u8]) -> Option<f32> {
    deref(pdf, d.get(key).ok()?).as_float().ok()
}

pub(crate) fn int(pdf: &lopdf::Document, d: &Dictionary, key: &[u8]) -> Option<i64> {
    deref(pdf, d.get(key).ok()?).as_i64().ok()
}

pub(crate) fn array<'a>(
    pdf: &'a lopdf::Document,
    d: &'a Dictionary,
    key: &[u8],
) -> Option<&'a Vec<Object>> {
    deref(pdf, d.get(key).ok()?).as_array().ok()
}

pub(crate) fn numbers(pdf: &lopdf::Document, o: &Object) -> Option<Vec<f32>> {
    deref(pdf, o)
        .as_array()
        .ok()?
        .iter()
        .map(|v| deref(pdf, v).as_float().ok())
        .collect()
}

pub(crate) fn rect(pdf: &lopdf::Document, d: &Dictionary, key: &[u8]) -> Option<Rect> {
    let v = numbers(pdf, d.get(key).ok()?)?;
    (v.len() == 4).then(|| Rect::new(v[0], v[1], v[2], v[3]).normalized())
}

fn quads(pdf: &lopdf::Document, d: &Dictionary) -> Option<Vec<Quad>> {
    let v = numbers(pdf, d.get(b"QuadPoints").ok()?)?;
    if v.is_empty() || v.len() % 8 != 0 {
        return None;
    }
    // Programme schreiben die Eckpunkte in unterschiedlicher Reihenfolge; wir normieren
    // jedes Viereck auf ein achsparalleles Rechteck (Text ist praktisch immer waagrecht).
    Some(
        v.chunks_exact(8)
            .filter_map(|c| Rect::bounding(c.chunks_exact(2).map(|p| Point::new(p[0], p[1]))))
            .map(Quad::from_rect)
            .collect(),
    )
}

fn color(pdf: &lopdf::Document, d: &Dictionary) -> Option<Color> {
    let v = numbers(pdf, d.get(b"C").ok()?)?;
    match v.as_slice() {
        [g] => Some(Color::rgb(*g, *g, *g)),
        [r, g, b] => Some(Color::rgb(*r, *g, *b)),
        [c, m, y, k] => Some(Color::rgb(
            (1.0 - c) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        )),
        _ => None,
    }
}

/// Linienbreite aus /BS /W oder (älter) /Border [h v w].
fn border_width(pdf: &lopdf::Document, d: &Dictionary) -> Option<f32> {
    if let Ok(bs) = d.get(b"BS")
        && let Ok(bs) = deref(pdf, bs).as_dict()
        && let Some(w) = float(pdf, bs, b"W")
    {
        return Some(w);
    }
    numbers(pdf, d.get(b"Border").ok()?)?.get(2).copied()
}

/// Liest Schriftgröße und Füllfarbe aus einem /DA-String wie "/Helv 12 Tf 1 0 0 rg".
pub fn parse_da(da: &[u8]) -> (Option<f32>, Option<Color>) {
    let text = String::from_utf8_lossy(da);
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let num = |i: usize| tokens.get(i).and_then(|t| t.parse::<f32>().ok());
    let mut size = None;
    let mut color = None;
    for (i, t) in tokens.iter().enumerate() {
        match *t {
            "Tf" if i >= 1 => size = num(i - 1).filter(|s| *s > 0.0),
            "rg" if i >= 3 => {
                if let (Some(r), Some(g), Some(b)) = (num(i - 3), num(i - 2), num(i - 1)) {
                    color = Some(Color::rgb(r, g, b));
                }
            }
            "g" if i >= 1 => color = num(i - 1).map(|g| Color::rgb(g, g, g)),
            _ => {}
        }
    }
    (size, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_da() {
        let (s, c) = parse_da(b"/Helv 14 Tf 1 0 0 rg");
        assert_eq!(s, Some(14.0));
        assert_eq!(c, Some(Color::rgb(1.0, 0.0, 0.0)));
        assert_eq!(
            parse_da(b"0.5 g /F1 0 Tf"),
            (None, Some(Color::rgb(0.5, 0.5, 0.5)))
        );
    }
}
