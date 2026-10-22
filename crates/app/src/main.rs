//! Einstiegspunkt der FolioCraft-Desktop-App.

mod annot_draw;
mod app;
mod comments;
mod compare;
mod compress_ui;
mod content_edit;
mod export;
mod file_ops;
mod form_draw;
mod form_edit;
mod form_props;
mod forms;
mod image_tool;
mod jobs;
mod menu;
#[cfg(feature = "ocr")]
mod ocr_ui;
mod page_dialogs;
mod pages;
mod redact;
mod search;
mod settings;
mod shortcuts;
mod sign_ui;
mod signatures;
mod text_cache;
mod toolbar;
mod tools;
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

    // Kommandozeile: `foliocraft [datei.pdf] [--vergleichen andere.pdf]`.
    let mut initial_file = None;
    let mut compare_with = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--vergleichen" {
            compare_with = args.next().map(PathBuf::from);
        } else if initial_file.is_none() {
            initial_file = Some(PathBuf::from(arg));
        }
    }

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
        // Die Closure wird einmal beim Start aufgerufen; `move` übergibt die Pfade hinein.
        Box::new(move |cc| Ok(Box::new(app::FolioApp::new(cc, initial_file, compare_with)))),
    )
}
