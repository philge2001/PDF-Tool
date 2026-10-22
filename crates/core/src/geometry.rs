//! Geometrie im PDF-Benutzerraum: Einheit Punkt (1/72 Zoll), Ursprung unten links,
//! y wächst nach oben. Rotation (/Rotate) wird hier NICHT angewendet – das macht die
//! Anzeige-Transformation in der App.

/// Ein Punkt. `Copy` heißt: Zuweisungen kopieren den Wert, statt ihn zu verschieben.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn offset(self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }

    pub fn distance(self, other: Point) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/// Achsparalleles Rechteck. Invariante nach `normalized()`: x0 <= x1, y0 <= y1.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// Rechteck aus zwei beliebigen Eckpunkten (z. B. Start und Ende eines Mauszugs).
    pub fn from_points(a: Point, b: Point) -> Self {
        Self::new(a.x, a.y, b.x, b.y).normalized()
    }

    pub fn normalized(self) -> Self {
        Self {
            x0: self.x0.min(self.x1),
            y0: self.y0.min(self.y1),
            x1: self.x0.max(self.x1),
            y1: self.y0.max(self.y1),
        }
    }

    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    pub fn center(&self) -> Point {
        Point::new((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.x0 <= o.x1 && o.x0 <= self.x1 && self.y0 <= o.y1 && o.y0 <= self.y1
    }

    /// Kleinstes Rechteck, das beide umschließt.
    pub fn union(&self, o: &Rect) -> Rect {
        Rect::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }

    /// Rechteck um `d` in alle Richtungen vergrößern (negativ = verkleinern).
    pub fn expand(&self, d: f32) -> Rect {
        Rect::new(self.x0 - d, self.y0 - d, self.x1 + d, self.y1 + d)
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x0 + dx, self.y0 + dy, self.x1 + dx, self.y1 + dy)
    }

    /// Umschließendes Rechteck einer Punktmenge; `None` bei leerer Menge.
    pub fn bounding<I: IntoIterator<Item = Point>>(points: I) -> Option<Rect> {
        let mut it = points.into_iter();
        let first = it.next()?; // `?` auf Option: bei None sofort None zurückgeben
        let init = Rect::new(first.x, first.y, first.x, first.y);
        Some(it.fold(init, |r, p| {
            Rect::new(r.x0.min(p.x), r.y0.min(p.y), r.x1.max(p.x), r.y1.max(p.y))
        }))
    }
}

/// Viereck für Text-Markup (/QuadPoints). Reihenfolge wie von Acrobat geschrieben und
/// von allen gängigen Readern erwartet: oben-links, oben-rechts, unten-links, unten-rechts.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Quad {
    pub points: [Point; 4],
}

impl Quad {
    /// Quad aus einem achsparallelen Rechteck.
    pub fn from_rect(r: Rect) -> Self {
        Self {
            points: [
                Point::new(r.x0, r.y1),
                Point::new(r.x1, r.y1),
                Point::new(r.x0, r.y0),
                Point::new(r.x1, r.y0),
            ],
        }
    }

    pub fn bounds(&self) -> Rect {
        // `unwrap_or_default` ist hier sicher: ein Quad hat immer genau 4 Punkte.
        Rect::bounding(self.points).unwrap_or_default()
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Quad {
        Quad {
            points: self.points.map(|p| p.offset(dx, dy)),
        }
    }
}

/// RGB-Farbe mit Komponenten 0.0..=1.0 (wie im PDF-Farbraum DeviceRGB).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Color {
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    pub const YELLOW: Color = Color::rgb(1.0, 0.86, 0.0);
    pub const RED: Color = Color::rgb(0.9, 0.1, 0.1);
    pub const GREEN: Color = Color::rgb(0.1, 0.65, 0.2);
    pub const BLUE: Color = Color::rgb(0.1, 0.35, 0.9);
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);

    pub fn to_rgb8(self) -> [u8; 3] {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [c(self.r), c(self.g), c(self.b)]
    }

    pub fn from_rgb8([r, g, b]: [u8; 3]) -> Self {
        Self::rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_from_points_is_normalized() {
        let r = Rect::from_points(Point::new(10.0, 50.0), Point::new(2.0, 5.0));
        assert_eq!(r, Rect::new(2.0, 5.0, 10.0, 50.0));
        assert_eq!(r.width(), 8.0);
        assert!(r.contains(Point::new(5.0, 5.0)));
    }

    #[test]
    fn quad_roundtrip_bounds() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(Quad::from_rect(r).bounds(), r);
    }

    #[test]
    fn color_rgb8_roundtrip() {
        let c = Color::from_rgb8([255, 128, 0]);
        assert_eq!(c.to_rgb8(), [255, 128, 0]);
    }
}
