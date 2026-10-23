//! Bilder bearbeiten: verschieben, skalieren, löschen, ersetzen, einfügen → speichern → neu laden.

use pdftool_core::content::Matrix;
use pdftool_core::raster::Raster;
use pdftool_core::{Document, NewImage, PageImage, Point, Rect};

/// 8×4 Pixel, rot (mit ImageMagick erzeugt: `convert -size 8x4 xc:red -strip red.jpg`).
const RED_JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x03, 0x02, 0x02, 0x03, 0x02, 0x02, 0x03,
    0x03, 0x03, 0x03, 0x04, 0x03, 0x03, 0x04, 0x05, 0x08, 0x05, 0x05, 0x04, 0x04, 0x05, 0x0A, 0x07,
    0x07, 0x06, 0x08, 0x0C, 0x0A, 0x0C, 0x0C, 0x0B, 0x0A, 0x0B, 0x0B, 0x0D, 0x0E, 0x12, 0x10, 0x0D,
    0x0E, 0x11, 0x0E, 0x0B, 0x0B, 0x10, 0x16, 0x10, 0x11, 0x13, 0x14, 0x15, 0x15, 0x15, 0x0C, 0x0F,
    0x17, 0x18, 0x16, 0x14, 0x18, 0x12, 0x14, 0x15, 0x14, 0xFF, 0xDB, 0x00, 0x43, 0x01, 0x03, 0x04,
    0x04, 0x05, 0x04, 0x05, 0x09, 0x05, 0x05, 0x09, 0x14, 0x0D, 0x0B, 0x0D, 0x14, 0x14, 0x14, 0x14,
    0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14,
    0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14,
    0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0x14, 0xFF, 0xC0,
    0x00, 0x11, 0x08, 0x00, 0x04, 0x00, 0x08, 0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11,
    0x01, 0xFF, 0xC4, 0x00, 0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0xFF, 0xC4, 0x00, 0x14, 0x10, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xC4, 0x00,
    0x15, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x07, 0x09, 0xFF, 0xC4, 0x00, 0x14, 0x11, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xDA, 0x00, 0x0C, 0x03, 0x01,
    0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00, 0x3A, 0x03, 0x15, 0x4D, 0xFF, 0xD9,
];

fn fixture(name: &str) -> Document {
    let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    Document::open(path).unwrap()
}

fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

/// PNG mit 4×2 Pixeln: links grün deckend, rechts durchsichtig.
fn green_png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 4, 2);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        let mut data = Vec::new();
        for _y in 0..2 {
            for x in 0..4 {
                data.extend_from_slice(&[0, 200, 0, if x < 2 { 255 } else { 0 }]);
            }
        }
        w.write_image_data(&data).unwrap();
    }
    out
}

fn close(a: Rect, b: Rect) -> bool {
    (a.x0 - b.x0).abs() < 0.01
        && (a.y0 - b.y0).abs() < 0.01
        && (a.x1 - b.x1).abs() < 0.01
        && (a.y1 - b.y1).abs() < 0.01
}

fn xobject_image(doc: &Document) -> PageImage {
    doc.page_images(0)
        .unwrap()
        .into_iter()
        .find(|i| !i.inline)
        .unwrap()
}

#[test]
fn lists_images_of_page() {
    let doc = fixture("content_mix.pdf");
    let images = doc.page_images(0).unwrap();
    assert_eq!(images.len(), 2, "XObject und Inline-Bild: {images:?}");
    assert!(close(
        xobject_image(&doc).bounds,
        Rect::new(350.0, 480.0, 450.0, 540.0)
    ));
    assert!(
        images
            .iter()
            .any(|i| i.inline && close(i.bounds, Rect::new(480.0, 480.0, 520.0, 520.0)))
    );
}

#[test]
fn move_and_scale_roundtrip() {
    let mut doc = fixture("content_mix.pdf");
    let img = xobject_image(&doc);
    doc.transform_image(0, &img, Matrix::translate(10.0, 20.0))
        .unwrap();
    let ops_after_first = doc.page_content(0).unwrap().content.len();
    // Nochmals verschieben: Die vorhandene q/cm/Q-Klammer wird angepasst, nicht verschachtelt.
    let img = xobject_image(&doc);
    doc.transform_image(0, &img, Matrix::translate(-5.0, 0.0))
        .unwrap();
    let grown = doc.page_content(0).unwrap().content.len() as i64 - ops_after_first as i64;
    assert!(
        grown.abs() < 20,
        "Inhalt wächst beim erneuten Verschieben: {grown}"
    );

    // Inline-Bild verdoppeln (linke untere Ecke bleibt).
    let inline = doc
        .page_images(0)
        .unwrap()
        .into_iter()
        .find(|i| i.inline)
        .unwrap();
    let anchor = Matrix::translate(-480.0, -480.0)
        .then(&Matrix([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]))
        .then(&Matrix::translate(480.0, 480.0));
    doc.transform_image(0, &inline, anchor).unwrap();

    let saved = reload(&doc);
    let images = saved.page_images(0).unwrap();
    assert!(close(
        xobject_image(&saved).bounds,
        Rect::new(355.0, 500.0, 455.0, 560.0)
    ));
    assert!(
        images
            .iter()
            .any(|i| i.inline && close(i.bounds, Rect::new(480.0, 480.0, 560.0, 560.0)))
    );
    // Text blieb unberührt.
    let text: String = saved
        .text_blocks(0)
        .unwrap()
        .iter()
        .map(|b| b.text.clone())
        .collect();
    assert!(text.contains("Geheim: 4711-0815"));

    // Undo stellt den Ausgangszustand wieder her.
    let mut doc = doc;
    doc.undo().unwrap();
    doc.undo().unwrap();
    doc.undo().unwrap();
    assert!(close(
        xobject_image(&reload(&doc)).bounds,
        Rect::new(350.0, 480.0, 450.0, 540.0)
    ));
}

#[test]
fn delete_image() {
    let mut doc = fixture("content_mix.pdf");
    let img = xobject_image(&doc);
    doc.delete_image(0, &img).unwrap();
    let saved = reload(&doc);
    let images = saved.page_images(0).unwrap();
    assert_eq!(images.len(), 1);
    assert!(images[0].inline);
}

#[test]
fn replace_with_png_keeps_aspect_and_alpha() {
    let mut doc = fixture("content_mix.pdf");
    let img = xobject_image(&doc);
    let new = NewImage::from_bytes(&green_png()).unwrap();
    assert!(new.smask.is_some(), "halb durchsichtig -> Maske");
    doc.replace_image(0, &img, new).unwrap();

    let saved = reload(&doc);
    let img = xobject_image(&saved);
    // 2:1 in 100×60 -> 100×50, senkrecht zentriert.
    assert!(
        close(img.bounds, Rect::new(350.0, 485.0, 450.0, 535.0)),
        "{:?}",
        img.bounds
    );

    // Pixel des neuen Bilds prüfen (über die Ressourcen der gespeicherten Seite).
    let page = saved.pages()[0].object_id;
    let res = pdftool_core::content::page_resources(saved.pdf(), page).unwrap();
    let xobjects = saved
        .pdf()
        .dereference(res.get(b"XObject").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap();
    let id = xobjects.get(b"FcImg1").unwrap().as_reference().unwrap();
    let stream = saved.pdf().get_object(id).unwrap().as_stream().unwrap();
    let raster = Raster::decode(saved.pdf(), stream).unwrap();
    assert_eq!((raster.width, raster.height, raster.comps), (4, 2, 3));
    assert_eq!(&raster.data[..3], &[0, 200, 0]);
    let mask_id = stream.dict.get(b"SMask").unwrap().as_reference().unwrap();
    let mask = Raster::decode(
        saved.pdf(),
        saved
            .pdf()
            .get_object(mask_id)
            .unwrap()
            .as_stream()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(mask.data, vec![255, 255, 0, 0, 255, 255, 0, 0]);
}

#[test]
fn insert_jpeg_on_rotated_page_and_save_twice() {
    let mut doc = fixture("rotated.pdf");
    let rotation = doc.pages()[0].rotation;
    let new = NewImage::from_bytes(RED_JPEG).unwrap();
    assert_eq!((new.width, new.height), (8, 4));
    doc.insert_image(0, new, Point::new(200.0, 300.0), 80.0)
        .unwrap();

    let dir = std::env::temp_dir().join(format!("fc_imgedit_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.pdf");
    doc.save_as(&path).unwrap();
    let img = doc.page_images(0).unwrap().pop().unwrap();
    // Anzeige: 80 breit, 40 hoch. Bei 90°/270° sind das im PDF-Raum 40 × 80.
    let (w, h) = if rotation % 180 == 0 {
        (80.0, 40.0)
    } else {
        (40.0, 80.0)
    };
    assert!(
        close(
            img.bounds,
            Rect::new(
                200.0 - w / 2.0,
                300.0 - h / 2.0,
                200.0 + w / 2.0,
                300.0 + h / 2.0
            )
        ),
        "{:?}",
        img.bounds
    );
    let count_images = |d: &Document| {
        d.pdf()
            .objects
            .values()
            .filter(|o| {
                o.as_stream().is_ok_and(|s| {
                    s.dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Image")
                })
            })
            .count()
    };
    let before = count_images(&doc);

    // Weitere Änderung derselben Seite + speichern: Das Bild wird nicht erneut angehängt.
    let block = doc.text_blocks(0).unwrap()[0].clone();
    doc.edit_text_block(0, &block, "Anders").unwrap();
    doc.save().unwrap();
    assert_eq!(count_images(&doc), before);
    assert_eq!(doc.page_images(0).unwrap().len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn rejects_unknown_formats() {
    assert!(NewImage::from_bytes(b"GIF89a....").is_err());
    assert!(NewImage::from_bytes(&[0xFF, 0xD8, 0x00]).is_err());
}
