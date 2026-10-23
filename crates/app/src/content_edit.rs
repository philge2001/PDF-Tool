//! Modus "Inhalt bearbeiten": Absätze bearbeiten/verschieben/in der Breite ändern und neuen
//! Text hinzufügen.
//!
//! Die Absätze kommen aus `core::paragraph` und werden pro Seite zwischengespeichert (gültig,
//! solange sich der Inhaltsstempel der Seite nicht ändert).
//! - Klick wählt einen Absatz (Kasten mit Anfasser rechts),
//! - Ziehen verschiebt den Kasten, Ziehen am Anfasser ändert die Breite (Text bricht neu um),
//! - Doppelklick oder Enter öffnet den Editor über dem Kasten; Strg+Enter oder ein Klick daneben
//!   übernimmt, Esc verwirft.
//!
//! Jede Änderung setzt den Absatz neu (ein Undo-Schritt); danach lädt `sync_structure` den
//! Renderer mit dem neuen Inhalt.

use crate::app::OpenDocument;
use crate::rich_edit::{FontChoices, RichEdit};
use crate::tools::Tool;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use pdftool_core::rich_text::{FontRef, StyledRun, TextStyle};
use pdftool_core::{PageId, ParaFrame, Paragraph, Point};
use std::collections::HashMap;

/// Breite/Höhe des Anfassers am rechten Rand (Bildschirmpunkte).
const HANDLE: Pos2 = pos2(8.0, 18.0);

pub struct ContentEdit {
    /// Stil für neuen Text (setzt die App aus ihren Einstellungen).
    pub new_style: TextStyle,
    /// Absätze pro Seite samt Inhaltsstempel, aus dem sie stammen.
    paragraphs: HashMap<PageId, (u64, Vec<Paragraph>)>,
    /// Ausgewählter Absatz (Seite, Absatz) und sein (evtl. schon geänderter) Kasten.
    pub selected: Option<(usize, Paragraph, ParaFrame)>,
    /// Laufendes Verschieben bzw. Breite ändern.
    pub(crate) drag: Option<ParaDrag>,
    /// Laufende Eingabe.
    pub editing: Option<InlineEdit>,
    /// Fehlermeldung für den Dialog der App.
    pub error: Option<String>,
    /// Bilder pro Seite samt Inhaltsstempel.
    pub(crate) images: HashMap<PageId, (u64, Vec<pdftool_core::PageImage>)>,
    /// Ausgewähltes Bild (Seite, Bild).
    pub selected_image: Option<(usize, pdftool_core::PageImage)>,
    /// Laufendes Verschieben/Skalieren.
    pub(crate) image_drag: Option<crate::image_tool::ImageDrag>,
    /// Bild, das mit dem nächsten Klick platziert wird.
    pub placing_image: Option<pdftool_core::NewImage>,
    /// Bild unter dem letzten Rechtsklick (für das Kontextmenü).
    pub context_image: Option<(usize, pdftool_core::PageImage)>,
    /// Mitte des sichtbaren Bereichs (Seite, PDF-Punkt) – Ziel für eingefügte Bilder.
    pub visible_center: Option<(usize, pdftool_core::Point)>,
    /// Stil des ausgewählten Absatzes für die Formatleiste (Seite, Stempel, erste Op).
    style_cache: Option<((usize, u64, usize), TextStyle, FontChoices)>,
}

impl Default for ContentEdit {
    fn default() -> Self {
        Self {
            new_style: TextStyle::noto(12.0, pdftool_core::Color::BLACK),
            paragraphs: HashMap::new(),
            selected: None,
            drag: None,
            editing: None,
            error: None,
            images: HashMap::new(),
            selected_image: None,
            image_drag: None,
            placing_image: None,
            style_cache: None,
            context_image: None,
            visible_center: None,
        }
    }
}

pub(crate) struct ParaDrag {
    page: usize,
    para: Paragraph,
    /// Kasten zu Beginn des Zugs und aktueller Kasten (Vorschau).
    start_frame: ParaFrame,
    frame: ParaFrame,
    /// Startpunkt im PDF-Raum.
    start: Point,
    width_only: bool,
}

pub struct InlineEdit {
    page: usize,
    /// Bearbeiteter Absatz samt Kasten; `None` = neuer Text an `origin`.
    para: Option<(Paragraph, ParaFrame)>,
    /// Anfang der Grundlinie (PDF-Raum), nur für neuen Text.
    origin: Point,
    /// Formatierter Text und die Läufe beim Öffnen (unverändert = nichts tun).
    rich: RichEdit,
    initial: Vec<StyledRun>,
    /// Schriften für die Auswahlliste.
    choices: FontChoices,
    /// Fokus beim ersten Bild setzen.
    focus: bool,
}

/// Kasten eines Absatzes auf dem Bildschirm.
fn frame_screen(
    d: &OpenDocument,
    view: &ViewOutput,
    page: usize,
    para: &Paragraph,
    frame: &ParaFrame,
) -> Option<Rect> {
    let (_, page_rect) = view.pages.iter().find(|(p, _)| *p == page)?;
    let info = d.doc.page(page).ok()?;
    Some(
        transform::pdf_rect_to_screen(info, *page_rect, para.frame_rect(frame, para.lines))
            .expand(2.0),
    )
}

impl OpenDocument {
    /// Absätze einer Seite (aus dem Zwischenspeicher oder neu berechnet).
    fn paragraphs(&mut self, page: usize) -> Vec<Paragraph> {
        let Ok(info) = self.doc.page(page) else {
            return Vec::new();
        };
        let (id, stamp) = (info.id, self.doc.content_stamp(info.object_id));
        let cache = &mut self.content_edit.paragraphs;
        if cache.get(&id).is_none_or(|(s, _)| *s != stamp) {
            let paras = self.doc.paragraphs(page).unwrap_or_else(|e| {
                tracing::debug!(error = %e, page, "Absätze nicht verfügbar");
                Vec::new()
            });
            cache.insert(id, (stamp, paras));
        }
        cache.get(&id).map(|(_, p)| p.clone()).unwrap_or_default()
    }

    /// Absatz unter dem Mauszeiger (der kleinste, falls sich Kästen überlappen).
    fn paragraph_at(&mut self, view: &ViewOutput, pos: Pos2) -> Option<(usize, Paragraph)> {
        let (page, _) = view.page_at(pos)?;
        self.paragraphs(page)
            .into_iter()
            .filter(|p| {
                frame_screen(self, view, page, p, &p.frame()).is_some_and(|r| r.contains(pos))
            })
            .min_by(|a, b| {
                let area = |r: &pdftool_core::Rect| r.width() * r.height();
                area(&a.bounds).total_cmp(&area(&b.bounds))
            })
            .map(|p| (page, p))
    }

    /// Anfasser am rechten Rand des ausgewählten Kastens.
    fn width_handle(&self, view: &ViewOutput) -> Option<Rect> {
        let (page, para, frame) = self.content_edit.selected.as_ref()?;
        let r = frame_screen(self, view, *page, para, frame)?;
        Some(Rect::from_center_size(r.right_center(), HANDLE.to_vec2()))
    }

    /// Maus im Modus "Inhalt bearbeiten" (Werkzeuge Text bearbeiten/hinzufügen).
    pub fn handle_content_edit(&mut self, ui: &egui::Ui, view: &ViewOutput, tool: Tool) {
        let resp = &view.response;
        if let Some(pos) = resp.hover_pos() {
            let icon = match tool {
                Tool::AddText => egui::CursorIcon::Text,
                _ if self.width_handle(view).is_some_and(|h| h.contains(pos)) => {
                    egui::CursorIcon::ResizeHorizontal
                }
                _ if self.paragraph_at(view, pos).is_some() => egui::CursorIcon::Move,
                _ => egui::CursorIcon::Default,
            };
            ui.ctx().set_cursor_icon(icon);
        }

        if tool == Tool::EditText {
            self.handle_paragraph_drag(ui, view);
            // Enter öffnet den Editor für den ausgewählten Absatz.
            if self.content_edit.editing.is_none()
                && !ui.ctx().egui_wants_keyboard_input()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                self.open_paragraph_editor();
            }
        }

        if !(resp.clicked() || resp.double_clicked()) {
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
                self.content_edit.selected = self.paragraph_at(view, pos).map(|(page, p)| {
                    let frame = p.frame();
                    (page, p, frame)
                });
                if resp.double_clicked() {
                    self.open_paragraph_editor();
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
                let style = self.content_edit.new_style.clone();
                let baseline = pos + vec2(0.0, style.size * 0.8 * zoom);
                self.content_edit.editing = Some(InlineEdit {
                    page,
                    para: None,
                    origin: transform::screen_to_pdf(info, page_rect, baseline),
                    rich: RichEdit::from_runs(&[], style),
                    initial: Vec::new(),
                    choices: self.font_choices(page, &[]),
                    focus: true,
                });
            }
            _ => {}
        }
    }

    /// Kasten ziehen (verschieben) oder am Anfasser die Breite ändern.
    fn handle_paragraph_drag(&mut self, ui: &egui::Ui, view: &ViewOutput) {
        let resp = &view.response;
        if self.content_edit.editing.is_none()
            && resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            let on_handle = self.width_handle(view).is_some_and(|h| h.contains(origin));
            let hit = if on_handle {
                self.content_edit.selected.clone()
            } else {
                self.paragraph_at(view, origin).map(|(page, p)| {
                    let f = p.frame();
                    (page, p, f)
                })
            };
            if let Some((page, para, frame)) = hit
                && let Some((_, page_rect)) = view.pages.iter().find(|(p, _)| *p == page).copied()
                && let Ok(info) = self.doc.page(page)
            {
                self.content_edit.selected = Some((page, para.clone(), frame));
                self.content_edit.drag = Some(ParaDrag {
                    page,
                    para,
                    start_frame: frame,
                    frame,
                    start: transform::screen_to_pdf(info, page_rect, origin),
                    width_only: on_handle,
                });
            }
        }

        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(drag) = &mut self.content_edit.drag
            && let Some((_, page_rect)) = view.pages.iter().find(|(p, _)| *p == drag.page).copied()
            && let Ok(info) = self.doc.page(drag.page)
        {
            let p = transform::screen_to_pdf(info, page_rect, pos);
            let (dx, dy) = (p.x - drag.start.x, p.y - drag.start.y);
            let f = drag.start_frame;
            drag.frame = if drag.width_only {
                // Breite = Abstand vom linken Rand entlang der Schreibrichtung.
                let dir = drag.para.direction;
                let along = (p.x - f.origin.x) * dir.x + (p.y - f.origin.y) * dir.y;
                ParaFrame {
                    width: Some(along.max(2.0 * drag.para.size_on_page)),
                    ..f
                }
            } else {
                ParaFrame {
                    origin: Point::new(f.origin.x + dx, f.origin.y + dy),
                    ..f
                }
            };
        }

        if resp.drag_stopped()
            && let Some(drag) = self.content_edit.drag.take()
            && drag.frame != drag.start_frame
        {
            let text = drag.para.text.clone();
            let result = self
                .doc
                .set_paragraph(drag.page, &drag.para, &text, drag.frame);
            self.after_paragraph_change(drag.page, result, drag.frame);
        }
    }

    pub fn open_paragraph_editor(&mut self) {
        let Some((page, para, frame)) = self.content_edit.selected.clone() else {
            return;
        };
        let runs = match self.doc.paragraph_runs(page, &para) {
            Ok(r) => r,
            Err(e) => {
                self.content_edit.error = Some(e.to_string());
                return;
            }
        };
        let default = runs
            .first()
            .map_or_else(|| self.content_edit.new_style.clone(), |r| r.style.clone());
        self.content_edit.editing = Some(InlineEdit {
            page,
            rich: RichEdit::from_runs(&runs, default),
            choices: self.font_choices(page, &runs),
            initial: runs,
            origin: frame.origin,
            para: Some((para, frame)),
            focus: true,
        });
    }

    /// Schriften für die Auswahlliste: die Originalschriften der Läufe (mit Anzeigenamen).
    fn font_choices(&self, page: usize, runs: &[StyledRun]) -> FontChoices {
        let mut originals: Vec<(FontRef, String)> = Vec::new();
        for r in runs {
            if let FontRef::Original(res) = &r.style.font
                && !originals.iter().any(|(f, _)| f == &r.style.font)
            {
                originals.push((r.style.font.clone(), self.doc.font_display_name(page, res)));
            }
        }
        FontChoices { originals }
    }

    /// Stil und Schriftliste des ausgewählten Absatzes (für die Formatleiste im Panel).
    pub fn selected_paragraph_style(&mut self) -> Option<(TextStyle, FontChoices)> {
        let (page, para, _) = self.content_edit.selected.as_ref()?;
        let key = (*page, para.stamp, *para.shows.first()?);
        if let Some((k, style, choices)) = &self.content_edit.style_cache
            && *k == key
        {
            return Some((style.clone(), choices.clone()));
        }
        let runs = self.doc.paragraph_runs(*page, para).ok()?;
        let style = runs.first()?.style.clone();
        let choices = self.font_choices(*page, &runs);
        self.content_edit.style_cache = Some((key, style.clone(), choices.clone()));
        Some((style, choices))
    }

    /// Formatiert den ganzen ausgewählten Absatz um (ein Undo-Schritt).
    pub fn format_selected_paragraph(&mut self, f: &dyn Fn(&mut TextStyle)) {
        let Some((page, para, frame)) = self.content_edit.selected.clone() else {
            return;
        };
        let result = self.doc.paragraph_runs(page, &para).and_then(|mut runs| {
            for r in &mut runs {
                f(&mut r.style);
            }
            self.doc.set_paragraph_rich(page, &para, &runs, frame)
        });
        self.after_paragraph_change(page, result, frame);
    }

    /// Nach dem Neusetzen: Fehler melden, den Absatz am neuen Ort wieder auswählen
    /// (mit dem gewählten Kasten, nicht der gemessenen Breite).
    fn after_paragraph_change(
        &mut self,
        page: usize,
        result: pdftool_core::Result<()>,
        frame: ParaFrame,
    ) {
        let origin = frame.origin;
        self.content_edit.selected = None;
        if let Err(e) = result {
            self.content_edit.error = Some(e.to_string());
            return;
        }
        self.content_edit.selected = self
            .paragraphs(page)
            .into_iter()
            .min_by(|a, b| {
                a.origin
                    .distance(origin)
                    .total_cmp(&b.origin.distance(origin))
            })
            .filter(|p| p.origin.distance(origin) < 1.0)
            .map(|p| {
                let f = ParaFrame {
                    origin: p.origin,
                    ..frame
                };
                (page, p, f)
            });
    }

    /// Übernimmt die laufende Eingabe (ein Undo-Schritt).
    pub fn commit_content_edit(&mut self) {
        let Some(ed) = self.content_edit.editing.take() else {
            return;
        };
        let runs = ed.rich.runs();
        match ed.para {
            Some(_) if runs == ed.initial => {}
            Some((para, frame)) => {
                let result = self.doc.set_paragraph_rich(ed.page, &para, &runs, frame);
                self.after_paragraph_change(ed.page, result, frame);
            }
            None if ed.rich.text.trim().is_empty() => {}
            None => {
                if let Err(e) = self.doc.add_rich_text(ed.page, ed.origin, &runs, None) {
                    self.content_edit.error = Some(e.to_string());
                }
            }
        }
    }

    /// Umrisse der Absätze, Auswahl mit Anfasser, Vorschau beim Ziehen, Eingabefeld.
    pub fn paint_content_edit(&mut self, ui: &egui::Ui, view: &ViewOutput, tool: Tool) {
        let painter = ui.painter().with_clip_rect(view.clip);
        let accent = Color32::from_rgb(40, 120, 230);
        if tool == Tool::EditText && self.content_edit.editing.is_none() {
            let hovered = view
                .response
                .hover_pos()
                .and_then(|p| self.paragraph_at(view, p));
            for &(page, _) in &view.pages {
                for p in self.paragraphs(page) {
                    let Some(r) = frame_screen(self, view, page, &p, &p.frame()) else {
                        continue;
                    };
                    let is_hovered = hovered
                        .as_ref()
                        .is_some_and(|(hp, h)| *hp == page && *h == p);
                    let stroke = if is_hovered {
                        Stroke::new(1.5, accent)
                    } else {
                        Stroke::new(0.5, accent.gamma_multiply(0.5))
                    };
                    painter.rect_stroke(r, 1.0, stroke, StrokeKind::Outside);
                }
            }
            if let Some((page, para, frame)) = &self.content_edit.selected
                && let Some(r) = frame_screen(self, view, *page, para, frame)
            {
                painter.rect_stroke(r, 1.0, Stroke::new(2.0, accent), StrokeKind::Outside);
                if let Some(h) = self.width_handle(view) {
                    painter.rect_filled(h, 2.0, accent);
                }
            }
            if let Some(drag) = &self.content_edit.drag
                && let Some(r) = frame_screen(self, view, drag.page, &drag.para, &drag.frame)
            {
                painter.rect_filled(r, 1.0, accent.gamma_multiply(0.12));
                painter.rect_stroke(r, 1.0, Stroke::new(1.5, accent), StrokeKind::Inside);
            }
        }
        self.inline_editor(ui, view);
    }

    fn inline_editor(&mut self, ui: &egui::Ui, view: &ViewOutput) {
        let zoom = self.viewer.zoom();
        let Some(ed) = &self.content_edit.editing else {
            return;
        };
        let Some(&(_, page_rect)) = view.pages.iter().find(|(p, _)| *p == ed.page) else {
            return; // Seite nicht sichtbar: Eingabe bleibt offen
        };
        let Ok(info) = self.doc.page(ed.page) else {
            return;
        };
        // Position und Größe: über dem Kasten bzw. am Klickpunkt.
        let (rect, rows) = match &ed.para {
            Some((p, f)) => (
                transform::pdf_rect_to_screen(info, page_rect, p.frame_rect(f, p.lines)),
                p.lines.max(2),
            ),
            None => {
                let size = ed.rich.current().size;
                let base = transform::pdf_to_screen(info, page_rect, ed.origin);
                let top = base - vec2(0.0, size * 0.8 * zoom);
                (Rect::from_min_size(top, vec2(0.0, size * 1.2 * zoom)), 1)
            }
        };
        let width = rect.width().max(560.0);
        let is_para = ed.para.is_some();
        let mut commit = false;
        let mut cancel = false;
        crate::rich_edit::ensure_bold_font(ui.ctx());
        let Some(ed) = &mut self.content_edit.editing else {
            return;
        };
        egui::Area::new(egui::Id::unique("content_inline_edit"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(rect.min.x - 4.0, rect.min.y - 44.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(width.max(560.0));
                    let current = ed.rich.current();
                    if let Some(f) =
                        crate::rich_edit::format_bar(ui, "inline", &current, &ed.choices)
                    {
                        ed.rich.apply(&*f);
                    }
                    let focus = std::mem::take(&mut ed.focus);
                    ed.rich.text_edit(ui, zoom, width, rows, focus);
                    if ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command) {
                        commit = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .button("Übernehmen")
                            .on_hover_text("Strg+Enter")
                            .clicked()
                        {
                            commit = true;
                        }
                        if ui.button("Abbrechen").on_hover_text("Esc").clicked() {
                            cancel = true;
                        }
                        ui.weak(if is_para {
                            "Text markieren und oben formatieren · Enter = neue Zeile"
                        } else {
                            "Text markieren und oben formatieren"
                        });
                    });
                });
            });
        if cancel {
            self.content_edit.editing = None;
        } else if commit {
            self.commit_content_edit();
        }
    }

    /// Blocksatz des ausgewählten Absatzes umschalten (setzt ihn neu).
    pub fn toggle_selected_justify(&mut self) {
        let Some((page, para, mut frame)) = self.content_edit.selected.clone() else {
            return;
        };
        frame.justify = !frame.justify;
        if frame.width.is_none() {
            frame.width = Some(para.width);
        }
        let result = self.doc.set_paragraph(page, &para, &para.text, frame);
        self.after_paragraph_change(page, result, frame);
    }
}
