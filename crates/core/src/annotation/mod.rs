//! Annotationsmodell: unterstützte Typen, Geometrie, Treffertest.
//!
//! Koordinaten sind immer PDF-Benutzerraum (siehe `geometry`).

pub mod appearance;
pub mod export;
pub mod import;
pub mod store;

use crate::geometry::{Color, Point, Quad, Rect};
use lopdf::ObjectId;

pub use store::AnnotationStore;

/// Sitzungsweit eindeutige ID einer Annotation (nicht im PDF gespeichert).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AnnotId(pub u64);

/// Seitenlänge des Notiz-Symbols in Punkt.
pub const NOTE_SIZE: f32 = 20.0;

/// Die Art der Annotation samt typspezifischer Geometrie.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotKind {
    /// Text-Markup: ein Viereck pro Textzeile.
    Highlight {
        quads: Vec<Quad>,
    },
    Underline {
        quads: Vec<Quad>,
    },
    StrikeOut {
        quads: Vec<Quad>,
    },
    /// Haftnotiz (/Text). `pos` = linke obere Ecke des Symbols.
    Note {
        pos: Point,
    },
    /// Freihand: mehrere Striche aus Punkten.
    Ink {
        strokes: Vec<Vec<Point>>,
        width: f32,
    },
    Square {
        rect: Rect,
        width: f32,
    },
    Circle {
        rect: Rect,
        width: f32,
    },
    /// Linie (/Line), optional mit Pfeilspitze am Ende.
    Line {
        start: Point,
        end: Point,
        width: f32,
        arrow_end: bool,
    },
    /// Freitext direkt auf der Seite; der Text steht in `Annotation::contents`.
    FreeText {
        rect: Rect,
        font_size: f32,
    },
    /// Zum Schwärzen markierter Bereich (/Redact) – erst beim Anwenden wird der Inhalt
    /// darunter wirklich entfernt. Ein Viereck pro Bereich bzw. Textzeile.
    Redact {
        quads: Vec<Quad>,
    },
}

impl AnnotKind {
    /// PDF-Subtyp (/Subtype).
    pub fn subtype(&self) -> &'static str {
        match self {
            AnnotKind::Highlight { .. } => "Highlight",
            AnnotKind::Underline { .. } => "Underline",
            AnnotKind::StrikeOut { .. } => "StrikeOut",
            AnnotKind::Note { .. } => "Text",
            AnnotKind::Ink { .. } => "Ink",
            AnnotKind::Square { .. } => "Square",
            AnnotKind::Circle { .. } => "Circle",
            AnnotKind::Line { .. } => "Line",
            AnnotKind::FreeText { .. } => "FreeText",
            AnnotKind::Redact { .. } => "Redact",
        }
    }

    /// Deutscher Anzeigename für die UI.
    pub fn label(&self) -> &'static str {
        match self {
            AnnotKind::Highlight { .. } => "Hervorhebung",
            AnnotKind::Underline { .. } => "Unterstreichung",
            AnnotKind::StrikeOut { .. } => "Durchstreichung",
            AnnotKind::Note { .. } => "Notiz",
            AnnotKind::Ink { .. } => "Freihand",
            AnnotKind::Square { .. } => "Rechteck",
            AnnotKind::Circle { .. } => "Ellipse",
            AnnotKind::Line {
                arrow_end: true, ..
            } => "Pfeil",
            AnnotKind::Line { .. } => "Linie",
            AnnotKind::FreeText { .. } => "Freitext",
            AnnotKind::Redact { .. } => "Schwärzung",
        }
    }

    pub fn is_text_markup(&self) -> bool {
        matches!(
            self,
            AnnotKind::Highlight { .. } | AnnotKind::Underline { .. } | AnnotKind::StrikeOut { .. }
        )
    }
}

/// Alle Subtypen, die PDF Tool selbst verwaltet (importiert, zeichnet, speichert).
pub const MANAGED_SUBTYPES: [&str; 10] = [
    "Highlight",
    "Underline",
    "StrikeOut",
    "Text",
    "Ink",
    "Square",
    "Circle",
    "Line",
    "FreeText",
    "Redact",
];

/// Herkunft einer importierten Annotation in der PDF-Datei.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Source {
    pub object_id: ObjectId,
    /// Zugehöriges Popup-Objekt (/Popup), wird beim Löschen mit entfernt.
    pub popup: Option<ObjectId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub id: AnnotId,
    /// Seitenindex (0-basiert).
    pub page: usize,
    pub kind: AnnotKind,
    pub color: Color,
    /// Deckkraft 0.0..=1.0 (/CA).
    pub opacity: f32,
    /// Kommentartext (/Contents). Bei FreeText der angezeigte Text.
    pub contents: String,
    /// Autor (/T).
    pub author: String,
    /// Eindeutiger Name (/NM) – bleibt über Speichern/Laden erhalten.
    pub name: String,
    /// Änderungsdatum im PDF-Format (/M), z. B. "D:20261022103000Z".
    pub modified: String,
    /// `Some`, wenn die Annotation aus der Datei stammt.
    pub source: Option<Source>,
}

impl Annotation {
    /// Neue Annotation (noch ohne ID – die vergibt der Store beim Einfügen).
    pub fn new(page: usize, kind: AnnotKind, color: Color) -> Self {
        Self {
            id: AnnotId(0),
            page,
            kind,
            color,
            // Auch Hervorhebungen deckend: Sie werden mit "Multiply" gezeichnet, Text bleibt lesbar.
            opacity: 1.0,
            contents: String::new(),
            author: String::new(),
            name: String::new(),
            modified: String::new(),
            source: None,
        }
    }

    /// Umschließendes Rechteck (wird zu /Rect). Linienbreiten sind eingerechnet.
    pub fn bounds(&self) -> Rect {
        match &self.kind {
            AnnotKind::Highlight { quads }
            | AnnotKind::Underline { quads }
            | AnnotKind::StrikeOut { quads }
            | AnnotKind::Redact { quads } => {
                Rect::bounding(quads.iter().flat_map(|q| q.points)).unwrap_or_default()
            }
            AnnotKind::Note { pos } => {
                Rect::new(pos.x, pos.y - NOTE_SIZE, pos.x + NOTE_SIZE, pos.y)
            }
            AnnotKind::Ink { strokes, width } => Rect::bounding(strokes.iter().flatten().copied())
                .unwrap_or_default()
                .expand(width / 2.0 + 1.0),
            AnnotKind::Square { rect, .. } | AnnotKind::Circle { rect, .. } => *rect,
            AnnotKind::Line {
                start,
                end,
                width,
                arrow_end,
            } => {
                let pad = if *arrow_end {
                    arrow_size(*width)
                } else {
                    width / 2.0
                } + 1.0;
                Rect::from_points(*start, *end).expand(pad)
            }
            AnnotKind::FreeText { rect, .. } => *rect,
        }
    }

    /// Verschiebt die Annotation.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        match &mut self.kind {
            AnnotKind::Highlight { quads }
            | AnnotKind::Underline { quads }
            | AnnotKind::StrikeOut { quads }
            | AnnotKind::Redact { quads } => {
                for q in quads.iter_mut() {
                    *q = q.translate(dx, dy);
                }
            }
            AnnotKind::Note { pos } => *pos = pos.offset(dx, dy),
            AnnotKind::Ink { strokes, .. } => {
                for p in strokes.iter_mut().flatten() {
                    *p = p.offset(dx, dy);
                }
            }
            AnnotKind::Square { rect, .. }
            | AnnotKind::Circle { rect, .. }
            | AnnotKind::FreeText { rect, .. } => *rect = rect.translate(dx, dy),
            AnnotKind::Line { start, end, .. } => {
                *start = start.offset(dx, dy);
                *end = end.offset(dx, dy);
            }
        }
    }

    /// Liegt `p` auf der Annotation? `tol` = Toleranz in Punkt (für dünne Linien).
    pub fn hit_test(&self, p: Point, tol: f32) -> bool {
        match &self.kind {
            AnnotKind::Highlight { quads }
            | AnnotKind::Underline { quads }
            | AnnotKind::StrikeOut { quads }
            | AnnotKind::Redact { quads } => {
                quads.iter().any(|q| q.bounds().expand(tol).contains(p))
            }
            AnnotKind::Ink { strokes, width } => strokes.iter().any(|s| {
                s.windows(2)
                    .any(|w| dist_to_segment(p, w[0], w[1]) <= width / 2.0 + tol)
                    || (s.len() == 1 && s[0].distance(p) <= width / 2.0 + tol)
            }),
            AnnotKind::Line {
                start, end, width, ..
            } => dist_to_segment(p, *start, *end) <= width / 2.0 + tol,
            // Formen: Rand ODER Innenfläche zählt – so lassen sie sich leichter greifen.
            _ => self.bounds().expand(tol).contains(p),
        }
    }
}

/// Länge der Pfeilspitzen-Schenkel für eine Linienbreite.
pub fn arrow_size(width: f32) -> f32 {
    (width * 4.0).max(8.0)
}

fn dist_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_and_translate() {
        let mut a = Annotation::new(
            0,
            AnnotKind::Line {
                start: Point::new(0.0, 0.0),
                end: Point::new(100.0, 0.0),
                width: 2.0,
                arrow_end: false,
            },
            Color::RED,
        );
        assert_eq!(a.bounds(), Rect::new(-2.0, -2.0, 102.0, 2.0));
        a.translate(10.0, 5.0);
        assert_eq!(a.bounds(), Rect::new(8.0, 3.0, 112.0, 7.0));
    }

    #[test]
    fn hit_test_line_and_ink() {
        let line = Annotation::new(
            0,
            AnnotKind::Line {
                start: Point::new(0.0, 0.0),
                end: Point::new(100.0, 100.0),
                width: 2.0,
                arrow_end: true,
            },
            Color::RED,
        );
        assert!(line.hit_test(Point::new(50.0, 51.0), 2.0));
        assert!(!line.hit_test(Point::new(50.0, 70.0), 2.0));

        let ink = Annotation::new(
            0,
            AnnotKind::Ink {
                strokes: vec![vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)]],
                width: 1.0,
            },
            Color::BLUE,
        );
        assert!(ink.hit_test(Point::new(5.0, 1.0), 1.0));
        assert!(!ink.hit_test(Point::new(5.0, 5.0), 1.0));
    }
}
