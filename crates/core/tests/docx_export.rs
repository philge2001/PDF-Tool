//! Word-Export: Inhalt der DOCX-Datei prüfen (ZIP öffnen, XML lesen).

use pdftool_core::Document;
use std::io::Read;

fn fixture(name: &str) -> Document {
    let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    Document::open(path).unwrap()
}

fn unzip(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut data = Vec::new();
            f.read_to_end(&mut data).unwrap();
            (f.name().unwrap().into_owned(), data)
        })
        .collect()
}

#[test]
fn paragraphs_page_breaks_and_images() {
    let doc = fixture("text_two_pages.pdf");
    let files = unzip(&pdftool_core::docx::export_docx(&doc, &[0, 1], &|_, _| true).unwrap());
    let xml = String::from_utf8(files["word/document.xml"].clone()).unwrap();
    assert!(xml.contains("Grüße aus Köln"), "Umlaute als UTF-8");
    // Absatz aus vier Zeilen = ein w:p.
    assert!(xml.contains("dolore magna. Grüße aus Köln"));
    assert_eq!(
        xml.matches(r#"w:type="page""#).count(),
        1,
        "ein Seitenumbruch"
    );
    assert!(xml.contains(r#"<w:sz w:val="48"/>"#), "Überschrift 24 pt");
    for part in [
        "[Content_Types].xml",
        "_rels/.rels",
        "word/_rels/document.xml.rels",
    ] {
        assert!(files.contains_key(part), "{part}");
    }

    let doc = fixture("content_mix.pdf");
    let files = unzip(&pdftool_core::docx::export_docx(&doc, &[0], &|_, _| true).unwrap());
    let png = &files["word/media/bild1.png"];
    let img = pdftool_core::NewImage::from_bytes(png).unwrap();
    assert_eq!((img.width, img.height), (4, 4));
    let xml = String::from_utf8(files["word/document.xml"].clone()).unwrap();
    // 100 × 60 pt = 1 270 000 × 762 000 EMU.
    assert!(xml.contains(r#"cx="1270000" cy="762000""#));

    // Abbrechen.
    assert!(pdftool_core::docx::export_docx(&doc, &[0], &|_, _| false).is_err());
}
