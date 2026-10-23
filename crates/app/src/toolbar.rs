//! Hilfsfunktionen für Werkzeuge (die Leisten selbst stehen in `shell` und `tools_panel`).

/// Dateiauswahl für PNG/JPEG; liefert die Bytes (Lesefehler werden protokolliert).
pub(crate) fn pick_image() -> Option<Vec<u8>> {
    let path = rfd::FileDialog::new()
        .add_filter("Bilder", &["png", "jpg", "jpeg"])
        .pick_file()?;
    std::fs::read(&path)
        .inspect_err(|e| tracing::error!(error = %e, path = %path.display(), "Bild lesen"))
        .ok()
}
