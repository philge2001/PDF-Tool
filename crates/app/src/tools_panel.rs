//! Werkzeugbereich links (Phase 10): die Liste "Alle Werkzeuge" und – nach dem Anklicken –
//! das Panel des gewählten Werkzeugs (Unterwerkzeuge, Optionen, Aktionen). Werkzeuge, die nur
//! einen Dialog öffnen (z. B. Komprimieren), tun das direkt aus der Liste.
//!
//! Dazu kommen die schwebenden Schnellwerkzeuge am linken Rand des Dokuments und eine kleine
//! Leiste für die ausgewählte Annotation.

use crate::app::PdfToolApp;
use crate::icons::{self, Icon};
use crate::pages::PageAction;
use crate::shell::{RightPane, floating_frame};
use crate::tools::Tool;
use eframe::egui::{self, RichText, Vec2};

/// Werkzeuggruppen in der Liste "Alle Werkzeuge" (Reihenfolge = Anzeige).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolGroup {
    Comment,
    Edit,
    Export,
    Combine,
    Organize,
    Scan,
    Protect,
    Redact,
    Compress,
    Form,
    Sign,
    Compare,
    Accessibility,
    Archive,
}

impl ToolGroup {
    pub const ALL: [ToolGroup; 14] = [
        ToolGroup::Comment,
        ToolGroup::Edit,
        ToolGroup::Export,
        ToolGroup::Combine,
        ToolGroup::Organize,
        ToolGroup::Scan,
        ToolGroup::Protect,
        ToolGroup::Redact,
        ToolGroup::Compress,
        ToolGroup::Form,
        ToolGroup::Sign,
        ToolGroup::Compare,
        ToolGroup::Accessibility,
        ToolGroup::Archive,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ToolGroup::Comment => "Kommentieren",
            ToolGroup::Edit => "PDF bearbeiten",
            ToolGroup::Export => "PDF exportieren",
            ToolGroup::Combine => "Dateien zusammenführen",
            ToolGroup::Organize => "Seiten organisieren",
            ToolGroup::Scan => "Scannen & Texterkennung",
            ToolGroup::Protect => "PDF schützen",
            ToolGroup::Redact => "Schwärzen",
            ToolGroup::Compress => "PDF komprimieren",
            ToolGroup::Form => "Formular vorbereiten",
            ToolGroup::Sign => "Zertifikat verwenden",
            ToolGroup::Compare => "Dateien vergleichen",
            ToolGroup::Accessibility => "Barrierefreiheit",
            ToolGroup::Archive => "Archivieren (PDF/A)",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            ToolGroup::Comment => Icon::Comment,
            ToolGroup::Edit => Icon::Edit,
            ToolGroup::Export => Icon::Export,
            ToolGroup::Combine => Icon::Combine,
            ToolGroup::Organize => Icon::Organize,
            ToolGroup::Scan => Icon::Scan,
            ToolGroup::Protect => Icon::Lock,
            ToolGroup::Redact => Icon::Redact,
            ToolGroup::Compress => Icon::Compress,
            ToolGroup::Form => Icon::Form,
            ToolGroup::Sign => Icon::Certificate,
            ToolGroup::Compare => Icon::Compare,
            ToolGroup::Accessibility => Icon::Accessibility,
            ToolGroup::Archive => Icon::Archive,
        }
    }

    /// Hat die Gruppe ein eigenes Panel (sonst öffnet sie nur einen Dialog)?
    fn has_panel(self) -> bool {
        matches!(
            self,
            ToolGroup::Comment
                | ToolGroup::Edit
                | ToolGroup::Organize
                | ToolGroup::Redact
                | ToolGroup::Form
                | ToolGroup::Sign
                | ToolGroup::Accessibility
        )
    }

    /// Gehört das Werkzeug zu dieser Gruppe? (Beim Verlassen der Gruppe wird es abgewählt.)
    fn owns(self, tool: Tool) -> bool {
        match self {
            ToolGroup::Comment => Tool::ANNOTATION.contains(&tool),
            ToolGroup::Edit => tool.is_content_tool(),
            ToolGroup::Redact => Tool::REDACT.contains(&tool),
            ToolGroup::Form => tool.is_form_tool(),
            _ => false,
        }
    }
}

/// Icon eines Werkzeugs (Unterwerkzeuge in den Panels, Schnellwerkzeuge).
pub fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::TextSelect => Icon::TextCursor,
        Tool::Select | Tool::FormEdit => Icon::Cursor,
        Tool::Hand => Icon::Hand,
        Tool::Highlight => Icon::Highlight,
        Tool::Underline => Icon::Underline,
        Tool::StrikeOut => Icon::Strike,
        Tool::Note => Icon::Note,
        Tool::FreeText | Tool::AddText => Icon::AddText,
        Tool::Ink => Icon::Draw,
        Tool::Rectangle => Icon::Rectangle,
        Tool::Ellipse => Icon::Ellipse,
        Tool::Arrow => Icon::Arrow,
        Tool::RedactText | Tool::RedactArea => Icon::Redact,
        Tool::FieldText => Icon::AddText,
        Tool::FieldCheckBox | Tool::FieldRadio | Tool::FieldCombo | Tool::FieldList => Icon::Form,
        Tool::EditText => Icon::Edit,
        Tool::EditImage => Icon::Image,
    }
}

impl PdfToolApp {
    /// Öffnet eine Werkzeuggruppe (Panel links) bzw. führt ihre Aktion direkt aus.
    pub(crate) fn open_tool_group(&mut self, ctx: &egui::Context, group: ToolGroup) {
        match group {
            ToolGroup::Export => return self.open_export_dialog(),
            ToolGroup::Combine => return self.open_merge_dialog(),
            ToolGroup::Scan => {
                #[cfg(feature = "ocr")]
                self.open_ocr_dialog();
                #[cfg(not(feature = "ocr"))]
                {
                    self.error = Some(if crate::platform::is_web() {
                        "Texterkennung gibt es in der Browser-Version noch nicht.".into()
                    } else {
                        "Diese Version wurde ohne Texterkennung gebaut.".into()
                    });
                }
                return;
            }
            ToolGroup::Protect => return self.open_security_dialog(),
            ToolGroup::Compress => return self.open_compress_dialog(),
            ToolGroup::Compare => return self.start_compare(ctx),
            ToolGroup::Archive => return self.start_pdfa(ctx),
            _ => {}
        }
        debug_assert!(group.has_panel());
        self.tools_panel = true;
        self.tool_group = Some(group);
        match group {
            ToolGroup::Edit if !self.tool.is_content_tool() => self.tool = Tool::EditText,
            ToolGroup::Form if !self.tool.is_form_tool() => self.tool = Tool::FormEdit,
            ToolGroup::Organize => self.right_pane = Some(RightPane::Thumbnails),
            ToolGroup::Accessibility => self.right_pane = Some(RightPane::Tags),
            ToolGroup::Sign => self.right_pane = Some(RightPane::Signatures),
            ToolGroup::Comment => self.right_pane = Some(RightPane::Comments),
            _ => {}
        }
    }

    /// Zurück zur Liste "Alle Werkzeuge"; Werkzeuge der Gruppe werden abgewählt.
    pub(crate) fn leave_tool_group(&mut self) {
        if let Some(g) = self.tool_group.take()
            && g.owns(self.tool)
        {
            self.tool = Tool::TextSelect;
        }
    }

    /// "Bearbeiten" in der Dokumentleiste: Bearbeiten-Panel an/aus.
    pub(crate) fn toggle_content_mode(&mut self) {
        if self.tool_group == Some(ToolGroup::Edit) && self.tools_panel {
            self.leave_tool_group();
        } else {
            self.tools_panel = true;
            self.tool_group = Some(ToolGroup::Edit);
            if !self.tool.is_content_tool() {
                self.tool = Tool::EditText;
            }
        }
    }

    /// Hält die gewählte Gruppe passend zum aktiven Werkzeug (z. B. nach Esc oder Tastenkürzel).
    pub(crate) fn sync_tool_group(&mut self) {
        let wanted = if self.tool.is_form_tool() {
            Some(ToolGroup::Form)
        } else if self.tool.is_content_tool() {
            Some(ToolGroup::Edit)
        } else {
            None
        };
        if let Some(g) = wanted
            && self.tool_group != Some(g)
        {
            self.tool_group = Some(g);
            self.tools_panel = true;
        }
        if wanted.is_none() && matches!(self.tool_group, Some(ToolGroup::Form | ToolGroup::Edit)) {
            self.tool_group = None;
        }
    }

    /// Linker Bereich.
    pub(crate) fn tools_panel_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        match self.tool_group {
            None => {
                if Self::pane_header(ui, "Alle Werkzeuge") {
                    self.tools_panel = false;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for g in ToolGroup::ALL {
                        if icons::row_button(ui, g.icon(), g.label(), false).clicked() {
                            self.open_tool_group(&ctx, g);
                        }
                    }
                });
            }
            Some(group) => {
                let mut back = false;
                ui.horizontal(|ui| {
                    back = icons::sized_button(ui, Icon::Back, "Alle Werkzeuge", false, 28.0)
                        .clicked();
                    ui.label(RichText::new(group.label()).size(16.0).strong());
                });
                ui.separator();
                if back {
                    self.leave_tool_group();
                    return;
                }
                egui::ScrollArea::vertical().show(ui, |ui| match group {
                    ToolGroup::Comment => self.comment_tools(ui),
                    ToolGroup::Edit => self.edit_tools(ui),
                    ToolGroup::Organize => self.organize_tools(ui),
                    ToolGroup::Redact => self.redact_tools(ui),
                    ToolGroup::Form => self.form_tools(ui),
                    ToolGroup::Sign => self.sign_tools(ui),
                    ToolGroup::Accessibility => self.a11y_tools(ui),
                    _ => {}
                });
            }
        }
    }

    /// Zeile für ein Unterwerkzeug; Klick wählt es (erneuter Klick: zurück zur Textauswahl).
    fn tool_row(&mut self, ui: &mut egui::Ui, tool: Tool) {
        let selected = self.tool == tool;
        if icons::row_button(ui, tool_icon(tool), tool.label(), selected).clicked() {
            let applied = tool.is_text_markup()
                && self
                    .document
                    .as_mut()
                    .is_some_and(|d| d.create_markup_from_selection(tool, &self.tool_settings));
            if !applied {
                self.tool = if selected && !tool.is_form_tool() && !tool.is_content_tool() {
                    Tool::TextSelect
                } else {
                    tool
                };
            }
        }
    }

    fn section(ui: &mut egui::Ui, title: &str) {
        ui.add_space(8.0);
        ui.label(RichText::new(title).weak().size(12.0));
    }

    fn comment_tools(&mut self, ui: &mut egui::Ui) {
        Self::section(ui, "Text markieren");
        for t in [Tool::Highlight, Tool::Underline, Tool::StrikeOut] {
            self.tool_row(ui, t);
        }
        Self::section(ui, "Kommentare");
        for t in [Tool::Note, Tool::FreeText] {
            self.tool_row(ui, t);
        }
        Self::section(ui, "Zeichnen");
        for t in [Tool::Ink, Tool::Rectangle, Tool::Ellipse, Tool::Arrow] {
            self.tool_row(ui, t);
        }
        if Tool::ANNOTATION.contains(&self.tool) {
            Self::section(ui, "Darstellung");
            self.tool_style(ui);
        }
        ui.add_space(8.0);
        ui.weak("Markieren: Text auswählen und Werkzeug anklicken – oder Werkzeug wählen und über den Text ziehen.");
    }

    /// Farbe, Linienbreite, Schriftgröße des aktiven Kommentar-Werkzeugs.
    fn tool_style(&mut self, ui: &mut egui::Ui) {
        let tool = self.tool;
        let s = &mut self.tool_settings;
        ui.horizontal(|ui| {
            ui.label("Farbe");
            let mut rgb = s.color(tool).to_rgb8();
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                s.set_color(tool, pdftool_core::Color::from_rgb8(rgb));
            }
        });
        if tool.uses_stroke_width() {
            ui.horizontal(|ui| {
                ui.label("Linienbreite");
                ui.add(
                    egui::DragValue::new(&mut s.stroke_width)
                        .range(0.5..=20.0)
                        .speed(0.1)
                        .suffix(" pt"),
                );
            });
        }
        if matches!(tool, Tool::FreeText | Tool::AddText) {
            ui.horizontal(|ui| {
                ui.label("Schriftgröße");
                ui.add(
                    egui::DragValue::new(&mut s.font_size)
                        .range(4.0..=144.0)
                        .speed(0.2)
                        .suffix(" pt"),
                );
            });
        }
    }

    fn edit_tools(&mut self, ui: &mut egui::Ui) {
        for t in Tool::CONTENT {
            self.tool_row(ui, t);
        }
        let Some(d) = &mut self.document else { return };
        Self::section(ui, "Optionen");
        match self.tool {
            Tool::AddText => {
                ui.label("Format für neuen Text:");
                let choices = crate::rich_edit::FontChoices {
                    originals: Vec::new(),
                };
                if let Some(f) =
                    crate::rich_edit::format_bar(ui, "new_text", &self.new_text_style, &choices)
                {
                    f(&mut self.new_text_style);
                }
                ui.weak("Auf die Seite klicken, Text eingeben, Strg+Enter. Im Eingabefeld lässt sich markierter Text einzeln formatieren.");
            }
            Tool::EditText => {
                if let Some((_, _, frame)) = d.content_edit.selected.clone() {
                    if let Some((style, choices)) = d.selected_paragraph_style() {
                        ui.label("Ganzen Absatz formatieren:");
                        if let Some(f) =
                            crate::rich_edit::format_bar(ui, "paragraph", &style, &choices)
                        {
                            d.format_selected_paragraph(&*f);
                        }
                        ui.add_space(4.0);
                    }
                    if ui
                        .button("Text ändern…")
                        .on_hover_text("Doppelklick oder Enter")
                        .clicked()
                    {
                        d.open_paragraph_editor();
                    }
                    if ui.selectable_label(frame.justify, "Blocksatz").clicked() {
                        d.toggle_selected_justify();
                    }
                }
                ui.weak("Absatz anklicken · ziehen = verschieben · Anfasser rechts = Breite · Doppelklick = Text ändern und einzelne Wörter formatieren");
            }
            Tool::EditImage => {
                if ui.button("Bild einfügen…").clicked() {
                    crate::toolbar::pick_image(ui.ctx(), crate::platform::Pick::InsertImage);
                }
                if d.content_edit.selected_image.is_some() {
                    if ui.button("Ersetzen…").clicked() {
                        crate::toolbar::pick_image(ui.ctx(), crate::platform::Pick::ReplaceImage);
                    }
                    if ui.button("Löschen").on_hover_text("Entf").clicked() {
                        d.delete_selected_image();
                    }
                }
                ui.weak(if d.content_edit.placing_image.is_some() {
                    "Auf die Seite klicken, um das Bild zu platzieren (Esc bricht ab)"
                } else {
                    "Bild ziehen = verschieben, Anfasser unten rechts = Größe"
                });
            }
            _ => {}
        }
    }

    fn organize_tools(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &mut self.document else { return };
        let target = d.target_pages();
        let n = d.doc.page_count();
        ui.weak(if target.len() == 1 {
            format!(
                "Seite {} (Mehrfachauswahl in den Miniaturen)",
                target[0] + 1
            )
        } else {
            format!("{} ausgewählte Seiten", target.len())
        });
        if icons::row_button(ui, Icon::RotateRight, "Im Uhrzeigersinn drehen", false).clicked() {
            d.apply_page_action(PageAction::Rotate(target.clone(), 1));
        }
        if icons::row_button(ui, Icon::Undo, "Gegen den Uhrzeigersinn drehen", false).clicked() {
            d.apply_page_action(PageAction::Rotate(target.clone(), -1));
        }
        let can_delete = target.len() < n;
        if ui
            .add_enabled_ui(can_delete, |ui| {
                icons::row_button(ui, Icon::Trash, "Seiten löschen", false)
            })
            .inner
            .clicked()
        {
            d.apply_page_action(PageAction::Delete(target));
        }
        Self::section(ui, "Einfügen");
        if icons::row_button(ui, Icon::Insert, "Aus Datei einfügen…", false).clicked() {
            self.start_insert_from_file(ui.ctx());
        }
        if icons::row_button(ui, Icon::Plus, "Leere Seite einfügen", false).clicked() {
            self.insert_blank_page();
        }
        Self::section(ui, "Aufteilen");
        if icons::row_button(ui, Icon::Export, "Seiten extrahieren…", false).clicked() {
            self.open_extract_dialog();
        }
        if icons::row_button(ui, Icon::Split, "Dokument teilen…", false).clicked() {
            self.open_split_dialog();
        }
    }

    fn redact_tools(&mut self, ui: &mut egui::Ui) {
        for t in Tool::REDACT {
            self.tool_row(ui, t);
        }
        let pending = self.document.as_ref().map_or(0, |d| d.pending_redactions());
        ui.weak(format!("{pending} Schwärzung(en) markiert"));
        if ui
            .add_enabled(pending > 0, egui::Button::new("Schwärzungen anwenden…"))
            .clicked()
        {
            self.redact_confirm = true;
        }
        Self::section(ui, "Weitere");
        if icons::row_button(ui, Icon::Search, "Suchtreffer schwärzen…", false).clicked()
            && let Some(d) = &mut self.document
        {
            d.search.open();
        }
        if icons::row_button(ui, Icon::Sanitize, "Dokument bereinigen…", false).clicked() {
            self.open_sanitize_dialog();
        }
    }

    fn form_tools(&mut self, ui: &mut egui::Ui) {
        for t in Tool::FORM {
            self.tool_row(ui, t);
        }
        Self::section(ui, "Ausfüllen");
        ui.checkbox(&mut self.settings.highlight_fields, "Felder hervorheben");
        if ui.button("Formular zurücksetzen").clicked()
            && let Some(d) = &mut self.document
        {
            d.reset_form();
        }
        ui.weak("Feld aufziehen oder anklicken; Eigenschaften stehen rechts. Entf löscht.");
    }

    fn sign_tools(&mut self, ui: &mut egui::Ui) {
        if icons::row_button(ui, Icon::Signature, "Digital signieren…", false).clicked() {
            self.open_sign_dialog();
        }
        if icons::row_button(ui, Icon::Certificate, "Testzertifikat erstellen…", false).clicked()
        {
            self.open_cert_dialog();
        }
        if icons::row_button(ui, Icon::Settings, "Vertrauenswürdige Zertifikate…", false).clicked()
        {
            self.show_settings_window = true;
        }
    }

    fn a11y_tools(&mut self, ui: &mut egui::Ui) {
        if icons::row_button(ui, Icon::Accessibility, "Barrierefreiheit prüfen…", false).clicked()
        {
            self.open_a11y_report();
        }
        if icons::row_button(ui, Icon::Tools, "Automatisch taggen…", false).clicked() {
            self.open_autotag_dialog();
        }
        if icons::row_button(ui, Icon::Thumbnails, "Panel „Tags“", false).clicked() {
            self.right_pane = Some(RightPane::Tags);
        }
    }

    // ---------------------------------------------------------------- schwebend

    /// Schnellwerkzeuge: senkrechte Leiste oben links über dem Dokument.
    pub(crate) fn quick_tools(&mut self, ctx: &egui::Context, viewer: egui::Rect) {
        let dark = ctx.theme() == egui::Theme::Dark;
        egui::Area::new(egui::Id::unique("quick_tools"))
            .order(egui::Order::Middle)
            .fixed_pos(viewer.left_top() + Vec2::new(12.0, 12.0))
            .show(ctx, |ui| {
                floating_frame(dark).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
                    ui.vertical(|ui| {
                        for t in [Tool::TextSelect, Tool::Select, Tool::Hand] {
                            self.quick_button(ui, t);
                        }
                        ui.separator();
                        for t in [Tool::Note, Tool::Highlight, Tool::Ink, Tool::FreeText] {
                            self.quick_button(ui, t);
                        }
                        ui.separator();
                        if icons::button(ui, Icon::More, "Alle Kommentar-Werkzeuge", false)
                            .clicked()
                        {
                            self.open_tool_group(ctx, ToolGroup::Comment);
                        }
                    });
                });
            });
    }

    fn quick_button(&mut self, ui: &mut egui::Ui, tool: Tool) {
        let selected = self.tool == tool;
        let label = match tool {
            Tool::Note => "Kommentar hinzufügen",
            Tool::Ink => "Zeichnen",
            Tool::FreeText => "Text hinzufügen (Kommentar)",
            Tool::Select => "Objekte auswählen",
            t => t.label(),
        };
        if icons::button(ui, tool_icon(tool), label, selected).clicked() {
            let applied = tool.is_text_markup()
                && self
                    .document
                    .as_mut()
                    .is_some_and(|d| d.create_markup_from_selection(tool, &self.tool_settings));
            if !applied {
                self.tool = if selected { Tool::TextSelect } else { tool };
            }
        }
    }

    /// Kleine Leiste oben mittig, solange eine Annotation ausgewählt ist: Farbe, löschen.
    pub(crate) fn selection_bar(&mut self, ctx: &egui::Context, viewer: egui::Rect) {
        let Some(d) = &mut self.document else { return };
        let Some(a) = d
            .selected
            .and_then(|id| d.doc.annotations().get(id))
            .cloned()
        else {
            return;
        };
        let dark = ctx.theme() == egui::Theme::Dark;
        egui::Area::new(egui::Id::unique("selection_bar"))
            .order(egui::Order::Middle)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(viewer.center_top() + Vec2::new(0.0, 12.0))
            .show(ctx, |ui| {
                floating_frame(dark).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(a.kind.label());
                        let mut rgb = a.color.to_rgb8();
                        if ui
                            .color_edit_button_srgb(&mut rgb)
                            .on_hover_text("Farbe")
                            .changed()
                        {
                            d.set_selected_color(pdftool_core::Color::from_rgb8(rgb));
                        }
                        if icons::button(ui, Icon::Trash, "Löschen (Entf)", false).clicked() {
                            d.delete_selected();
                        }
                    });
                });
            });
    }
}
