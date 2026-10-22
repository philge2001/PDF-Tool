//! Eingabefenster für den Text von Notizen und Freitext (neu oder bestehend).

use crate::app::OpenDocument;
use eframe::egui::{self, Key, Modifiers, Pos2};
use foliocraft_core::{AddAnnotation, AnnotKind, Annotation, UpdateAnnotation};

pub struct TextEditor {
    /// Die Annotation, wie sie nach "OK" aussehen soll (Text wird beim Bestätigen eingesetzt).
    pub annotation: Annotation,
    /// `None` = neue Annotation, `Some` = Stand vor dem Bearbeiten.
    before: Option<Annotation>,
    pub text: String,
    /// Wo das Fenster erscheinen soll (Bildschirmkoordinaten).
    anchor: Pos2,
    focus: bool,
}

impl TextEditor {
    pub fn create(annotation: Annotation, anchor: Pos2) -> Self {
        Self {
            text: annotation.contents.clone(),
            annotation,
            before: None,
            anchor,
            focus: true,
        }
    }

    pub fn edit(annotation: Annotation, anchor: Pos2) -> Self {
        Self {
            text: annotation.contents.clone(),
            before: Some(annotation.clone()),
            annotation,
            anchor,
            focus: true,
        }
    }

    pub fn is_new(&self) -> bool {
        self.before.is_none()
    }
}

enum Outcome {
    Open,
    Confirm,
    Cancel,
}

impl OpenDocument {
    /// Zeigt das Eingabefenster (falls offen) und übernimmt das Ergebnis.
    pub fn show_editor(&mut self, ctx: &egui::Context) {
        let Some(ed) = &mut self.editor else { return };
        let is_freetext = matches!(ed.annotation.kind, AnnotKind::FreeText { .. });
        let title = if is_freetext { "Freitext" } else { "Kommentar" };
        let mut outcome = Outcome::Open;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .default_pos(ed.anchor + egui::vec2(16.0, 8.0))
            .show(ctx, |ui| {
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut ed.text)
                        .desired_rows(4)
                        .desired_width(280.0)
                        .hint_text(if is_freetext {
                            "Text auf der Seite"
                        } else {
                            "Kommentar"
                        }),
                );
                if std::mem::take(&mut ed.focus) {
                    resp.request_focus();
                }
                ui.horizontal(|ui| {
                    if ui.button("OK").on_hover_text("Strg+Enter").clicked() {
                        outcome = Outcome::Confirm;
                    }
                    if ui.button("Abbrechen").on_hover_text("Esc").clicked() {
                        outcome = Outcome::Cancel;
                    }
                });
                if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
                    outcome = Outcome::Confirm;
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    outcome = Outcome::Cancel;
                }
            });

        match outcome {
            Outcome::Open => {}
            Outcome::Cancel => self.editor = None,
            Outcome::Confirm => {
                // `take` nimmt den Editor aus dem Option-Feld heraus (Ownership), danach ist es None.
                let Some(ed) = self.editor.take() else { return };
                let mut a = ed.annotation;
                a.contents = ed.text.trim_end().to_string();
                match ed.before {
                    // Leerer Freitext wäre unsichtbar – dann gar nicht erst anlegen.
                    None if is_freetext && a.contents.is_empty() => {}
                    None => self.execute(Box::new(AddAnnotation::new(a))),
                    Some(before) if before != a => {
                        a.modified = foliocraft_core::date::pdf_date_now();
                        self.execute(Box::new(UpdateAnnotation::new(before, a)));
                    }
                    Some(_) => {}
                }
            }
        }
    }
}
