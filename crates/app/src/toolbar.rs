//! Hilfsfunktionen für Werkzeuge (die Leisten selbst stehen in `shell` und `tools_panel`).

/// Dateiauswahl für PNG/JPEG; die Bytes verarbeitet `handle_picked` (`pick` sagt, wofür).
pub(crate) fn pick_image(ctx: &eframe::egui::Context, pick: crate::platform::Pick) {
    crate::platform::FileDialog::new()
        .add_filter("Bilder", &["png", "jpg", "jpeg"])
        .pick(ctx, pick, false);
}
