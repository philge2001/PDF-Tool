//! Modus "Inhalt bearbeiten": Text im Seiteninhalt ändern und neuen Text hinzufügen.
//!
//! Die Textblöcke kommen aus `core::text_edit` und werden pro Seite zwischengespeichert
//! (gültig, solange sich der Inhaltsstempel der Seite nicht ändert). Bearbeitet wird direkt
//! über dem Block in einem Eingabefeld; Enter oder ein Klick daneben übernimmt, Esc verwirft.
//! Danach lädt `sync_structure` den Renderer mit dem neuen Inhalt.

use crate::app::OpenDocument;
use crate::tools::Tool;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, Color32, Rect, Stroke, StrokeKind, pos2, vec2};
use foliocraft_core::{PageId, Point, TextBlock};
use std::collections::HashMap;

pub struct ContentEdit {
    /// Schriftgröße und Farbe für neuen Text (setzt die App aus den Werkzeugeinstellungen).
    pub style: (f32, foliocraft_core::Color),
    /// Textblöcke pro Seite samt Inhaltsstempel, aus dem sie stammen.
    blocks: HashMap<PageId, (u64, Vec<TextBlock>)>,
    /// Laufende Eingabe.
    pub editing: Option<InlineEdit>,
    /// Fehlermeldung für den Dialog der App.
    pub error: Option<String>,
}

impl Default for ContentEdit {
    fn default() -> Self {
        Self {
            style: (12.0, foliocraft_core::Color::BLACK),
            blocks: HashMap::new(),
            editing: None,
            error: None,
        }
    }
}

pub struct InlineEdit {
    page: usize,
    /// Bearbeiteter Block; `None` = neuer Text an `origin`.
    block: Option<TextBlock>,
    /// Anfang der Grundlinie (PDF-Raum), nur für neuen Text.
    origin: Point,
    text: String,
    /// Fokus beim ersten Bild setzen.
    focus: bool,
}

impl OpenDocument {
    /// Textblöcke einer Seite (aus dem Zwischenspeicher oder neu berechnet).
    fn blocks(&mut self, page: usize) -> &[TextBlock] {
        let Ok(info) = self.doc.page(page) else {
            return &[];
        };
        let (id, stamp) = (info.id, self.doc.content_stamp(info.object_id));
        let fresh = self
            .content_edit
            .blocks
            .get(&id)
            .is_some_and(|(s, _)| *s == stamp);
        if !fresh {
            let blocks = self.doc.text_blocks(page).unwrap_or_else(|e| {
                tracing::debug!(error = %e, page, "Textblöcke nicht verfügbar");
                Vec::new()
            });
            self.content_edit.blocks.insert(id, (stamp, blocks));
        }
        self.content_edit
            .blocks
            .get(&id)
            .map_or(&[], |(_, b)| b.as_slice())
    }

    /// Block unter dem Mauszeiger (der kleinste, falls sich Blöcke überlappen).
    fn block_at(&mut self, view: &ViewOutput, pos: egui::Pos2) -> Option<(usize, TextBlock)> {
        let (page, page_rect) = view.page_at(pos)?;
        let info = self.doc.page(page).ok()?.clone();
        let p = transform::screen_to_pdf(&info, page_rect, pos);
        self.blocks(page)
            .iter()
            .filter(|b| b.bounds.expand(1.0).contains(p))
            .min_by(|a, b| {
                let area = |r: &foliocraft_core::Rect| r.width() * r.height();
                area(&a.bounds).total_cmp(&area(&b.bounds))
            })
            .map(|b| (page, b.clone()))
    }

    /// Maus im Modus "Inhalt bearbeiten".
    pub fn handle_content_edit(&mut self, ui: &egui::Ui, view: &ViewOutput, tool: Tool) {
        let resp = &view.response;
        if let Some(pos) = resp.hover_pos() {
            let icon = match tool {
                Tool::AddText => egui::CursorIcon::Text,
                _ if self.block_at(view, pos).is_some() => egui::CursorIcon::Text,
                _ => egui::CursorIcon::Default,
            };
            ui.ctx().set_cursor_icon(icon);
        }
        if !resp.clicked() {
            return;
        }
        let Some(pos) = resp.interact_pointer_pos() else {
            return;
        };
        // Ein Klick neben das Eingabefeld übernimmt die laufende Eingabe.
        if self.content_edit.editing.is_some() {
            self.commit_content_edit();
            return;
        }
        match tool {
            Tool::EditText => {
                if let Some((page, block)) = self.block_at(view, pos) {
                    self.content_edit.editing = Some(InlineEdit {
                        page,
                        text: block.text.clone(),
                        origin: block.origin,
                        block: Some(block),
                        focus: true,
                    });
                }
            }
            Tool::AddText => {
                let Some((page, page_rect)) = view.page_at(pos) else {
                    return;
                };
                let Ok(info) = self.doc.page(page) else {
                    return;
                };
                // Klickpunkt = linke obere Ecke; die Grundlinie liegt eine Oberlänge tiefer
                // (in Anzeigerichtung – so stimmt es auch auf gedrehten Seiten).
                let zoom = self.viewer.zoom();
                let size = self.content_edit_size();
                let baseline = pos + vec2(0.0, size * 0.8 * zoom);
                self.content_edit.editing = Some(InlineEdit {
                    page,
                    block: None,
                    origin: transform::screen_to_pdf(info, page_rect, baseline),
                    text: String::new(),
                    focus: true,
                });
            }
            _ => {}
        }
    }

    fn content_edit_size(&self) -> f32 {
        self.content_edit.style.0
    }

    /// Übernimmt die laufende Eingabe (ein Undo-Schritt).
    pub fn commit_content_edit(&mut self) {
        let Some(ed) = self.content_edit.editing.take() else {
            return;
        };
        let result = match &ed.block {
            Some(block) if block.text == ed.text => Ok(()),
            Some(block) => self.doc.edit_text_block(ed.page, block, &ed.text),
            None if ed.text.trim().is_empty() => Ok(()),
            None => {
                let (size, color) = self.content_edit.style;
                self.doc.add_text(ed.page, ed.origin, &ed.text, size, color)
            }
        };
        if let Err(e) = result {
            self.content_edit.error = Some(e.to_string());
        }
    }

    /// Umrisse der Blöcke auf den sichtbaren Seiten und das Eingabefeld.
    pub fn paint_content_edit(&mut self, ui: &egui::Ui, view: &ViewOutput, tool: Tool) {
        let painter = ui.painter().with_clip_rect(view.clip);
        let hover = view.response.hover_pos();
        let accent = Color32::from_rgb(40, 120, 230);
        if tool == Tool::EditText {
            let hovered = hover.and_then(|p| self.block_at(view, p));
            for &(page, page_rect) in &view.pages {
                let Ok(info) = self.doc.page(page).cloned() else {
                    continue;
                };
                for b in self.blocks(page).to_vec() {
                    let r = transform::pdf_rect_to_screen(&info, page_rect, b.bounds).expand(1.0);
                    let is_hovered = hovered.as_ref().is_some_and(|(p, h)| *p == page && *h == b);
                    let stroke = if is_hovered {
                        Stroke::new(1.5, accent)
                    } else {
                        Stroke::new(0.5, accent.gamma_multiply(0.5))
                    };
                    painter.rect_stroke(r, 1.0, stroke, StrokeKind::Outside);
                }
            }
        }
        self.inline_editor(ui, view);
    }

    fn inline_editor(&mut self, ui: &egui::Ui, view: &ViewOutput) {
        let zoom = self.viewer.zoom();
        let Some(ed) = &mut self.content_edit.editing else {
            return;
        };
        let Some(&(_, page_rect)) = view.pages.iter().find(|(p, _)| *p == ed.page) else {
            return; // Seite nicht sichtbar: Eingabe bleibt offen
        };
        let Ok(info) = self.doc.page(ed.page) else {
            return;
        };
        // Position und Größe: über dem Block bzw. am Klickpunkt.
        let (rect, size) = match &ed.block {
            Some(b) => (
                transform::pdf_rect_to_screen(info, page_rect, b.bounds),
                b.size_on_page,
            ),
            None => {
                let size = self.content_edit.style.0;
                let base = transform::pdf_to_screen(info, page_rect, ed.origin);
                let top = base - vec2(0.0, size * 0.8 * zoom);
                (Rect::from_min_size(top, vec2(0.0, size * 1.2 * zoom)), size)
            }
        };
        let font = egui::FontId::new(
            (size * zoom).clamp(9.0, 48.0),
            egui::FontFamily::Name(crate::app::NOTO_FAMILY.into()),
        );
        let width = rect.width().max(220.0);
        let mut commit = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::unique("content_inline_edit"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(rect.min.x - 4.0, rect.min.y - 4.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let edit = egui::TextEdit::singleline(&mut ed.text)
                        .font(font)
                        .desired_width(width);
                    let resp = ui.add(edit);
                    if std::mem::take(&mut ed.focus) {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Übernehmen").on_hover_text("Enter").clicked() {
                            commit = true;
                        }
                        if ui.button("Abbrechen").on_hover_text("Esc").clicked() {
                            cancel = true;
                        }
                        if ed.block.is_some() {
                            ui.weak("Originalschrift, fehlende Zeichen in Noto Sans");
                        }
                    });
                });
            });
        if cancel {
            self.content_edit.editing = None;
        } else if commit {
            self.commit_content_edit();
        }
    }
}
