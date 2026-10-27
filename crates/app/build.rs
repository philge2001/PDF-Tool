//! Build-Skript: bettet unter Windows das App-Icon und die Versionsinfo in `pdftool.exe` ein
//! (macOS/Linux bekommen das Icon über den Installer) und setzt den Schalter `has_ocr`.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icons/pdftool.ico");
    // `has_ocr`: Texterkennung vorhanden – auf dem Desktop mit Feature `ocr` (Tesseract),
    // im Browser immer (tesseract.js, siehe `ocr_ui::web`). Im Code dann `#[cfg(has_ocr)]`
    // statt jedes Mal `#[cfg(any(feature = "ocr", target_arch = "wasm32"))]`.
    println!("cargo::rustc-check-cfg=cfg(has_ocr)");
    let ocr = std::env::var_os("CARGO_FEATURE_OCR").is_some();
    let web = std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
    if ocr || web {
        println!("cargo::rustc-cfg=has_ocr");
    }
    #[cfg(windows)]
    windows_resources();
}

/// Ressourcen-Datei (.rc) erzeugen und mit dem Resource-Compiler des Windows-SDK übersetzen.
/// Fehlt der Compiler, gibt es nur eine Warnung – die App baut trotzdem, nur ohne Icon.
#[cfg(windows)]
fn windows_resources() {
    // Build-Skripte laufen auf dem Build-Rechner; maßgeblich ist aber das Zielsystem.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/icons/pdftool.ico")
        .set("ProductName", "PDF Tool")
        .set("FileDescription", "PDF Tool")
        .set("LegalCopyright", "MIT OR Apache-2.0");
    if let Err(e) = res.compile() {
        println!("cargo:warning=Icon nicht eingebettet: {e}");
    }
}
