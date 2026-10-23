//! Formulare ausfüllen: Felder anklicken, tippen, mit Tab weiterspringen.
//!
//! Jede Änderung ist ein `UpdateField`-Command (also rückgängig machbar). Beim Tippen
//! erzeugt die erste Änderung seit dem Fokussieren einen neuen Undo-Schritt, alle weiteren
//! werden mit ihm verschmolzen.

use crate::annot_draw::color32;
use crate::app::OpenDocument;
use crate::form_draw::{self, Style};
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, Key, Modifiers, Pos2, Rect};
use pdftool_core::form::appearance::{self, PADDING};
use pdftool_core::{Batch, Command, FieldId, FieldKind, FieldValue, FormField, UpdateField};

/// Ein Widget eines Felds (Feld + Index in `FormField::widgets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldFocus {
    pub field: FieldId,
    pub widget: usize,
}

/// Zustand der Formular-Bedienung eines Dokuments.
#[derive(Default)]
pub struct FormUi {
    /// Feld mit Tastaturfokus.
    pub focus: Option<FieldFocus>,
    /// Wurde seit dem Fokussieren schon getippt? (steuert das Zusammenführen für Undo)
    typed: bool,
    /// Aufklappliste eines Kombinationsfelds offen – seit welchem Frame.
    dropdown: Option<u64>,
    /// Im nächsten Frame dem Eingabefeld den Fokus geben.
    focus_text: bool,
}

impl FormUi {
    fn set_focus(&mut self, focus: Option<FieldFocus>) {
        if self.focus != focus {
            self.typed = false;
            self.dropdown = None;
        }
        self.focus = focus;
    }
}

/// Kann der Nutzer in dieses Feld tippen?
fn is_typable(f: &FormField) -> bool {
    !f.read_only
        && matches!(
            f.kind,
            FieldKind::Text { .. } | FieldKind::Combo { editable: true }
        )
}

impl OpenDocument {
    /// Oberstes ausfüllbares Widget unter `pos` (auf einer sichtbaren Seite).
    pub(crate) fn field_at(&self, view: &ViewOutput, pos: Pos2) -> Option<FieldFocus> {
        let (page, page_rect) = view.pages.iter().copied().find(|(_, r)| r.contains(pos))?;
        let info = self.doc.page(page).ok()?;
        let p = transform::screen_to_pdf(info, page_rect, pos);
        let mut hit = None;
        for f in self.doc.forms().iter().filter(|f| f.kind.is_fillable()) {
            for (i, w) in f.widgets.iter().enumerate() {
                if w.page == info.object_id && !w.hidden && w.rect.contains(p) {
                    hit = Some(FieldFocus {
                        field: f.id,
                        widget: i,
                    });
                }
            }
        }
        hit
    }

    /// Bildschirmrechteck eines Widgets, falls seine Seite gerade sichtbar ist.
    fn focus_rect(&self, view: &ViewOutput, focus: FieldFocus) -> Option<(Rect, f32)> {
        let w = self
            .doc
            .forms()
            .get(focus.field)?
            .widgets
            .get(focus.widget)?;
        let page = self.doc.page_index_of(w.page)?;
        let (_, page_rect) = view.pages.iter().find(|(i, _)| *i == page)?;
        let info = self.doc.page(page).ok()?;
        Some((
            form_draw::widget_rect(info, *page_rect, w),
            form_draw::scale(info, *page_rect),
        ))
    }

    fn set_field_value(&mut self, id: FieldId, value: FieldValue, merge: bool) {
        let Some(field) = self.doc.forms().get(id) else {
            return;
        };
        if field.value == value {
            return;
        }
        let cmd = UpdateField::value(field, value);
        let result = if merge {
            self.doc.execute_merging(Box::new(cmd.mergeable()))
        } else {
            // Auch beim ersten Tastendruck "zusammenführbar" markieren, damit die folgenden
            // Tastendrücke mit ihm verschmelzen.
            self.doc.execute(Box::new(cmd.mergeable()))
        };
        if let Err(e) = result {
            tracing::error!(error = %e, "Feldwert setzen fehlgeschlagen");
        }
    }

    /// Maus über Feldern: Cursor, Klicks. Gibt `true` zurück, wenn die Maus über einem
    /// Feld ist – dann bekommt das aktive Werkzeug (Textauswahl, Auswählen) nichts davon.
    pub fn handle_form_input(&mut self, ui: &egui::Ui, view: &ViewOutput) -> bool {
        let resp = &view.response;
        let hover = resp.hover_pos().and_then(|p| self.field_at(view, p));
        if let Some(h) = hover
            && let Some(f) = self.doc.forms().get(h.field)
            && !f.read_only
        {
            let icon = if matches!(f.kind, FieldKind::Text { .. }) {
                egui::CursorIcon::Text
            } else {
                egui::CursorIcon::PointingHand
            };
            ui.ctx().set_cursor_icon(icon);
            if !f.tooltip.is_empty() {
                resp.clone().on_hover_text(&f.tooltip);
            }
        }
        if resp.clicked() {
            let pos = resp.interact_pointer_pos();
            match pos.and_then(|p| self.field_at(view, p)) {
                Some(hit) => {
                    let ctrl = ui.input(|i| i.modifiers.command);
                    self.activate(ui.ctx(), view, hit, pos, ctrl);
                }
                None => self.form.set_focus(None),
            }
        }
        hover.is_some()
    }

    /// Klick auf ein Feld (oder Leertaste): je nach Art umschalten, auswählen, Eingabe öffnen.
    fn activate(
        &mut self,
        ctx: &egui::Context,
        view: &ViewOutput,
        hit: FieldFocus,
        pos: Option<Pos2>,
        ctrl: bool,
    ) {
        let was_focused = self.form.focus == Some(hit);
        self.form.set_focus(Some(hit));
        let Some(f) = self.doc.forms().get(hit.field).cloned() else {
            return;
        };
        if f.read_only {
            return;
        }
        let w = &f.widgets[hit.widget];
        match f.kind {
            FieldKind::CheckBox => {
                let value = if f.widget_is_on(w) {
                    None
                } else {
                    w.on_state.clone()
                };
                self.set_field_value(f.id, FieldValue::Toggle(value), false);
            }
            FieldKind::Radio => {
                self.set_field_value(f.id, FieldValue::Toggle(w.on_state.clone()), false);
            }
            FieldKind::Text { .. } => self.form.focus_text = true,
            FieldKind::Combo { editable } => {
                if editable {
                    self.form.focus_text = true;
                }
                self.form.dropdown = if was_focused && self.form.dropdown.is_some() {
                    None
                } else {
                    Some(ctx.cumulative_pass_nr())
                };
            }
            FieldKind::List { multi } => {
                let Some(row) = pos.and_then(|p| self.list_row_at(view, &f, hit, p)) else {
                    return;
                };
                let mut selected = match &f.value {
                    FieldValue::Choice(v) => v.clone(),
                    _ => Vec::new(),
                };
                if multi && ctrl {
                    // Strg+Klick schaltet einzelne Einträge um.
                    if let Some(i) = selected.iter().position(|s| *s == row) {
                        selected.remove(i);
                    } else {
                        selected.push(row);
                    }
                } else {
                    selected = vec![row];
                }
                self.set_field_value(f.id, FieldValue::Choice(selected), false);
            }
            FieldKind::PushButton | FieldKind::Signature => {}
        }
    }

    /// Exportwert der Listenzeile unter `pos`.
    fn list_row_at(
        &self,
        view: &ViewOutput,
        f: &FormField,
        hit: FieldFocus,
        pos: Pos2,
    ) -> Option<String> {
        let w = &f.widgets[hit.widget];
        let page = self.doc.page_index_of(w.page)?;
        let (_, page_rect) = view.pages.iter().find(|(i, _)| *i == page)?;
        let info = self.doc.page(page).ok()?;
        let p = transform::screen_to_pdf(info, *page_rect, pos);
        let local_y = p.y - w.rect.y0;
        let layout = appearance::layout(f, w)?;
        let lh = layout.line_height();
        let line = layout.lines.iter().find(|l| {
            let bottom = l.baseline - layout.font_size * 0.25;
            (bottom..bottom + lh).contains(&local_y)
        })?;
        // Die Zeile kennt nur den Anzeigetext – zurück zum Exportwert.
        f.options
            .iter()
            .find(|o| o.display == line.text)
            .map(|o| o.export.clone())
    }

    /// Tab/Umschalt+Tab, Leertaste, Esc für das fokussierte Feld. Läuft vor dem Eingabefeld,
    /// damit Tab nicht im Text landet.
    pub fn handle_form_keys(&mut self, ctx: &egui::Context, view: &ViewOutput) {
        let Some(focus) = self.form.focus else { return };
        let (next, prev) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::SHIFT, Key::Tab),
            )
        });
        if next || prev {
            self.move_focus(next);
            return;
        }
        let Some(f) = self.doc.forms().get(focus.field) else {
            self.form.set_focus(None);
            return;
        };
        if is_typable(f) {
            return; // Tasten gehören dem Eingabefeld
        }
        // Nur die Art merken (`clone`), damit `self` danach wieder veränderlich ausleihbar ist.
        let kind = f.kind.clone();
        if matches!(kind, FieldKind::CheckBox | FieldKind::Radio)
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space))
        {
            self.activate(ctx, view, focus, None, false);
        }
        if matches!(kind, FieldKind::Combo { .. })
            && ctx.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Space)
                    || i.consume_key(Modifiers::NONE, Key::Enter)
            })
        {
            self.activate(ctx, view, focus, None, false);
        }
    }

    /// Springt zum nächsten/vorigen ausfüllbaren Feld (Reihenfolge der Datei).
    fn move_focus(&mut self, forward: bool) {
        let doc = &self.doc;
        let order: Vec<FieldFocus> = doc
            .forms()
            .iter()
            .filter(|f| f.kind.is_fillable() && !f.read_only)
            .flat_map(|f| {
                f.widgets
                    .iter()
                    .enumerate()
                    .filter(|(_, w)| !w.hidden && doc.page_index_of(w.page).is_some())
                    // Optionsfelder: nur das erste Widget der Gruppe ist ein Tab-Halt.
                    .take(if f.kind == FieldKind::Radio {
                        1
                    } else {
                        usize::MAX
                    })
                    .map(move |(i, _)| FieldFocus {
                        field: f.id,
                        widget: i,
                    })
            })
            .collect();
        if order.is_empty() {
            return;
        }
        let pos = self
            .form
            .focus
            .and_then(|cur| order.iter().position(|f| f.field == cur.field));
        let n = order.len();
        let target = match (pos, forward) {
            (Some(p), true) => order[(p + 1) % n],
            (Some(p), false) => order[(p + n - 1) % n],
            (None, true) => order[0],
            (None, false) => order[n - 1],
        };
        self.form.set_focus(Some(target));
        if let Some(f) = self.doc.forms().get(target.field) {
            self.form.focus_text = is_typable(f);
            // Feld in den sichtbaren Bereich holen.
            let w = &f.widgets[target.widget];
            if let Some(page) = self.doc.page_index_of(w.page)
                && let Ok(info) = self.doc.page(page)
            {
                let unit = transform::pdf_rect_to_unit(info, w.rect);
                self.viewer.reveal(page, unit);
            }
        }
    }

    /// Zeichnet alle Felder der sichtbaren Seiten.
    pub fn paint_forms(&self, ui: &egui::Ui, view: &ViewOutput, highlight: bool) {
        if self.doc.forms().is_empty() {
            return;
        }
        let painter = ui.painter().with_clip_rect(view.clip);
        let editing = self
            .form
            .focus
            .filter(|f| self.doc.forms().get(f.field).is_some_and(is_typable));
        for &(page, page_rect) in &view.pages {
            let Ok(info) = self.doc.page(page) else {
                continue;
            };
            for f in self.doc.forms().iter().filter(|f| f.kind.is_fillable()) {
                for (i, w) in f.widgets.iter().enumerate() {
                    if w.page != info.object_id || w.hidden {
                        continue;
                    }
                    let me = FieldFocus {
                        field: f.id,
                        widget: i,
                    };
                    let style = Style {
                        highlight,
                        skip_text: editing == Some(me),
                        focused: self.form.focus == Some(me) && editing != Some(me),
                    };
                    form_draw::paint(&painter, info, page_rect, f, w, style);
                }
            }
        }
    }

    /// Eingabefeld über dem fokussierten Text-/Kombinationsfeld und die Aufklappliste.
    pub fn show_field_editor(&mut self, ui: &mut egui::Ui, view: &ViewOutput) {
        let Some(focus) = self.form.focus else { return };
        let Some(f) = self.doc.forms().get(focus.field).cloned() else {
            self.form.set_focus(None);
            return;
        };
        let Some((rect, scale)) = self.focus_rect(view, focus) else {
            return; // Seite nicht sichtbar
        };
        if is_typable(&f) {
            self.text_editor(ui, view, &f, focus, rect, scale);
        }
        if matches!(f.kind, FieldKind::Combo { .. }) && self.form.dropdown.is_some() {
            self.dropdown(ui.ctx(), &f, rect);
        }
    }

    fn text_editor(
        &mut self,
        ui: &mut egui::Ui,
        view: &ViewOutput,
        f: &FormField,
        focus: FieldFocus,
        rect: Rect,
        scale: f32,
    ) {
        let w = &f.widgets[focus.widget];
        let (multiline, password, max_len) = match f.kind {
            FieldKind::Text {
                multiline,
                password,
                max_len,
            } => (multiline, password, max_len),
            _ => (false, false, None),
        };
        let mut text = match &f.value {
            FieldValue::Text(t) => t.clone(),
            FieldValue::Choice(v) => v.first().cloned().unwrap_or_default(),
            _ => String::new(),
        };
        // Schriftgröße wie im fertigen Feld (bei "automatisch" passt sie sich beim Tippen an).
        let size = appearance::layout(f, w).map_or(12.0, |l| l.font_size);
        let pad = (PADDING * scale).round() as i8;
        let edit = if multiline {
            egui::TextEdit::multiline(&mut text)
        } else {
            egui::TextEdit::singleline(&mut text).align(egui::Align2::LEFT_CENTER)
        }
        .id(egui::Id::unique((
            "pdftool_form_field",
            f.id.0,
            focus.widget,
        )))
        .font(egui::FontId::proportional((size * scale).max(1.0)))
        .text_color(color32(f.text_color, 1.0))
        .frame(egui::Frame::NONE)
        .margin(egui::Margin::same(pad))
        .password(password)
        .lock_focus(true)
        .char_limit(max_len.map_or(usize::MAX, |m| m as usize))
        .desired_width(rect.width());

        let resp = ui
            .scope(|ui| {
                ui.set_clip_rect(view.clip);
                ui.put(rect, edit)
            })
            .inner;
        if std::mem::take(&mut self.form.focus_text) {
            resp.request_focus();
        }
        if resp.changed() {
            let value = match f.kind {
                FieldKind::Combo { .. } => FieldValue::Choice(if text.is_empty() {
                    Vec::new()
                } else {
                    vec![text]
                }),
                _ => FieldValue::Text(text),
            };
            let merge = self.form.typed;
            self.set_field_value(f.id, value, merge);
            self.form.typed = true;
        }
        if resp.lost_focus()
            && ui.input(|i| i.key_pressed(Key::Escape) || (!multiline && i.key_pressed(Key::Enter)))
        {
            self.form.set_focus(None);
        }
    }

    fn dropdown(&mut self, ctx: &egui::Context, f: &FormField, rect: Rect) {
        let opened = self.form.dropdown.unwrap_or(0);
        let current = match &f.value {
            FieldValue::Choice(v) => v.first().cloned(),
            _ => None,
        };
        let mut chosen = None;
        let area = egui::Area::new(egui::Id::unique("pdftool_form_dropdown"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.left_bottom())
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(rect.width());
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for o in &f.options {
                                let on = current.as_deref() == Some(o.export.as_str());
                                if ui.selectable_label(on, &o.display).clicked() {
                                    chosen = Some(o.export.clone());
                                }
                            }
                        });
                });
            });
        if let Some(export) = chosen {
            self.set_field_value(f.id, FieldValue::Choice(vec![export]), false);
            self.form.dropdown = None;
        } else if ctx.cumulative_pass_nr() > opened
            && (area.response.clicked_elsewhere() || ctx.input(|i| i.key_pressed(Key::Escape)))
        {
            self.form.dropdown = None;
        }
    }

    /// Alle Felder auf ihren Standardwert zurücksetzen (ein Undo-Schritt).
    pub fn reset_form(&mut self) {
        let cmds: Vec<Box<dyn Command>> = self
            .doc
            .forms()
            .iter()
            .filter(|f| f.kind.is_fillable() && !f.read_only && f.value != f.default)
            .map(|f| Box::new(UpdateField::value(f, f.default.clone())) as Box<dyn Command>)
            .collect();
        if !cmds.is_empty() {
            self.execute(Box::new(Batch::new("Formular zurücksetzen", cmds)));
        }
        self.form.set_focus(None);
    }
}
