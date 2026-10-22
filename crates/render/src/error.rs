use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(
        "PDFium-Bibliothek nicht gefunden. Gesucht in: {searched:?}. \
         Siehe README, Abschnitt \"PDFium beschaffen\"."
    )]
    LibraryNotFound { searched: Vec<PathBuf> },
    #[error("PDFium-Fehler: {0}")]
    Pdfium(String),
    #[error("Render-Thread ist beendet")]
    WorkerGone,
}

/// `PdfiumError` enthält nicht immer `Send`-fähige Daten; wir speichern nur den Text.
impl From<pdfium_render::prelude::PdfiumError> for RenderError {
    fn from(e: pdfium_render::prelude::PdfiumError) -> Self {
        RenderError::Pdfium(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, RenderError>;
