//! Prüft vor dem Paketieren, ob `packaging/dist/` vollständig ist (PDFium + Lizenztexte).
//! Läuft automatisch als `before-packaging-command` von `cargo packager` – ohne diese Prüfung
//! würde ein Installer ohne PDFium entstehen, und die App fände nach der Installation nichts.
//!
//! `cargo run -p pdftool-render --example check_dist`

use pdfium_render::prelude::Pdfium;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    // Zur Übersetzungszeit festgelegt: Ordner dieser Crate → Wurzel des Repositorys.
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/dist");
    // Dateiname für das System, auf dem gebaut wird (pdfium.dll, libpdfium.dylib, libpdfium.so).
    let lib = dist
        .join("lib")
        .join(Pdfium::pdfium_platform_library_name());
    let big_enough = std::fs::metadata(&lib).is_ok_and(|m| m.len() > 1_000_000);
    if !big_enough {
        eprintln!(
            "FEHLER: PDFium fehlt in {}.\n\
             Zuerst ausführen:\n  \
             Windows (PowerShell): .\\packaging\\prepare.ps1\n  \
             macOS/Linux:          bash packaging/prepare.sh mac-arm64|linux-x64",
            lib.display()
        );
        return ExitCode::FAILURE;
    }
    if !dist.join("licenses").is_dir() {
        eprintln!(
            "FEHLER: Lizenztexte fehlen in {}",
            dist.join("licenses").display()
        );
        return ExitCode::FAILURE;
    }
    println!("packaging/dist ist vollständig: {}", lib.display());
    ExitCode::SUCCESS
}
