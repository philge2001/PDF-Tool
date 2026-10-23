//! Echte Erkennung: das Bild der "gescannten" Fixture-Seite (von gen_fixtures gezeichnet).

use pdftool_core::raster::Raster;
use pdftool_ocr::{Engine, Image};

#[test]
fn recognizes_scanned_letter() {
    let path = format!(
        "{}/../../tests/fixtures/scan.pdf",
        env!("CARGO_MANIFEST_DIR")
    );
    let pdf = lopdf::Document::load(path).unwrap();
    let stream = pdf
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| s.dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Image"))
        .expect("Bild in scan.pdf");
    let raster = Raster::decode(&pdf, stream).unwrap();
    assert_eq!((raster.comps, raster.bpc), (1, 8));

    let engine = Engine::new("deu").unwrap();
    let words = engine
        .recognize(&Image {
            data: &raster.data,
            width: raster.width as u32,
            height: raster.height as u32,
            bytes_per_pixel: 1,
            dpi: 150,
        })
        .unwrap();
    // Tesseract rechnet je nach Prozessor (SIMD) minimal anders – einzelne Zeichen können
    // abweichen. Deshalb: die meisten Schlüsselwörter müssen da sein, nicht alle exakt.
    let text: String = words
        .iter()
        .map(|w| w.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    let expected = [
        "rechnung",
        "4711",
        "bestellung",
        "betrag",
        "freitag",
        "grüßen",
    ];
    let found = expected.iter().filter(|e| text.contains(*e)).count();
    assert!(found >= 5, "nur {found}/6 erkannt: {text}");
    // "Rechnung" steht oben links: x ≈ 72 pt = 150 px, Grundlinie bei 82 pt ≈ 171 px.
    let r = words
        .iter()
        .find(|w| w.text.to_lowercase().contains("rechn"))
        .expect("Überschrift erkannt");
    assert!((130..180).contains(&r.left), "{r:?}");
    assert!(r.top < 175 && r.top + r.height > 145, "{r:?}");
}
