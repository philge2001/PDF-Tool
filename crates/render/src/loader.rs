//! Findet und lädt die PDFium-Bibliothek zur Laufzeit.

use crate::error::{RenderError, Result};
use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};

/// Name der Umgebungsvariable, mit der man den Pfad überschreiben kann.
pub const PDFIUM_ENV: &str = "PDFTOOL_PDFIUM_PATH";
/// Früherer Name der Variable (wird weiterhin gelesen).
const LEGACY_PDFIUM_ENV: &str = "FOLIOCRAFT_PDFIUM_PATH";

/// Kandidaten in der Reihenfolge aus der README. Ein Eintrag kann ein Ordner
/// oder direkt die Bibliotheksdatei sein.
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for var in [PDFIUM_ENV, LEGACY_PDFIUM_ENV] {
        if let Some(p) = std::env::var_os(var) {
            out.push(PathBuf::from(p));
        }
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        // Wo die Installer die Bibliothek ablegen (siehe `[package.metadata.packager]`):
        // Windows neben `pdftool.exe`, macOS in `PDF Tool.app/Contents/Resources`,
        // Linux (deb/AppImage) in `usr/lib/pdftool` neben `usr/bin`.
        out.push(dir.clone());
        out.push(dir.join("..").join("Resources"));
        out.push(dir.join("..").join("lib").join("pdftool"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        out.push(cwd.join("pdfium").join("lib"));
        out.push(cwd.join("pdfium").join("bin"));
        out.push(cwd.join("pdfium"));
    }
    out
}

/// Im Browser: an pdfium.wasm binden, das die Webseite vorher geladen und mit
/// `initialize_pdfium_render` angemeldet hat (siehe PDF Webeditor, `web/index.html`).
#[cfg(target_arch = "wasm32")]
pub fn bind_pdfium() -> Result<Pdfium> {
    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|e| RenderError::Pdfium(format!("pdfium.wasm nicht angemeldet: {e}")))
}

/// Bindet PDFium. Darf pro Prozess nur EINMAL erfolgreich aufgerufen werden
/// (PDFium hält globalen Zustand) – deshalb lebt das Ergebnis im Render-Worker.
#[cfg(not(target_arch = "wasm32"))]
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
