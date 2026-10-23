//! Erstellen von Annotationen mit den Annotationswerkzeugen.

use super::editor::TextEditor;
use super::{Tool, ToolSettings};
use crate::annot_draw;
use crate::app::OpenDocument;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, CursorIcon, Pos2};
use pdftool_core::{
    AddAnnotation, AnnotKind, Annotation, Batch, Command, Point, Quad, Rect, date::pdf_date_now,
};

/// Ein laufender Mauszug, der eine Annotation erzeugt.
pub enum Gesture {
    Ink {
        page: usize,
        points: Vec<Point>,
    },
    Shape {
        page: usize,
        start: Point,
        end: Point,
    },
}

/// Mindestgröße (pt), ab der ein Aufziehen als Form statt als Klick zählt.
const MIN_DRAG: f32 = 4.0;

impl OpenDocument {
    /// Neue Annotation mit Farbe/Autor aus den Einstellungen (ID wird schon vergeben).
    pub(crate) fn new_annotation(
        &mut self,
        page: usize,
        kind: AnnotKind,
        tool: Tool,
        settings: &ToolSettings,
    ) -> Annotation {
        let mut a = Annotation::new(page, kind, settings.color(tool));
        a.id = self.doc.allocate_annotation_id();
        a.author = settings.author.clone();
        a.modified = pdf_date_now();
        a
    }

    /// Führt ein Command aus; Fehler werden geloggt (sie deuten auf einen Programmfehler hin).
    pub fn execute(&mut self, cmd: Box<dyn Command>) {
        if let Err(e) = self.doc.execute(cmd) {
            tracing::error!(error = %e, "Änderung fehlgeschlagen");
        }
    }

    /// Erzeugt Hervorhebung/Unterstreichung/Durchstreichung aus der aktuellen Textauswahl.
    /// Gibt `true` zurück, wenn etwas erzeugt wurde.
    pub fn create_markup_from_selection(&mut self, tool: Tool, settings: &ToolSettings) -> bool {
        let Some(sel) = self.selection else {
            return false;
        };
        let ranges = sel.page_ranges(|p| self.texts.get(p).map(|t| t.len()));
        let mut cmds: Vec<Box<dyn Command>> = Vec::new();
        for (page, range) in ranges {
            // `cloned()` klont nur den Arc (Zähler +1) – so ist `self` danach nicht mehr
            // ausgeliehen und `new_annotation(&mut self)` unten ist erlaubt.
            let Some(text) = self.texts.get(page).cloned() else {
                continue;
            };
            let quads = text.range_quads(range);
            if quads.is_empty() {
                continue;
            }
            let kind = match tool {
                Tool::Highlight => AnnotKind::Highlight { quads },
                Tool::Underline => AnnotKind::Underline { quads },
                Tool::StrikeOut => AnnotKind::StrikeOut { quads },
                Tool::RedactText => AnnotKind::Redact { quads },
                _ => return false,
            };
            let mut a = self.new_annotation(page, kind, tool, settings);
            // Der markierte Text als Kommentar-Vorgabe hilft später im Kommentar-Panel.
            a.contents = text.text_in(range).replace('\n', " ");
            cmds.push(Box::new(AddAnnotation::new(a)));
        }
        if cmds.is_empty() {
            return false;
        }
        let label = format!("{} hinzufügen", tool.label());
        self.execute(Box::new(Batch::new(label, cmds)));
        self.selection = None;
        true
    }

    /// Mausposition -> (Seite, Bildschirmrechteck der Seite), nur wenn direkt auf einer Seite.
    fn page_under(view: &ViewOutput, pos: Pos2) -> Option<(usize, egui::Rect)> {
        view.pages.iter().copied().find(|(_, r)| r.contains(pos))
    }

    fn to_pdf(&self, view: &ViewOutput, page: usize, pos: Pos2) -> Option<Point> {
        let (_, rect) = view.pages.iter().find(|(p, _)| *p == page)?;
        let info = self.doc.page(page).ok()?;
        // Auf die Seite begrenzen, damit Formen nicht über den Rand ragen.
        Some(transform::screen_to_pdf(info, *rect, rect.clamp(pos)))
    }

    /// Maus-Interaktion für Notiz, Freitext, Freihand und Formen.
    pub fn handle_create_tool(
        &mut self,
        ui: &egui::Ui,
        view: &ViewOutput,
        tool: Tool,
        settings: &ToolSettings,
    ) {
        let resp = &view.response;
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        let origin = ui.input(|i| i.pointer.press_origin());
        let pointer = resp.interact_pointer_pos();

        // --- Klick: Notiz setzen bzw. Freitext mit Standardgröße ---
        if resp.clicked()
            && let Some(pos) = pointer
            && let Some((page, _)) = Self::page_under(view, pos)
            && let Some(p) = self.to_pdf(view, page, pos)
        {
            match tool {
                Tool::Note => {
                    let a = self.new_annotation(page, AnnotKind::Note { pos: p }, tool, settings);
                    self.editor = Some(TextEditor::create(a, pos));
                }
                Tool::FreeText => {
                    let h = settings.font_size * 1.2 * 2.0 + 4.0;
                    let rect = Rect::new(p.x, p.y - h, p.x + 200.0, p.y);
                    let kind = AnnotKind::FreeText {
                        rect,
                        font_size: settings.font_size,
                    };
                    let a = self.new_annotation(page, kind, tool, settings);
                    self.editor = Some(TextEditor::create(a, pos));
                }
                _ => {}
            }
            return;
        }

        // --- Ziehen: Freihand, Formen, Freitext-Rahmen ---
        if resp.drag_started()
            && let Some(start_pos) = origin
            && let Some((page, _)) = Self::page_under(view, start_pos)
            && let Some(start) = self.to_pdf(view, page, start_pos)
        {
            self.gesture = Some(match tool {
                Tool::Ink => Gesture::Ink {
                    page,
                    points: vec![start],
                },
                _ => Gesture::Shape {
                    page,
                    start,
                    end: start,
                },
            });
        }
        if resp.dragged()
            && let Some(pos) = pointer
        {
            let page = match &self.gesture {
                Some(Gesture::Ink { page, .. } | Gesture::Shape { page, .. }) => *page,
                None => return,
            };
            let Some(p) = self.to_pdf(view, page, pos) else {
                return;
            };
            match &mut self.gesture {
                Some(Gesture::Ink { points, .. }) => {
                    // Nur Punkte mit etwas Abstand aufnehmen – hält die Datei klein.
                    if points.last().is_none_or(|last| last.distance(p) > 0.75) {
                        points.push(p);
                    }
                }
                Some(Gesture::Shape { end, .. }) => *end = p,
                None => {}
            }
        }
        if resp.drag_stopped()
            && let Some(g) = self.gesture.take()
            && let Some(a) = self.gesture_to_annotation(&g, tool, settings)
        {
            if tool == Tool::FreeText {
                let pos = pointer.unwrap_or_default();
                self.editor = Some(TextEditor::create(a, pos));
            } else {
                self.execute(Box::new(AddAnnotation::new(a)));
            }
        }
    }

    /// Wandelt einen Mauszug in eine Annotation um (`None`, wenn zu klein).
    fn gesture_to_annotation(
        &mut self,
        g: &Gesture,
        tool: Tool,
        settings: &ToolSettings,
    ) -> Option<Annotation> {
        let width = settings.stroke_width;
        let (page, kind) = match g {
            Gesture::Ink { page, points } => (
                *page,
                AnnotKind::Ink {
                    strokes: vec![points.clone()],
                    width,
                },
            ),
            Gesture::Shape { page, start, end } => {
                if start.distance(*end) < MIN_DRAG {
                    return None;
                }
                let rect = Rect::from_points(*start, *end);
                let kind = match tool {
                    Tool::Rectangle => AnnotKind::Square { rect, width },
                    Tool::Ellipse => AnnotKind::Circle { rect, width },
                    Tool::Arrow => AnnotKind::Line {
                        start: *start,
                        end: *end,
                        width,
                        arrow_end: true,
                    },
                    Tool::FreeText => AnnotKind::FreeText {
                        rect,
                        font_size: settings.font_size,
                    },
                    Tool::RedactArea => AnnotKind::Redact {
                        quads: vec![Quad::from_rect(rect)],
                    },
                    _ => return None,
                };
                (*page, kind)
            }
        };
        Some(self.new_annotation(page, kind, tool, settings))
    }

    /// Live-Vorschau des laufenden Mauszugs.
    pub fn paint_gesture_preview(
        &mut self,
        ui: &egui::Ui,
        view: &ViewOutput,
        tool: Tool,
        settings: &ToolSettings,
    ) {
        let Some(g) = &self.gesture else { return };
        let page = match g {
            Gesture::Ink { page, .. } | Gesture::Shape { page, .. } => *page,
        };
        // Vorschau-Annotation bauen, ohne eine echte ID zu verbrauchen.
        let width = settings.stroke_width;
        let kind = match g {
            Gesture::Ink { points, .. } => AnnotKind::Ink {
                strokes: vec![points.clone()],
                width,
            },
            Gesture::Shape { start, end, .. } => {
                let rect = Rect::from_points(*start, *end);
                match tool {
                    Tool::Rectangle | Tool::FreeText => AnnotKind::Square { rect, width: 1.0 },
                    Tool::RedactArea => AnnotKind::Redact {
                        quads: vec![Quad::from_rect(rect)],
                    },
                    Tool::Ellipse => AnnotKind::Circle { rect, width },
                    Tool::Arrow => AnnotKind::Line {
                        start: *start,
                        end: *end,
                        width,
                        arrow_end: true,
                    },
                    _ => return,
                }
            }
        };
        let preview = Annotation::new(page, kind, settings.color(tool));
        if let (Some((_, rect)), Ok(info)) = (
            view.pages.iter().find(|(p, _)| *p == page),
            self.doc.page(page),
        ) {
            annot_draw::paint(&ui.painter_at(view.clip), info, *rect, &preview);
        }
    }
}
