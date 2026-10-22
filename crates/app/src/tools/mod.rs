//! Werkzeuge: bestimmen, was Maus-Interaktionen im Dokument bewirken.

pub mod create;
pub mod editor;
pub mod select;
pub mod text_select;

use foliocraft_core::Color;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Das aktive Werkzeug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Tool {
    /// Text markieren und kopieren.
    #[default]
    TextSelect,
    /// Annotationen auswählen und verschieben.
    Select,
    /// Dokument durch Ziehen verschieben.
    Hand,
    Highlight,
    Underline,
    StrikeOut,
    Note,
    FreeText,
    Ink,
    Rectangle,
    Ellipse,
    Arrow,
    /// Formular bearbeiten: Felder auswählen, verschieben, Größe ändern.
    FormEdit,
    FieldText,
    FieldCheckBox,
    FieldRadio,
    FieldCombo,
    FieldList,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::TextSelect => "Text auswählen",
            Tool::Select => "Auswählen",
            Tool::Hand => "Hand",
            Tool::Highlight => "Hervorheben",
            Tool::Underline => "Unterstreichen",
            Tool::StrikeOut => "Durchstreichen",
            Tool::Note => "Notiz",
            Tool::FreeText => "Freitext",
            Tool::Ink => "Freihand",
            Tool::Rectangle => "Rechteck",
            Tool::Ellipse => "Ellipse",
            Tool::Arrow => "Pfeil",
            Tool::FormEdit => "Auswählen",
            Tool::FieldText => "Textfeld",
            Tool::FieldCheckBox => "Kontrollkästchen",
            Tool::FieldRadio => "Optionsfeld",
            Tool::FieldCombo => "Kombinationsfeld",
            Tool::FieldList => "Listenfeld",
        }
    }

    pub const NAVIGATION: [Tool; 3] = [Tool::TextSelect, Tool::Select, Tool::Hand];
    pub const ANNOTATION: [Tool; 9] = [
        Tool::Highlight,
        Tool::Underline,
        Tool::StrikeOut,
        Tool::Note,
        Tool::FreeText,
        Tool::Ink,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Arrow,
    ];

    pub const FORM: [Tool; 6] = [
        Tool::FormEdit,
        Tool::FieldText,
        Tool::FieldCheckBox,
        Tool::FieldRadio,
        Tool::FieldCombo,
        Tool::FieldList,
    ];

    /// Gehört das Werkzeug zum Modus "Formular bearbeiten"?
    pub fn is_form_tool(self) -> bool {
        Self::FORM.contains(&self)
    }

    /// Arbeitet das Werkzeug auf markiertem Text?
    pub fn is_text_markup(self) -> bool {
        matches!(self, Tool::Highlight | Tool::Underline | Tool::StrikeOut)
    }

    pub fn uses_stroke_width(self) -> bool {
        matches!(
            self,
            Tool::Ink | Tool::Rectangle | Tool::Ellipse | Tool::Arrow
        )
    }

    fn default_color(self) -> Color {
        match self {
            Tool::Highlight | Tool::Note => Color::YELLOW,
            Tool::Underline => Color::GREEN,
            Tool::StrikeOut | Tool::Rectangle | Tool::Ellipse => Color::RED,
            Tool::Ink | Tool::Arrow => Color::BLUE,
            _ => Color::BLACK,
        }
    }
}

/// Einstellungen für neue Annotationen.
pub struct ToolSettings {
    colors: HashMap<Tool, Color>,
    pub stroke_width: f32,
    pub font_size: f32,
    /// Autor neuer Annotationen (/T).
    pub author: String,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            colors: HashMap::new(),
            stroke_width: 2.0,
            font_size: 12.0,
            // Benutzername des Betriebssystems als Vorgabe (USER unter Linux/macOS, USERNAME unter Windows).
            author: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_default(),
        }
    }
}

impl ToolSettings {
    pub fn color(&self, tool: Tool) -> Color {
        self.colors
            .get(&tool)
            .copied()
            .unwrap_or(tool.default_color())
    }

    pub fn set_color(&mut self, tool: Tool, color: Color) {
        self.colors.insert(tool, color);
    }
}
