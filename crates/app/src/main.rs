//! Einstiegspunkt der FolioCraft-Desktop-App.

mod app;
mod shortcuts;
mod toolbar;
mod viewer;

use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // Logging: Standard "info", überschreibbar mit z. B. RUST_LOG=debug
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,foliocraft=debug".into()),
        )
        .init();

    // Optional: erste Kommandozeilen-Argument = zu öffnende Datei.
    let initial_file = std::env::args_os().nth(1).map(PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FolioCraft")
            .with_inner_size([1200.0, 800.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "FolioCraft",
        options,
        // Die Closure wird einmal beim Start aufgerufen; `move` übergibt `initial_file` hinein.
        Box::new(move |cc| Ok(Box::new(app::FolioApp::new(cc, initial_file)))),
    )
}
