//! Textauswahl über eine oder mehrere Seiten, Kopieren in die Zwischenablage.

use crate::app::OpenDocument;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, Color32, CursorIcon};
use foliocraft_core::{PageText, TextRange};

/// Eine Caret-Position im Dokument: Seite + Position zwischen Zeichen.
/// `Ord` wird automatisch abgeleitet: Vergleich erst nach `page`, dann nach `caret` –
/// genau die Lesereihenfolge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPos {
    pub page: usize,
    pub caret: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextSelection {
    /// Wo die Auswahl begonnen hat.
    pub anchor: TextPos,
    /// Wo die Maus gerade ist.
    pub focus: TextPos,
}

impl TextSelection {
    /// Zeichenbereiche pro Seite. `page_len` liefert die Zeichenanzahl einer Seite
    /// (`None`, wenn ihr Text noch nicht geladen ist – die Seite wird dann übersprungen).
    pub fn page_ranges(
        &self,
        page_len: impl Fn(usize) -> Option<usize>,
    ) -> Vec<(usize, TextRange)> {
        let (start, end) = if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        };
        (start.page..=end.page)
            .filter_map(|page| {
                let len = page_len(page)?;
                let from = if page == start.page { start.caret } else { 0 };
                let to = if page == end.page { end.caret } else { len };
                let r = TextRange::new(from.min(len), to.min(len));
                (!r.is_empty()).then_some((page, r))
            })
            .collect()
    }
}

/// Wie weit (in PDF-Punkten) ein Klick neben Text liegen darf, um eine Auswahl zu starten.
const START_DIST: f32 = 12.0;

impl OpenDocument {
    fn page_len(&self, page: usize) -> Option<usize> {
        self.texts.get(page).map(|t| t.len())
    }

    /// Mausposition -> (Seite, Punkt im PDF-Raum, Seitentext), falls der Text geladen ist.
    fn text_under(
        &self,
        view: &ViewOutput,
        pos: egui::Pos2,
    ) -> Option<(usize, foliocraft_core::Point, &PageText)> {
        let (page, page_rect) = view.page_at(pos)?;
        let info = self.doc.page(page).ok()?;
        let text = self.texts.get(page)?;
        Some((page, transform::screen_to_pdf(info, page_rect, pos), text))
    }

    /// Caret unter dem Mauszeiger.
    fn caret_at(&self, view: &ViewOutput, pos: egui::Pos2, max_dist: f32) -> Option<TextPos> {
        let (page, p, text) = self.text_under(view, pos)?;
        let caret = text.caret_at(p, max_dist)?;
        Some(TextPos { page, caret })
    }

    /// Maus-Interaktion des Textauswahl-Werkzeugs.
    pub fn handle_text_select(&mut self, ui: &egui::Ui, view: &ViewOutput) {
        let resp = &view.response;
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());

        // Cursor als Textcursor anzeigen, wenn die Maus über Text steht.
        if let Some(pos) = resp.hover_pos()
            && self.caret_at(view, pos, 1.0).is_some()
        {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }

        if resp.double_clicked() {
            if let Some(pos) = pointer
                && let Some((page, p, text)) = self.text_under(view, pos)
                && let Some(w) = text.char_at(p, START_DIST).and_then(|i| text.word_at(i))
            {
                self.selection = Some(TextSelection {
                    anchor: TextPos {
                        page,
                        caret: w.start,
                    },
                    focus: TextPos { page, caret: w.end },
                });
            }
            return;
        }

        if resp.drag_started() {
            // Nicht die aktuelle Position nehmen: egui erkennt das Ziehen erst nach einigen
            // Pixeln Bewegung. Der Anker ist dort, wo die Maustaste gedrückt wurde.
            let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
            self.selection = origin
                .and_then(|pos| self.caret_at(view, pos, START_DIST))
                .map(|tp| TextSelection {
                    anchor: tp,
                    focus: tp,
                });
        } else if resp.dragged()
            && let Some(pos) = pointer
            && let Some(focus) = self.caret_at(view, pos, f32::INFINITY)
            && let Some(sel) = &mut self.selection
        {
            sel.focus = focus;
            // Beim Ziehen über den Fensterrand automatisch weiterscrollen.
            let margin = 30.0;
            let dy = if pos.y < view.clip.min.y + margin {
                -15.0
            } else if pos.y > view.clip.max.y - margin {
                15.0
            } else {
                0.0
            };
            if dy != 0.0 {
                self.viewer.scroll_by(egui::vec2(0.0, dy));
                ui.ctx().request_repaint();
            }
        } else if resp.clicked() {
            self.selection = None;
        }

        // Rechtsklick-Menü
        let has_selection = self.selected_text().is_some_and(|t| !t.is_empty());
        resp.context_menu(|ui| {
            if ui
                .add_enabled(has_selection, egui::Button::new("Kopieren  (Strg+C)"))
                .clicked()
            {
                self.copy_selection(ui.ctx());
                ui.close();
            }
        });
    }

    /// Ausgewählter Text (Seiten durch Zeilenumbruch getrennt).
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        let parts: Vec<String> = sel
            .page_ranges(|p| self.page_len(p))
            .into_iter()
            .filter_map(|(page, r)| Some(self.texts.get(page)?.text_in(r)))
            .collect();
        Some(parts.join("\n"))
    }

    pub fn copy_selection(&self, ctx: &egui::Context) {
        if let Some(text) = self.selected_text().filter(|t| !t.is_empty()) {
            ctx.copy_text(text);
        }
    }

    pub fn paint_selection(&self, ui: &egui::Ui, view: &ViewOutput) {
        let Some(sel) = self.selection else { return };
        let painter = ui.painter_at(view.clip);
        let fill = Color32::from_rgba_unmultiplied(40, 110, 230, 80);
        for (page, range) in sel.page_ranges(|p| self.page_len(p)) {
            let Some(&(_, page_rect)) = view.pages.iter().find(|(p, _)| *p == page) else {
                continue;
            };
            let (Ok(info), Some(text)) = (self.doc.page(page), self.texts.get(page)) else {
                continue;
            };
            for r in text.range_rects(range) {
                painter.rect_filled(transform::pdf_rect_to_screen(info, page_rect, r), 0.0, fill);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(page: usize, caret: usize) -> TextPos {
        TextPos { page, caret }
    }

    #[test]
    fn ranges_over_multiple_pages() {
        let sel = TextSelection {
            anchor: pos(3, 5),
            focus: pos(1, 2),
        }; // rückwärts gezogen
        let lens = |p: usize| Some([10, 20, 30, 40][p]);
        assert_eq!(
            sel.page_ranges(lens),
            vec![
                (1, TextRange::new(2, 20)),
                (2, TextRange::new(0, 30)),
                (3, TextRange::new(0, 5)),
            ]
        );
    }

    #[test]
    fn empty_selection_has_no_ranges() {
        let sel = TextSelection {
            anchor: pos(0, 4),
            focus: pos(0, 4),
        };
        assert!(sel.page_ranges(|_| Some(10)).is_empty());
    }

    #[test]
    fn pages_without_text_are_skipped() {
        let sel = TextSelection {
            anchor: pos(0, 0),
            focus: pos(2, 3),
        };
        let ranges = sel.page_ranges(|p| if p == 1 { None } else { Some(10) });
        assert_eq!(ranges.len(), 2);
    }
}
