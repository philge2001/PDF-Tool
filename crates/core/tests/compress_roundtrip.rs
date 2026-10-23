//! Komprimieren: Bilder verkleinern, Text bleibt, Datei wird kleiner und bleibt gültig.

use pdftool_core::Document;
use pdftool_core::compress::{CompressOptions, compress_pdf};
use pdftool_core::raster::Raster;

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// (Breite, Höhe, Filter) aller Bild-XObjects.
fn images(bytes: &[u8]) -> Vec<(i64, i64, String)> {
    let pdf = lopdf::Document::load_mem(bytes).unwrap();
    let mut v: Vec<_> = pdf
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| s.dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Image"))
        .map(|s| {
            let n = |k: &[u8]| s.dict.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
            let f = s
                .dict
                .get(b"Filter")
                .ok()
                .and_then(|f| f.as_name().ok())
                .map(|f| String::from_utf8_lossy(f).into_owned())
                .unwrap_or_default();
            (n(b"Width"), n(b"Height"), f)
        })
        .collect();
    v.sort();
    v
}

#[test]
fn downsamples_photo_and_mask() {
    let before = fixture_bytes("photo.pdf");
    let (after, report) = compress_pdf(&before, &CompressOptions::default(), &|_, _| true).unwrap();
    assert_eq!(report.images_resampled, 1);
    assert!(
        after.len() < before.len() / 2,
        "{} -> {}",
        before.len(),
        after.len()
    );
    // 200 × 133 pt bei 150 dpi ≈ 417 × 278 Pixel; Bild als JPEG, Maske gleich groß.
    let imgs = images(&after);
    assert_eq!(imgs.len(), 2, "{imgs:?}");
    for (w, h, _) in &imgs {
        assert!(
            (410..=420).contains(w) && (275..=282).contains(h),
            "{imgs:?}"
        );
    }
    assert!(imgs.iter().any(|i| i.2 == "DCTDecode"));
    // Gültig, Text unverändert, Bild dekodierbar.
    let doc = Document::from_bytes(after.clone()).unwrap();
    let text: String = doc
        .text_blocks(0)
        .unwrap()
        .iter()
        .map(|b| b.text.clone())
        .collect();
    assert!(text.contains("Foto mit hoher Aufloesung"));
    let pdf = lopdf::Document::load_mem(&after).unwrap();
    let jpeg = pdf
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| s.dict.get(b"Filter").and_then(|f| f.as_name()).ok() == Some(b"DCTDecode"))
        .unwrap();
    let r = Raster::decode(&pdf, jpeg).unwrap();
    // Oben links: rot ≈ 0, grün ≈ 255 (Verlauf).
    assert!(r.data[0] < 40 && r.data[1] > 215, "{:?}", &r.data[..3]);
}

#[test]
fn small_images_untouched_and_without_resampling() {
    let before = fixture_bytes("content_mix.pdf");
    let (after, report) = compress_pdf(&before, &CompressOptions::default(), &|_, _| true).unwrap();
    assert_eq!(report.images_resampled, 0);
    assert_eq!(images(&after), images(&before));

    let photo = fixture_bytes("photo.pdf");
    let opts = CompressOptions {
        image_dpi: None,
        ..Default::default()
    };
    let (after, report) = compress_pdf(&photo, &opts, &|_, _| true).unwrap();
    assert_eq!(report.images_resampled, 0);
    assert_eq!(images(&after), images(&photo));
}

#[test]
fn removes_old_revisions() {
    // Inkrementell gespeicherte Änderung -> alte Objekte stehen noch in der Datei.
    let mut doc = Document::open(format!(
        "{}/../../tests/fixtures/text_two_pages.pdf",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let b = doc.text_blocks(0).unwrap()[0].clone();
    doc.edit_text_block(0, &b, "Neu").unwrap();
    let incremental = doc.to_bytes().unwrap();
    let (after, report) =
        compress_pdf(&incremental, &CompressOptions::default(), &|_, _| true).unwrap();
    assert!(report.objects_removed >= 1, "{report:?}");
    assert!(after.len() < incremental.len());
    let text: String = Document::from_bytes(after)
        .unwrap()
        .text_blocks(0)
        .unwrap()
        .iter()
        .map(|b| b.text.clone())
        .collect();
    assert!(text.starts_with("Neu"));
}
