//! Dauerhafte Einstellungen (zuletzt geöffnete Dateien, Design, Werkzeugstil).
//!
//! eframe speichert sie über das Feature `persistence` im Konfigurationsordner des
//! Betriebssystems (als RON-Datei). `serde` erzeugt per `derive` den Code zum
//! (De-)Serialisieren.

use crate::tools::{Tool, ToolSettings};
use eframe::egui::ThemePreference;
use foliocraft_core::Color;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Schlüssel im eframe-Speicher.
pub const STORAGE_KEY: &str = "foliocraft_settings";

/// Maximale Länge der Liste "Zuletzt geöffnet".
const MAX_RECENT: usize = 10;

/// `#[serde(default)]`: Fehlen Felder (z. B. aus einer älteren Version), werden die
/// Standardwerte genommen statt die ganzen Einstellungen zu verwerfen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub recent_files: Vec<PathBuf>,
    pub theme: ThemePreference,
    pub show_thumbnails: bool,
    pub show_comments: bool,
    pub author: String,
    pub stroke_width: f32,
    pub font_size: f32,
    /// Farbe pro Werkzeug als RGB-Bytes.
    pub tool_colors: Vec<(Tool, [u8; 3])>,
}

impl Default for Settings {
    fn default() -> Self {
        let tools = ToolSettings::default();
        Self {
            recent_files: Vec::new(),
            theme: ThemePreference::System,
            show_thumbnails: true,
            show_comments: true,
            author: tools.author,
            stroke_width: tools.stroke_width,
            font_size: tools.font_size,
            tool_colors: Vec::new(),
        }
    }
}

impl Settings {
    /// Datei vorne in "Zuletzt geöffnet" einreihen (ohne Duplikate, max. 10).
    pub fn add_recent(&mut self, path: &Path) {
        // Absoluter Pfad, damit der Eintrag auch nach einem Wechsel des Arbeitsordners stimmt.
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(MAX_RECENT);
    }

    /// Entfernt Einträge, deren Datei es nicht mehr gibt.
    pub fn prune_missing(&mut self) {
        self.recent_files.retain(|p| p.is_file());
    }

    pub fn tool_settings(&self) -> ToolSettings {
        let mut t = ToolSettings::default();
        t.author = self.author.clone();
        t.stroke_width = self.stroke_width;
        t.font_size = self.font_size;
        for (tool, rgb) in &self.tool_colors {
            t.set_color(*tool, Color::from_rgb8(*rgb));
        }
        t
    }

    pub fn store_tool_settings(&mut self, t: &ToolSettings) {
        self.author = t.author.clone();
        self.stroke_width = t.stroke_width;
        self.font_size = t.font_size;
        self.tool_colors = Tool::ANNOTATION
            .iter()
            .map(|tool| (*tool, t.color(*tool).to_rgb8()))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_are_unique_and_limited() {
        let mut s = Settings::default();
        for i in 0..15 {
            s.add_recent(Path::new(&format!("/nicht/vorhanden/{i}.pdf")));
        }
        s.add_recent(Path::new("/nicht/vorhanden/3.pdf"));
        assert_eq!(s.recent_files.len(), MAX_RECENT);
        assert_eq!(s.recent_files[0], PathBuf::from("/nicht/vorhanden/3.pdf"));
        assert_eq!(
            s.recent_files
                .iter()
                .filter(|p| p.ends_with("3.pdf"))
                .count(),
            1
        );
        s.prune_missing();
        assert!(s.recent_files.is_empty());
    }

    #[test]
    fn tool_settings_roundtrip() {
        let mut t = ToolSettings::default();
        t.set_color(Tool::Ink, Color::from_rgb8([1, 2, 3]));
        t.stroke_width = 7.0;
        let mut s = Settings::default();
        s.store_tool_settings(&t);
        let back = s.tool_settings();
        assert_eq!(back.color(Tool::Ink).to_rgb8(), [1, 2, 3]);
        assert_eq!(back.stroke_width, 7.0);
    }
}
