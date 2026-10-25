//! Einstiegspunkt der PDF-Tool-Desktop-App.

// Windows: Release-Builds sind eine reine Fenster-App (kein schwarzes Konsolenfenster beim
// Start aus dem Startmenü). Debug-Builds behalten die Konsole für die Log-Ausgabe.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui;
use pdftool_app::PdfToolApp;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // Logging: Standard "info", überschreibbar mit z. B. RUST_LOG=debug
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,pdftool=debug,pdftool_app=debug".into()),
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
            .with_drag_and_drop(true)
            .with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        APP_ID,
        options,
        // Die Closure wird einmal beim Start aufgerufen; `move` übergibt die Pfade hinein.
        Box::new(move |cc| Ok(Box::new(PdfToolApp::new(cc, initial_files, compare_with)))),
    )
}

/// App-Icon für Fenster und Taskleiste (eigene Zeichnung, Quelle `assets/icons/pdftool.svg`).
fn window_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icons/pdftool-256.png"))
        .unwrap_or_default()
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
