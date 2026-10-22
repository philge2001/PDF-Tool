//! Werkzeuge: bestimmen, was Maus-Interaktionen im Dokument bewirken.

pub mod text_select;

/// Das aktive Werkzeug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Text markieren und kopieren.
    #[default]
    TextSelect,
    /// Dokument durch Ziehen verschieben.
    Hand,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::TextSelect => "Text auswählen",
            Tool::Hand => "Hand",
        }
    }

    pub const ALL: [Tool; 2] = [Tool::TextSelect, Tool::Hand];
}
