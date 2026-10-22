use std::path::PathBuf;

/// Fehler der Core-Bibliothek.
///
/// `thiserror` erzeugt aus den `#[error(...)]`-Attributen die `Display`-Implementierung,
/// `#[from]` erzeugt `From`-Konvertierungen, damit `?` fremde Fehler automatisch umwandelt.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("Datei konnte nicht gelesen/geschrieben werden: {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("PDF ist ungültig oder wird nicht unterstützt: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("Seite {0} existiert nicht")]
    PageOutOfRange(usize),
    #[error("Annotation {0} existiert nicht")]
    AnnotationNotFound(String),
    #[error("{0}")]
    Other(String),
}

/// Kurzform, damit Funktionen einfach `Result<T>` schreiben können.
pub type Result<T> = std::result::Result<T, CoreError>;
