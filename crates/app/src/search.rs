//! Textsuche: Suchleiste, Treffer sammeln (seitenweise, sobald Text vorliegt), Hervorhebung.

use crate::app::OpenDocument;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, Color32, Key, Stroke, StrokeKind};
use foliocraft_core::Rect;
use foliocraft_render::RenderService;

pub struct SearchHit {
    pub page: usize,
    /// Ein Rechteck pro Textzeile, PDF-Koordinaten.
    pub rects: Vec<Rect>,
}

#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    pub case_sensitive: bool,
    /// Treffer, sortiert nach (Seite, Position).
    pub hits: Vec<SearchHit>,
    pub current: Option<usize>,
    /// Welche Seiten für die aktuelle Anfrage schon durchsucht sind.
    searched: Vec<bool>,
    searched_count: usize,
    /// Anfrage, für die `hits` gilt (Text, Groß-/Kleinschreibung).
    active: Option<(String, bool)>,
    /// Beim ersten Treffer automatisch dorthin springen.
    jump_to_first: bool,
    focus_field: bool,
}

impl SearchState {
    /// Öffnet die Suchleiste und setzt den Fokus ins Eingabefeld.
    pub fn open(&mut self) {
        self.open = true;
        self.focus_field = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.hits.clear();
        self.current = None;
        self.active = None;
    }

    /// Indexbereich der Treffer auf `page` (Treffer sind nach Seite sortiert -> Binärsuche).
    fn hits_on_page(&self, page: usize) -> std::ops::Range<usize> {
        let start = self.hits.partition_point(|h| h.page < page);
        let end = self.hits.partition_point(|h| h.page <= page);
        start..end
    }
}

impl OpenDocument {
    /// Führt die Suche weiter: Text anfordern, neue Seiten durchsuchen, ggf. springen.
    pub fn update_search(&mut self, render: &RenderService) {
        let s = &mut self.search;
        let query = s.query.trim().to_string();
        if !s.open || query.is_empty() {
            s.hits.clear();
            s.current = None;
            s.active = None;
            return;
        }
        let n = self.texts.page_count();
        let key = (query.clone(), s.case_sensitive);
        if s.active.as_ref() != Some(&key) {
            s.active = Some(key);
            s.hits.clear();
            s.current = None;
            s.searched = vec![false; n];
            s.searched_count = 0;
            s.jump_to_first = true;
        }
        if s.searched_count == n {
            return;
        }

        for page in 0..n {
            if s.searched[page] {
                continue;
            }
            let Some(text) = self.texts.get(page) else {
                self.texts.ensure(render, page);
                continue;
            };
            s.searched[page] = true;
            s.searched_count += 1;
            let new_hits: Vec<SearchHit> = text
                .find(&query, s.case_sensitive)
                .into_iter()
                .map(|range| SearchHit {
                    page,
                    rects: text.range_rects(range),
                })
                .collect();
            if new_hits.is_empty() {
                continue;
            }
            // Einfügen an der richtigen Stelle; der aktuelle Index verschiebt sich ggf. mit.
            let at = s.hits.partition_point(|h| h.page < page);
            if let Some(cur) = s.current.as_mut()
                && *cur >= at
            {
                *cur += new_hits.len();
            }
            // `splice` ersetzt einen (hier leeren) Bereich durch die neuen Elemente.
            s.hits.splice(at..at, new_hits);
        }

        // Erster Sprung: zum ersten Treffer ab der aktuellen Seite (sobald dieser feststeht).
        if s.jump_to_first && !s.hits.is_empty() {
            let from = self.viewer.current_page;
            let idx = s.hits.partition_point(|h| h.page < from);
            // Erst springen, wenn alle Seiten bis zum Kandidaten durchsucht sind –
            // sonst könnte ein früherer Treffer noch nachkommen.
            let candidate_page = s.hits.get(idx).map_or(n, |h| h.page);
            let ready = (from..candidate_page.min(n)).all(|p| s.searched[p]);
            if ready && (idx < s.hits.len() || s.searched_count == n) {
                s.jump_to_first = false;
                let target = if idx < s.hits.len() { idx } else { 0 };
                // `s` (die Ausleihe von `self.search`) wird hier nicht mehr benutzt,
                // daher darf `goto_hit` jetzt ganz `self` ausleihen.
                self.goto_hit(target);
            }
        }
    }

    pub fn search_step(&mut self, forward: bool) {
        let len = self.search.hits.len();
        if len == 0 {
            return;
        }
        self.search.jump_to_first = false;
        let next = match self.search.current {
            None => 0,
            Some(i) if forward => (i + 1) % len,
            Some(i) => (i + len - 1) % len,
        };
        self.goto_hit(next);
    }

    fn goto_hit(&mut self, idx: usize) {
        self.search.current = Some(idx);
        let hit = &self.search.hits[idx];
        let Ok(info) = self.doc.page(hit.page) else {
            return;
        };
        let bounds = hit.rects.iter().copied().reduce(|a, b| a.union(&b));
        if let Some(b) = bounds {
            let unit = transform::pdf_rect_to_unit(info, b);
            self.viewer.reveal(hit.page, unit);
        }
    }

    /// Hebt alle sichtbaren Treffer hervor, den aktuellen stärker.
    pub fn paint_search_hits(&self, ui: &egui::Ui, view: &ViewOutput) {
        if self.search.hits.is_empty() {
            return;
        }
        let painter = ui.painter_at(view.clip);
        let fill = Color32::from_rgba_unmultiplied(255, 220, 0, 90);
        let current_fill = Color32::from_rgba_unmultiplied(255, 120, 0, 110);
        for &(page, page_rect) in &view.pages {
            let Ok(info) = self.doc.page(page) else {
                continue;
            };
            for i in self.search.hits_on_page(page) {
                let is_current = self.search.current == Some(i);
                for r in &self.search.hits[i].rects {
                    let sr = transform::pdf_rect_to_screen(info, page_rect, *r).expand(1.0);
                    painter.rect_filled(sr, 2.0, if is_current { current_fill } else { fill });
                    if is_current {
                        painter.rect_stroke(
                            sr,
                            2.0,
                            Stroke::new(1.5, Color32::from_rgb(230, 90, 0)),
                            StrokeKind::Outside,
                        );
                    }
                }
            }
        }
    }

    /// Die Suchleiste (unter der Werkzeugleiste).
    pub fn search_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Suchen:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.search.query)
                    .desired_width(220.0)
                    .hint_text("Text im Dokument"),
            );
            if std::mem::take(&mut self.search.focus_field) {
                resp.request_focus();
            }
            // Ein einzeiliges Feld verliert bei Enter den Fokus -> wir holen ihn zurück.
            let (enter, shift) = ui.input(|i| (i.key_pressed(Key::Enter), i.modifiers.shift));
            if resp.lost_focus() && enter {
                self.search_step(!shift);
                resp.request_focus();
            }
            if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                self.search.close();
                return;
            }

            if ui
                .button("‹")
                .on_hover_text("Vorheriger Treffer (Umschalt+Enter)")
                .clicked()
            {
                self.search_step(false);
            }
            if ui
                .button("›")
                .on_hover_text("Nächster Treffer (Enter / F3)")
                .clicked()
            {
                self.search_step(true);
            }
            ui.checkbox(&mut self.search.case_sensitive, "Groß-/Kleinschreibung");

            let s = &self.search;
            let n = self.texts.page_count();
            let status = if s.query.trim().is_empty() {
                String::new()
            } else if s.hits.is_empty() && s.searched_count < n {
                format!("Suche … {}/{n} Seiten", s.searched_count)
            } else if s.hits.is_empty() {
                "Keine Treffer".to_string()
            } else {
                let cur = s.current.map_or("–".to_string(), |c| (c + 1).to_string());
                let more = if s.searched_count < n { "+" } else { "" };
                format!("{cur} von {}{more}", s.hits.len())
            };
            ui.label(status);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("×")
                    .on_hover_text("Suche schließen (Esc)")
                    .clicked()
                {
                    self.search.close();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_on_page_uses_sorted_order() {
        let hit = |page| SearchHit {
            page,
            rects: vec![],
        };
        let s = SearchState {
            hits: vec![hit(0), hit(2), hit(2), hit(5)],
            ..Default::default()
        };
        assert_eq!(s.hits_on_page(2), 1..3);
        assert_eq!(s.hits_on_page(3), 3..3);
    }
}
