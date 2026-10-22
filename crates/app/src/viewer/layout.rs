//! Vertikales Seitenlayout mit Präfixsummen – O(log n) für "welche Seiten sind sichtbar?".
//!
//! Einheiten: Alle Rückgaben sind in egui-Punkten (logische Pixel) relativ zum
//! Inhaltsursprung des Scrollbereichs. Zoom 1.0 = 1 PDF-Punkt pro egui-Punkt.

use eframe::egui::{Rect, Vec2, pos2, vec2};

/// Abstand zwischen Seiten und zum Rand (zoomunabhängig, in egui-Punkten).
pub const GAP: f32 = 12.0;

pub struct Layout {
    /// Anzeigegröße jeder Seite in PDF-Punkten (nach /Rotate).
    sizes: Vec<(f32, f32)>,
    /// `cum_heights[i]` = Summe der Höhen der Seiten 0..i (Länge n+1).
    cum_heights: Vec<f32>,
    max_width: f32,
}

impl Layout {
    pub fn new(sizes: Vec<(f32, f32)>) -> Self {
        let mut cum_heights = Vec::with_capacity(sizes.len() + 1);
        let mut acc = 0.0;
        cum_heights.push(acc);
        for &(_, h) in &sizes {
            acc += h;
            cum_heights.push(acc);
        }
        let max_width = sizes.iter().map(|s| s.0).fold(0.0, f32::max);
        Self {
            sizes,
            cum_heights,
            max_width,
        }
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    pub fn page_size_pt(&self, i: usize) -> (f32, f32) {
        self.sizes[i]
    }

    /// Oberkante von Seite `i`.
    pub fn page_top(&self, i: usize, zoom: f32) -> f32 {
        GAP + self.cum_heights[i] * zoom + i as f32 * GAP
    }

    /// Gesamtgröße des Inhalts. `view_width` = verfügbare Breite, damit Seiten zentriert werden können.
    pub fn content_size(&self, zoom: f32, view_width: f32) -> Vec2 {
        let n = self.sizes.len();
        let h = self.page_top(n, zoom); // "Oberkante" der gedachten Seite n = Ende des Inhalts
        let w = (self.max_width * zoom + 2.0 * GAP).max(view_width);
        vec2(w, h)
    }

    /// Rechteck von Seite `i`, horizontal zentriert in `content_width`.
    pub fn page_rect(&self, i: usize, zoom: f32, content_width: f32) -> Rect {
        let (w, h) = self.sizes[i];
        let size = vec2(w * zoom, h * zoom);
        let x = ((content_width - size.x) / 2.0).max(GAP);
        Rect::from_min_size(pos2(x, self.page_top(i, zoom)), size)
    }

    /// Indexbereich der Seiten, die den vertikalen Bereich [y0, y1] berühren.
    pub fn visible_range(&self, y0: f32, y1: f32, zoom: f32) -> std::ops::Range<usize> {
        let n = self.sizes.len();
        if n == 0 {
            return 0..0;
        }
        // Erste Seite, deren Unterkante >= y0 ist.
        let first = self.partition(|i| self.page_top(i, zoom) + self.sizes[i].1 * zoom < y0);
        // Erste Seite, deren Oberkante > y1 ist.
        let end = self.partition(|i| self.page_top(i, zoom) <= y1);
        first.min(n)..end.max(first).min(n)
    }

    /// Seite, die bei y liegt (oder die nächstgelegene).
    pub fn page_at(&self, y: f32, zoom: f32) -> usize {
        let r = self.visible_range(y, y, zoom);
        r.start.min(self.sizes.len().saturating_sub(1))
    }

    /// Binärsuche: erster Index, für den `pred` false liefert (pred muss monoton sein).
    fn partition(&self, pred: impl Fn(usize) -> bool) -> usize {
        let (mut lo, mut hi) = (0, self.sizes.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if pred(mid) { lo = mid + 1 } else { hi = mid }
        }
        lo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a4(n: usize) -> Layout {
        Layout::new(vec![(595.0, 842.0); n])
    }

    #[test]
    fn tops_and_content_height() {
        let l = a4(3);
        assert_eq!(l.page_top(0, 1.0), GAP);
        assert_eq!(l.page_top(1, 1.0), GAP + 842.0 + GAP);
        assert_eq!(l.content_size(1.0, 100.0).y, 4.0 * GAP + 3.0 * 842.0);
    }

    #[test]
    fn visible_range_finds_pages() {
        let l = a4(500);
        // Ganz oben, Fensterhöhe 800 bei 100 %: nur Seite 0.
        assert_eq!(l.visible_range(0.0, 800.0, 1.0), 0..1);
        // Mitten in Seite 250 bei 50 % Zoom
        let y = l.page_top(250, 0.5) + 10.0;
        let r = l.visible_range(y, y + 400.0, 0.5);
        assert_eq!(r.start, 250);
        assert!(r.end >= 251);
        // Ganz am Ende
        let end = l.content_size(1.0, 0.0).y;
        assert_eq!(l.visible_range(end - 10.0, end, 1.0), 500..500);
    }

    #[test]
    fn page_at_lands_in_page() {
        let l = a4(10);
        let y = l.page_top(7, 2.0) + 5.0;
        assert_eq!(l.page_at(y, 2.0), 7);
    }

    #[test]
    fn page_rect_is_centered() {
        let l = Layout::new(vec![(100.0, 100.0), (200.0, 100.0)]);
        let r = l.page_rect(0, 1.0, 400.0);
        assert_eq!(r.min.x, 150.0);
    }
}
