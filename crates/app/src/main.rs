//! Einstiegspunkt der FolioCraft-Desktop-App.

use eframe::egui;

fn main() -> eframe::Result<()> {
    // Logging: Standard "info", überschreibbar mit z. B. RUST_LOG=debug
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,foliocraft=debug".into()),
        )
        .init();

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
        Box::new(|_cc| Ok(Box::new(FolioApp))),
    )
}

struct FolioApp;

impl eframe::App for FolioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| ui.heading("FolioCraft – noch kein Dokument geöffnet"));
        });
    }
}
