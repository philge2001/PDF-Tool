//! Eigenschaften-Panel für das ausgewählte Formularfeld (Modus "Formular bearbeiten").
//!
//! Das Panel arbeitet auf einer Kopie des Felds. Hat sich am Ende etwas geändert, wird
//! daraus ein `UpdateField`. Änderungen aus Textfeldern, Zahlenfeldern und Farbwählern
//! verschmelzen zu einem Undo-Schritt, Häkchen und Knöpfe sind je ein eigener Schritt.

use crate::app::OpenDocument;
use crate::forms::FieldFocus;
use eframe::egui;
use pdftool_core::form::{Align, ChoiceOption};
use pdftool_core::{Color, FieldKind, FieldValue, FormField, UpdateField};

/// Eingaben, die erst übernommen werden, wenn sie gültig sind (Name, Optionsliste).
#[derive(Default)]
pub struct PropInputs {
    name: Option<(pdftool_core::FieldId, String)>,
    options: Option<(pdftool_core::FieldId, String)>,
}

impl OpenDocument {
    pub fn field_properties(&mut self, ui: &mut egui::Ui) {
        ui.heading("Feldeigenschaften");
        let Some(sel) = self.form_edit.selected else {
            ui.label(
                "Kein Feld ausgewählt.\n\nWerkzeug oben wählen und auf der Seite ein Feld \
                 aufziehen, oder mit „Auswählen“ ein vorhandenes Feld anklicken.",
            );
            return;
        };
        let Some(before) = self.doc.forms().get(sel.field).cloned() else {
            return;
        };
        let mut f = before.clone();
        // Wird `true`, sobald eine "fortlaufende" Eingabe (Tippen, Ziehen) die Änderung auslöst.
        let mut merge = false;
        let mut delete = false;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(egui::RichText::new(f.kind.label()).weak());
            ui.add_space(4.0);
            egui::Grid::new("field_props")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    merge |= self.name_row(ui, &before, &mut f);
                    ui.label("Kurzinfo");
                    merge |= ui
                        .text_edit_singleline(&mut f.tooltip)
                        .on_hover_text("Wird als Tooltip angezeigt")
                        .changed();
                    ui.end_row();
                    ui.label("");
                    ui.vertical(|ui| {
                        ui.checkbox(&mut f.required, "Pflichtfeld");
                        ui.checkbox(&mut f.read_only, "Schreibgeschützt");
                    });
                    ui.end_row();
                });

            ui.separator();
            merge |= kind_section(ui, &mut f, sel, &mut self.prop_inputs);
            ui.separator();
            merge |= style_section(ui, &mut f);
            ui.separator();

            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Aktuellen Wert als Standard")
                    .on_hover_text("Wird bei „Formular zurücksetzen“ wiederhergestellt")
                    .clicked()
                {
                    f.default = f.value.clone();
                }
                if ui.button("Feld löschen").on_hover_text("Entf").clicked() {
                    delete = true;
                }
            });
        });

        if delete {
            self.delete_selected_field();
            return;
        }
        // Auswahl anpassen, falls ein Widget (Option) entfernt wurde.
        if sel.widget >= f.widgets.len() {
            self.form_edit.selected = Some(FieldFocus {
                field: f.id,
                widget: f.widgets.len().saturating_sub(1),
            });
        }
        if f != before {
            let cmd = UpdateField::new(before, f);
            if merge {
                if let Err(e) = self.doc.execute_merging(Box::new(cmd.mergeable())) {
                    tracing::error!(error = %e, "Feld ändern fehlgeschlagen");
                }
            } else {
                self.execute(Box::new(cmd));
            }
        }
    }

    /// Name: nur der letzte Teil ist änderbar ("person.name" -> "name"). Ungültige Namen
    /// (leer, mit Punkt, schon vergeben) werden angezeigt, aber nicht übernommen.
    fn name_row(&mut self, ui: &mut egui::Ui, before: &FormField, f: &mut FormField) -> bool {
        ui.label("Name");
        let buf = match &mut self.prop_inputs.name {
            Some((id, text)) if *id == f.id => text,
            slot => {
                *slot = Some((f.id, before.short_name().to_string()));
                &mut slot.as_mut().expect("gerade gesetzt").1
            }
        };
        let resp = ui.text_edit_singleline(buf);
        let changed = resp.changed();
        ui.end_row();
        let prefix = &before.name[..before.name.len() - before.short_name().len()];
        let full = format!("{prefix}{}", buf.trim());
        // Nicht im Fokus und abweichend (z. B. nach Rückgängig oder ungültiger Eingabe):
        // wieder den gültigen Namen anzeigen.
        if !resp.has_focus() && !changed && full != before.name {
            *buf = before.short_name().to_string();
            return false;
        }
        let error = if buf.trim().is_empty() {
            Some("Name darf nicht leer sein")
        } else if buf.contains('.') {
            Some("Kein Punkt im Namen (trennt Ebenen)")
        } else if full != before.name && self.doc.forms().by_name(&full).is_some() {
            Some("Name ist schon vergeben")
        } else {
            None
        };
        if let Some(e) = error {
            ui.label("");
            ui.colored_label(egui::Color32::from_rgb(220, 60, 60), e);
            ui.end_row();
        } else if changed {
            f.name = full;
        }
        changed
    }
}

/// Artspezifische Eigenschaften. Gibt `true` zurück, wenn eine fortlaufende Eingabe etwas änderte.
fn kind_section(
    ui: &mut egui::Ui,
    f: &mut FormField,
    sel: FieldFocus,
    inputs: &mut PropInputs,
) -> bool {
    let mut merge = false;
    match &mut f.kind {
        FieldKind::Text {
            multiline,
            password,
            max_len,
        } => {
            ui.checkbox(multiline, "Mehrzeilig");
            ui.checkbox(password, "Passwort (Sternchen)");
            ui.horizontal(|ui| {
                let mut limited = max_len.is_some();
                if ui.checkbox(&mut limited, "Höchstens").changed() {
                    *max_len = limited.then_some(20);
                }
                if let Some(m) = max_len {
                    merge |= ui.add(egui::DragValue::new(m).range(1..=10_000)).changed();
                }
                ui.label("Zeichen");
            });
        }
        FieldKind::CheckBox => {
            ui.horizontal(|ui| {
                ui.label("Exportwert");
                let w = &mut f.widgets[0];
                let mut on = w.on_state.clone().unwrap_or_default();
                if ui
                    .text_edit_singleline(&mut on)
                    .on_hover_text("Wert, der beim Ankreuzen gespeichert wird")
                    .changed()
                    && !on.trim().is_empty()
                {
                    let was_on = f.value == FieldValue::Toggle(w.on_state.clone());
                    w.on_state = Some(on.trim().to_string());
                    if was_on {
                        f.value = FieldValue::Toggle(w.on_state.clone());
                    }
                    merge = true;
                }
            });
        }
        FieldKind::Radio => merge |= radio_section(ui, f, sel),
        FieldKind::Combo { editable } => {
            ui.checkbox(editable, "Eigene Eingabe erlaubt");
            merge |= options_editor(ui, f, inputs);
        }
        FieldKind::List { multi } => {
            ui.checkbox(multi, "Mehrfachauswahl");
            merge |= options_editor(ui, f, inputs);
        }
        FieldKind::PushButton | FieldKind::Signature => {}
    }
    merge
}

/// Optionen eines Optionsfelds: Exportwert je Widget, Option hinzufügen/entfernen.
fn radio_section(ui: &mut egui::Ui, f: &mut FormField, sel: FieldFocus) -> bool {
    let mut merge = false;
    ui.label("Optionen (Exportwerte)");
    let mut remove = None;
    let count = f.widgets.len();
    for i in 0..count {
        ui.horizontal(|ui| {
            let marker = if i == sel.widget { "•" } else { "  " };
            ui.label(marker);
            let mut on = f.widgets[i].on_state.clone().unwrap_or_default();
            if ui.text_edit_singleline(&mut on).changed() && !on.trim().is_empty() {
                let old = f.widgets[i].on_state.clone();
                f.widgets[i].on_state = Some(on.trim().to_string());
                if f.value == FieldValue::Toggle(old) {
                    f.value = FieldValue::Toggle(f.widgets[i].on_state.clone());
                }
                merge = true;
            }
            if count > 1
                && ui
                    .small_button("✕")
                    .on_hover_text("Option entfernen")
                    .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        let w = f.widgets.remove(i);
        if f.value == FieldValue::Toggle(w.on_state) {
            f.value = FieldValue::Toggle(None);
        }
    }
    let can_add = !f.is_merged();
    if ui
        .add_enabled(can_add, egui::Button::new("Option hinzufügen"))
        .on_hover_text("Legt eine weitere Option rechts neben der ausgewählten an")
        .on_disabled_hover_text("Bei diesem Feld (aus der Datei) nicht möglich")
        .clicked()
    {
        let src = &f.widgets[sel.widget.min(f.widgets.len() - 1)];
        let mut w = src.clone();
        w.object_id = None;
        w.rect = w.rect.translate(w.rect.width() + 30.0, 0.0);
        let n = (1..)
            .map(|i| format!("Option{i}"))
            .find(|s| f.widgets.iter().all(|w| w.on_state.as_deref() != Some(s)))
            .unwrap_or_default();
        w.on_state = Some(n);
        f.widgets.push(w);
    }
    merge
}

/// Optionen von Kombinations-/Listenfeldern: eine pro Zeile.
fn options_editor(ui: &mut egui::Ui, f: &mut FormField, inputs: &mut PropInputs) -> bool {
    ui.label("Einträge (einer pro Zeile)");
    let buf = match &mut inputs.options {
        Some((id, text)) if *id == f.id => text,
        slot => {
            let text = f
                .options
                .iter()
                .map(|o| o.display.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            *slot = Some((f.id, text));
            &mut slot.as_mut().expect("gerade gesetzt").1
        }
    };
    let resp = ui.add(egui::TextEdit::multiline(buf).desired_rows(4));
    let changed = resp.changed();
    let shown: Vec<&str> = buf
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let actual: Vec<&str> = f.options.iter().map(|o| o.display.as_str()).collect();
    if !resp.has_focus() && !changed && shown != actual {
        *buf = actual.join("\n"); // nach Rückgängig o. Ä. neu anzeigen
        return false;
    }
    if changed {
        // Exportwerte bekannter Einträge behalten, neue bekommen ihren Anzeigetext.
        let old = std::mem::take(&mut f.options);
        f.options = buf
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| {
                old.iter()
                    .find(|o| o.display == l)
                    .cloned()
                    .unwrap_or_else(|| ChoiceOption {
                        export: l.to_string(),
                        display: l.to_string(),
                    })
            })
            .collect();
        // Gewählte Werte, die es nicht mehr gibt, fallen weg (außer bei freier Eingabe).
        let editable = matches!(f.kind, FieldKind::Combo { editable: true });
        if let FieldValue::Choice(v) = &mut f.value
            && !editable
        {
            v.retain(|e| f.options.iter().any(|o| &o.export == e));
        }
    }
    changed
}

/// Schrift, Ausrichtung, Rahmen und Hintergrund.
fn style_section(ui: &mut egui::Ui, f: &mut FormField) -> bool {
    let mut merge = false;
    let has_text = matches!(
        f.kind,
        FieldKind::Text { .. } | FieldKind::Combo { .. } | FieldKind::List { .. }
    );
    egui::Grid::new("field_style")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            if has_text {
                ui.label("Schriftgröße");
                merge |= ui
                    .add(
                        egui::DragValue::new(&mut f.font_size)
                            .range(0.0..=72.0)
                            .speed(0.2)
                            .custom_formatter(|v, _| {
                                if v <= 0.0 {
                                    "Auto".into()
                                } else {
                                    format!("{v:.1} pt")
                                }
                            }),
                    )
                    .on_hover_text("0 = automatisch an das Feld anpassen")
                    .changed();
                ui.end_row();
                ui.label("Ausrichtung");
                egui::ComboBox::from_id_salt("field_align")
                    .selected_text(match f.align {
                        Align::Left => "Links",
                        Align::Center => "Mitte",
                        Align::Right => "Rechts",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut f.align, Align::Left, "Links");
                        ui.selectable_value(&mut f.align, Align::Center, "Mitte");
                        ui.selectable_value(&mut f.align, Align::Right, "Rechts");
                    });
                ui.end_row();
            }
            ui.label(if has_text { "Textfarbe" } else { "Symbolfarbe" });
            merge |= color_button(ui, &mut f.text_color);
            ui.end_row();

            // Rahmen/Hintergrund gelten für alle Widgets des Felds.
            let w0 = f.widgets[0].clone();
            let mut border = w0.border;
            let mut background = w0.background;
            let mut width = w0.border_width;
            ui.label("Rahmen");
            merge |= optional_color(ui, &mut border, Color::rgb(0.45, 0.45, 0.45));
            ui.end_row();
            if border.is_some() {
                ui.label("Rahmenbreite");
                merge |= ui
                    .add(
                        egui::DragValue::new(&mut width)
                            .range(0.5..=10.0)
                            .speed(0.1)
                            .suffix(" pt"),
                    )
                    .changed();
                ui.end_row();
            }
            ui.label("Hintergrund");
            merge |= optional_color(ui, &mut background, Color::rgb(0.95, 0.95, 1.0));
            ui.end_row();
            for w in &mut f.widgets {
                w.border = border;
                w.background = background;
                w.border_width = width;
            }
        });
    merge
}

fn color_button(ui: &mut egui::Ui, c: &mut Color) -> bool {
    let mut rgb = c.to_rgb8();
    let changed = ui.color_edit_button_srgb(&mut rgb).changed();
    if changed {
        *c = Color::from_rgb8(rgb);
    }
    changed
}

/// Häkchen "an/aus" plus Farbwähler. Nur der Farbwähler zählt als fortlaufende Eingabe.
fn optional_color(ui: &mut egui::Ui, c: &mut Option<Color>, default: Color) -> bool {
    let mut merge = false;
    ui.horizontal(|ui| {
        let mut on = c.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *c = on.then_some(default);
        }
        if let Some(color) = c {
            merge = color_button(ui, color);
        }
    });
    merge
}
