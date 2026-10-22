//! Findet und lädt die PDFium-Bibliothek zur Laufzeit.

use crate::error::{RenderError, Result};
use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};

/// Name der Umgebungsvariable, mit der man den Pfad überschreiben kann.
pub const PDFIUM_ENV: &str = "FOLIOCRAFT_PDFIUM_PATH";

/// Kandidaten in der Reihenfolge aus der README. Ein Eintrag kann ein Ordner
/// oder direkt die Bibliotheksdatei sein.
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os(PDFIUM_ENV) {
        out.push(PathBuf::from(p));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        out.push(dir);
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd.join("pdfium").join("lib"));
        out.push(cwd.join("pdfium").join("bin"));
        out.push(cwd.join("pdfium"));
    }
    out
}

/// Bindet PDFium. Darf pro Prozess nur EINMAL erfolgreich aufgerufen werden
/// (PDFium hält globalen Zustand) – deshalb lebt das Ergebnis im Render-Worker.
pub fn bind_pdfium() -> Result<Pdfium> {
    let mut searched = Vec::new();
    for candidate in candidate_paths() {
        let file = if candidate.is_dir() {
            Pdfium::pdfium_platform_library_name_at_path(&candidate)
        } else {
            candidate
        };
        if file.is_file() {
            match Pdfium::bind_to_library(&file) {
                Ok(bindings) => {
                    tracing::info!(path = %file.display(), "PDFium geladen");
                    return Ok(Pdfium::new(bindings));
                }
                Err(e) => tracing::warn!(path = %file.display(), error = %e, "PDFium nicht ladbar"),
            }
        }
        searched.push(file);
    }
    match Pdfium::bind_to_system_library() {
        Ok(bindings) => {
            tracing::info!("PDFium aus Systembibliothek geladen");
            Ok(Pdfium::new(bindings))
        }
        Err(_) => {
            searched.push(PathBuf::from("<Systembibliothek>"));
            Err(RenderError::LibraryNotFound { searched })
        }
    }
}
