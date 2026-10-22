//! Unsichtbare Textebene: Wörter → Textebene → speichern → neu laden → Text ist da.

use foliocraft_core::{Document, OcrWord};

fn fixture(name: &str) -> Document {
    let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    Document::open(path).unwrap()
}

fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

fn word(text: &str, left: f32, top: f32, right: f32, bottom: f32) -> OcrWord {
    OcrWord {
        text: text.into(),
        left,
        top,
        right,
        bottom,
    }
}

#[test]
fn layer_is_invisible_searchable_and_replaceable() {
    let mut doc = fixture("scan.pdf");
    assert!(!doc.page_has_text(0).unwrap());
    // "Rechnung" links oben (Anzeige-Einheiten), Breite ~ 0,2 der Seite.
    let words = vec![
        word("Rechnung", 0.12, 0.08, 0.32, 0.105),
        word("4711", 0.5, 0.08, 0.6, 0.105),
    ];
    doc.add_ocr_layers(vec![(0, words.clone())]).unwrap();
    assert!(doc.page_has_text(0).unwrap());

    let saved = reload(&doc);
    let blocks = saved.text_blocks(0).unwrap();
    let rechnung = blocks
        .iter()
        .find(|b| b.text == "Rechnung")
        .expect("Wort da");
    assert_eq!(rechnung.render_mode, 3, "unsichtbar");
    // Breite und Lage passen zum Rechteck (595 × 842 pt).
    let b = rechnung.bounds;
    assert!((b.x0 - 0.12 * 595.0).abs() < 1.0, "{b:?}");
    assert!((b.x1 - 0.32 * 595.0).abs() < 2.0, "{b:?}");
    assert!(
        b.y0 < 842.0 * (1.0 - 0.08) && b.y1 > 842.0 * (1.0 - 0.105),
        "{b:?}"
    );

    // Erneut erkennen: die alte Ebene wird ersetzt, nicht verdoppelt.
    let mut saved = saved;
    saved
        .add_ocr_layers(vec![(0, vec![word("Neu", 0.1, 0.5, 0.2, 0.52)])])
        .unwrap();
    let texts: Vec<String> = reload(&saved)
        .text_blocks(0)
        .unwrap()
        .into_iter()
        .map(|b| b.text)
        .collect();
    assert_eq!(texts, vec!["Neu".to_string()]);

    // Ein Undo-Schritt.
    doc.undo().unwrap();
    assert!(!doc.page_has_text(0).unwrap());
}

#[test]
fn layer_on_rotated_page_runs_left_to_right_in_display() {
    let mut doc = fixture("rotated.pdf");
    let info = doc.pages()[0].clone();
    doc.add_ocr_layers(vec![(0, vec![word("Quer", 0.1, 0.1, 0.3, 0.13)])])
        .unwrap();
    let blocks = reload(&doc).text_blocks(0).unwrap();
    let b = blocks.iter().find(|b| b.text == "Quer").expect("Wort da");
    let angle = b
        .direction
        .y
        .atan2(b.direction.x)
        .to_degrees()
        .rem_euclid(360.0);
    assert!((angle - f32::from(info.rotation)).abs() < 0.5, "{angle}");
    // Anfang des Wortes = linke Kante in der Anzeige.
    let start = info.pdf_to_display_unit(b.origin);
    assert!((start.x - 0.1).abs() < 0.01, "{start:?}");
}

#[test]
fn empty_and_unknown_glyphs_are_skipped() {
    let mut doc = fixture("scan.pdf");
    doc.add_ocr_layers(vec![(0, vec![word("日本", 0.1, 0.1, 0.2, 0.12)])])
        .unwrap();
    assert!(!doc.is_dirty(), "nichts Darstellbares -> keine Änderung");
}
