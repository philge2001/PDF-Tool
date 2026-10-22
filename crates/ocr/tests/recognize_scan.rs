//! Echte Erkennung: das Bild der "gescannten" Fixture-Seite (von gen_fixtures gezeichnet).

use foliocraft_core::raster::Raster;
use foliocraft_ocr::{Engine, Image};

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
    let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    for expected in ["Rechnung", "4711", "Bestellung.", "für", "Grüßen"] {
        assert!(text.contains(&expected), "{expected:?} fehlt in {text:?}");
    }
    // "Rechnung" steht oben links: x ≈ 72 pt = 150 px, Grundlinie bei 82 pt ≈ 171 px.
    let r = words.iter().find(|w| w.text == "Rechnung").unwrap();
    assert!((140..170).contains(&r.left), "{r:?}");
    assert!(r.top < 171 && r.top + r.height > 150, "{r:?}");
}
