//! Einstiegspunkt der PDF-Tool-Desktop-App.

mod a11y_ui;
mod annot_draw;
mod app;
mod bookmarks;
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
mod home;
mod icons;
mod image_tool;
mod jobs;
mod menu;
#[cfg(feature = "ocr")]
mod ocr_ui;
mod page_dialogs;
mod pages;
mod redact;
mod rich_edit;
mod search;
mod security_ui;
mod settings;
mod shell;
mod shortcuts;
mod sign_ui;
mod signatures;
mod style;
mod tabs;
mod tags_ui;
mod text_cache;
mod toolbar;
mod tools;
mod tools_panel;
mod viewer;

use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // Logging: Standard "info", überschreibbar mit z. B. RUST_LOG=debug
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,pdftool=debug".into()),
        )
        .init();

    // Kommandozeile: `pdftool [datei.pdf …] [--vergleichen andere.pdf]` – jede Datei ein Tab.
    let mut initial_files = Vec::new();
    let mut compare_with = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--vergleichen" {
            compare_with = args.next().map(PathBuf::from);
        } else {
            initial_files.push(PathBuf::from(arg));
        }
    }

    migrate_storage();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PDF Tool")
            .with_inner_size([1200.0, 800.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        APP_ID,
        options,
        // Die Closure wird einmal beim Start aufgerufen; `move` übergibt die Pfade hinein.
        Box::new(move |cc| {
            Ok(Box::new(app::PdfToolApp::new(
                cc,
                initial_files,
                compare_with,
            )))
        }),
    )
}

/// Name für eframe (bestimmt auch den Ordner der gespeicherten Einstellungen).
const APP_ID: &str = "PDF Tool";
/// Früherer Name der App.
const LEGACY_APP_ID: &str = "FolioCraft";

/// Übernimmt beim ersten Start nach der Umbenennung die Einstellungen aus dem alten
/// Ordner (zuletzt geöffnet, vertraute Zertifikate …). Fehler sind egal: dann startet die
/// App eben mit Standardwerten.
fn migrate_storage() {
    let (Some(new), Some(old)) = (
        eframe::storage_dir(APP_ID),
        eframe::storage_dir(LEGACY_APP_ID),
    ) else {
        return;
    };
    if new.exists() || !old.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&old) else {
        return;
    };
    if std::fs::create_dir_all(&new).is_err() {
        return;
    }
    for e in entries.flatten() {
        if e.path().is_file() {
            let _ = std::fs::copy(e.path(), new.join(e.file_name()));
        }
    }
}
