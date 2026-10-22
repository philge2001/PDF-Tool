//! Seitentext mit Zeichenpositionen: Suche, Bereiche, Zeilen-Quads.
//!
//! Der Text wird vom Render-Worker (PDFium) extrahiert; alles hier ist reine Logik.

use crate::geometry::{Point, Quad, Rect};

/// Ein Zeichen mit seinem Begrenzungsrechteck im PDF-Benutzerraum.
#[derive(Debug, Clone, PartialEq)]
pub struct CharBox {
    pub ch: char,
    /// Leeres Rechteck (`Rect::default()`) bei generierten Zeichen ohne Position
    /// (z. B. Zeilenumbrüche, die PDFium beim Extrahieren einfügt).
    pub rect: Rect,
}

impl CharBox {
    pub fn has_area(&self) -> bool {
        self.rect.width() > 0.0 && self.rect.height() > 0.0
    }
}

/// Halb-offener Bereich von Zeichenindizes `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

impl TextRange {
    pub fn new(a: usize, b: usize) -> Self {
        Self {
            start: a.min(b),
            end: a.max(b),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// Text einer Seite in Lesereihenfolge (wie von PDFium geliefert).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageText {
    pub chars: Vec<CharBox>,
}

impl PageText {
    pub fn new(chars: Vec<CharBox>) -> Self {
        Self { chars }
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Text eines Bereichs. Zeilenumbrüche aus `\r\n` werden zu `\n` vereinheitlicht.
    pub fn text_in(&self, range: TextRange) -> String {
        let end = range.end.min(self.chars.len());
        let start = range.start.min(end);
        self.chars[start..end]
            .iter()
            .map(|c| c.ch)
            .filter(|c| *c != '\r')
            .collect()
    }

    /// Findet alle (nicht überlappenden) Vorkommen von `query`.
    ///
    /// Jeder Whitespace (Leerzeichen, Zeilenumbruch, Tab) passt auf jeden anderen,
    /// sodass "Lorem ipsum" auch über einen Zeilenumbruch hinweg gefunden wird,
    /// solange PDFium dort genau ein Trennzeichen liefert.
    pub fn find(&self, query: &str, case_sensitive: bool) -> Vec<TextRange> {
        let norm = |c: char| -> char {
            if c.is_whitespace() {
                ' '
            } else if case_sensitive {
                c
            } else {
                // `to_lowercase` kann mehrere Zeichen liefern (z. B. 'İ'); wir nehmen das erste,
                // damit jedes Zeichen genau einem Zeichen entspricht und die Indizes stimmen.
                c.to_lowercase().next().unwrap_or(c)
            }
        };
        let needle: Vec<char> = query.trim().chars().map(norm).collect();
        if needle.is_empty() {
            return Vec::new();
        }
        let hay: Vec<char> = self.chars.iter().map(|c| norm(c.ch)).collect();
        let mut hits = Vec::new();
        let mut i = 0;
        // `windows` wäre eleganter, aber wir wollen nach einem Treffer überspringen.
        while i + needle.len() <= hay.len() {
            if hay[i..i + needle.len()] == needle[..] {
                hits.push(TextRange {
                    start: i,
                    end: i + needle.len(),
                });
                i += needle.len();
            } else {
                i += 1;
            }
        }
        hits
    }

    /// Zerlegt einen Bereich in ein Rechteck pro Zeile (für Markierungen und Text-Markup).
    pub fn range_rects(&self, range: TextRange) -> Vec<Rect> {
        let end = range.end.min(self.chars.len());
        let mut lines: Vec<Rect> = Vec::new();
        for c in self.chars[range.start.min(end)..end]
            .iter()
            .filter(|c| c.has_area())
        {
            match lines.last_mut() {
                Some(line) if same_line(line, &c.rect) => *line = line.union(&c.rect),
                _ => lines.push(c.rect),
            }
        }
        lines
    }

    pub fn range_quads(&self, range: TextRange) -> Vec<Quad> {
        self.range_rects(range)
            .into_iter()
            .map(Quad::from_rect)
            .collect()
    }

    /// Index des Zeichens an (oder nächstgelegen zu) einem Punkt – für die Textauswahl.
    /// Liefert `None`, wenn die Seite keinen Text hat. Mit `max_dist` lässt sich
    /// begrenzen, wie weit der Punkt vom Zeichen entfernt sein darf.
    pub fn char_at(&self, p: Point, max_dist: f32) -> Option<usize> {
        self.chars
            .iter()
            .enumerate()
            .filter(|(_, c)| c.has_area())
            .map(|(i, c)| (i, distance_to_rect(p, &c.rect)))
            .filter(|(_, d)| *d <= max_dist)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

impl PageText {
    /// Caret-Position (0..=len) an einem Punkt: vor dem nächstgelegenen Zeichen, oder
    /// dahinter, wenn der Punkt in dessen rechter Hälfte liegt (wie in einem Texteditor).
    pub fn caret_at(&self, p: Point, max_dist: f32) -> Option<usize> {
        let i = self.char_at(p, max_dist)?;
        let r = self.chars[i].rect;
        Some(if p.x > (r.x0 + r.x1) / 2.0 { i + 1 } else { i })
    }

    /// Wort um das Zeichen `i` (Buchstaben, Ziffern, Bindestrich, Unterstrich).
    pub fn word_at(&self, i: usize) -> Option<TextRange> {
        let is_word = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
        if !is_word(self.chars.get(i)?.ch) {
            return Some(TextRange::new(i, i + 1));
        }
        let start = self.chars[..i]
            .iter()
            .rposition(|c| !is_word(c.ch))
            .map_or(0, |p| p + 1);
        let end = self.chars[i..]
            .iter()
            .position(|c| !is_word(c.ch))
            .map_or(self.chars.len(), |p| i + p);
        Some(TextRange::new(start, end))
    }
}

/// Zwei Rechtecke gehören zur selben Zeile, wenn sie sich vertikal mindestens zur
/// Hälfte der kleineren Höhe überlappen.
fn same_line(a: &Rect, b: &Rect) -> bool {
    let overlap = a.y1.min(b.y1) - a.y0.max(b.y0);
    overlap >= 0.5 * a.height().min(b.height())
}

fn distance_to_rect(p: Point, r: &Rect) -> f32 {
    let dx = (r.x0 - p.x).max(0.0).max(p.x - r.x1);
    let dy = (r.y0 - p.y).max(0.0).max(p.y - r.y1);
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Baut Testtext: jede Zeile auf eigener Höhe, Zeichen 10 pt breit, getrennt durch "\r\n".
    pub(crate) fn sample(lines: &[&str]) -> PageText {
        let mut chars = Vec::new();
        for (row, line) in lines.iter().enumerate() {
            if row > 0 {
                chars.push(CharBox {
                    ch: '\r',
                    rect: Rect::default(),
                });
                chars.push(CharBox {
                    ch: '\n',
                    rect: Rect::default(),
                });
            }
            let y = 700.0 - row as f32 * 20.0;
            for (col, ch) in line.chars().enumerate() {
                let x = 72.0 + col as f32 * 10.0;
                chars.push(CharBox {
                    ch,
                    rect: Rect::new(x, y, x + 10.0, y + 12.0),
                });
            }
        }
        PageText::new(chars)
    }

    #[test]
    fn finds_case_insensitive() {
        let t = sample(&["Zebra und ZEBRA", "zebra"]);
        let hits = t.find("zebra", false);
        assert_eq!(hits.len(), 3);
        assert_eq!(t.text_in(hits[1]), "ZEBRA");
        assert_eq!(t.find("zebra", true).len(), 1);
    }

    #[test]
    fn finds_umlauts() {
        let t = sample(&["Grüße aus", "Köln"]);
        assert_eq!(t.find("GRÜSSE", false).len(), 0); // ß wird nicht zu ss erweitert
        assert_eq!(t.find("grüße", false).len(), 1);
        assert_eq!(t.find("köln", false).len(), 1);
    }

    #[test]
    fn empty_query_finds_nothing() {
        assert!(sample(&["abc"]).find("  ", false).is_empty());
    }

    #[test]
    fn range_rects_split_per_line() {
        let t = sample(&["abc", "def"]);
        // Bereich von 'b' (1) bis einschließlich 'e' (Index 6: a b c \r \n d e)
        let rects = t.range_rects(TextRange::new(1, 7));
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], Rect::new(82.0, 700.0, 102.0, 712.0));
        assert_eq!(rects[1], Rect::new(72.0, 680.0, 92.0, 692.0));
        assert_eq!(t.text_in(TextRange::new(1, 7)), "bc\nde");
    }

    #[test]
    fn caret_before_or_after_char() {
        let t = sample(&["abc"]);
        // 'b' liegt bei x = 82..92: linke Hälfte -> Caret 1, rechte Hälfte -> Caret 2.
        assert_eq!(t.caret_at(Point::new(84.0, 705.0), 1.0), Some(1));
        assert_eq!(t.caret_at(Point::new(90.0, 705.0), 1.0), Some(2));
    }

    #[test]
    fn word_at_expands_to_word() {
        let t = sample(&["foo bar-baz qux"]);
        let w = t.word_at(5).unwrap(); // 'a' in "bar-baz"
        assert_eq!(t.text_in(w), "bar-baz");
        assert_eq!(t.text_in(t.word_at(3).unwrap()), " ");
    }

    #[test]
    fn char_at_point() {
        let t = sample(&["abc"]);
        assert_eq!(t.char_at(Point::new(95.0, 705.0), 1.0), Some(2));
        assert_eq!(t.char_at(Point::new(300.0, 705.0), 5.0), None);
        assert_eq!(t.char_at(Point::new(300.0, 705.0), f32::INFINITY), Some(2));
    }
}
