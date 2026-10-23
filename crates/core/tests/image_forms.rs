//! Bilder in Formular-XObjects erkennen, verschieben, ersetzen, löschen; Einzelbild-PDF.

use pdftool_core::content::Matrix;
use pdftool_core::{Document, NewImage};

fn fixture() -> Document {
    Document::open(format!(
        "{}/../../tests/fixtures/images.pdf",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

#[test]
fn finds_direct_form_and_inline_images() {
    let doc = fixture();
    let imgs = doc.page_images(0).unwrap();
    assert_eq!(imgs.len(), 3, "{imgs:#?}");
    let direct = &imgs[0];
    assert!(!direct.in_form && !direct.inline);
    assert_eq!(direct.pixels, Some((40, 20)));
    let form = &imgs[1];
    assert!(form.in_form);
    assert_eq!(form.pixels, Some((30, 30)));
    // Das Bild füllt das Formular (120 pt), die Beschneidung zählt nicht zur Matrix.
    assert!((form.bounds.x0 - 300.0).abs() < 0.5 && (form.bounds.width() - 120.0).abs() < 0.5);
    assert!(imgs[2].inline);
}

#[test]
fn moves_replaces_and_deletes_form_images() {
    let mut doc = fixture();
    let form = doc.page_images(0).unwrap()[1].clone();
    doc.transform_image(0, &form, Matrix::translate(50.0, -100.0))
        .unwrap();
    let saved = reload(&doc);
    let moved = saved.page_images(0).unwrap()[1].clone();
    assert!(moved.in_form);
    assert!((moved.bounds.x0 - 350.0).abs() < 0.5 && (moved.bounds.y0 - 460.0).abs() < 0.5);

    // Ersetzen: aus dem Formular wird ein normales Bild an derselben Stelle.
    let png = pdftool_core::image_codec::encode_png(
        &[255, 255, 0, 255, 255, 0, 255, 255, 0, 255, 255, 0],
        2,
        2,
        pdftool_core::image_codec::PixelFormat::Rgb,
    )
    .unwrap();
    let mut doc = saved;
    let target = doc.page_images(0).unwrap()[1].clone();
    doc.replace_image(0, &target, NewImage::from_bytes(&png).unwrap())
        .unwrap();
    let saved = reload(&doc);
    let replaced = saved.page_images(0).unwrap()[1].clone();
    assert!(!replaced.in_form);
    assert!((replaced.bounds.center().x - moved.bounds.center().x).abs() < 0.5);

    let mut doc = saved;
    let img = doc.page_images(0).unwrap()[1].clone();
    doc.delete_image(0, &img).unwrap();
    assert_eq!(reload(&doc).page_images(0).unwrap().len(), 2);
}

#[test]
fn image_snapshot_is_a_one_page_pdf_at_pixel_size() {
    let doc = fixture();
    for img in doc.page_images(0).unwrap() {
        let (bytes, (w, h)) = doc.image_snapshot(0, &img).unwrap();
        let single = Document::from_bytes(bytes).unwrap();
        assert_eq!(single.page_count(), 1);
        if let Some((pw, ph)) = img.pixels {
            assert!(
                (w - pw as f32).abs() < 1.0 && (h - ph as f32).abs() < 1.0,
                "{w}×{h}"
            );
        }
        // Nur das Bild, kein Text der Seite.
        assert!(single.page_images(0).unwrap().len() == 1);
    }
}
