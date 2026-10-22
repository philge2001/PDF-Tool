//! PDF/A-2b-Umwandlung: strukturelle Anforderungen prüfen (veraPDF läuft nur manuell, siehe
//! CLAUDE.md – dort bestehen alle Fixtures außer content_mix.pdf mit nicht eingebetteter CID-Schrift).

use foliocraft_core::pdfa::convert_to_pdfa;

fn convert(name: &str) -> (lopdf::Document, foliocraft_core::pdfa::PdfaReport) {
    let bytes = std::fs::read(format!(
        "{}/../../tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let (out, report) = convert_to_pdfa(&bytes).unwrap();
    (lopdf::Document::load_mem(&out).unwrap(), report)
}

fn fonts_embedded(pdf: &lopdf::Document) -> bool {
    pdf.objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .all(|d| {
            let is_simple = d.get(b"Type").and_then(|t| t.as_name()).ok() == Some(b"Font")
                && matches!(
                    d.get(b"Subtype").and_then(|t| t.as_name()).ok(),
                    Some(b"Type1" | b"TrueType")
                );
            !is_simple
                || d.get(b"FontDescriptor")
                    .and_then(|r| r.as_reference())
                    .and_then(|r| pdf.get_dictionary(r))
                    .is_ok_and(|fd| {
                        fd.has(b"FontFile2") || fd.has(b"FontFile") || fd.has(b"FontFile3")
                    })
        })
}

#[test]
fn text_document_becomes_pdfa() {
    let (pdf, report) = convert("text_two_pages.pdf");
    assert!(report.problems.is_empty(), "{report:?}");
    assert_eq!(pdf.version, "1.7");
    assert!(pdf.trailer.has(b"ID"));
    let catalog = pdf.catalog().unwrap();
    // OutputIntent mit sRGB-Profil.
    let intents = catalog.get(b"OutputIntents").unwrap().as_array().unwrap();
    let intent = pdf
        .get_dictionary(intents[0].as_reference().unwrap())
        .unwrap();
    assert_eq!(intent.get(b"S").unwrap().as_name().unwrap(), b"GTS_PDFA1");
    // XMP: unkomprimiert, mit PDF/A-Kennung.
    let meta = pdf
        .get_object(catalog.get(b"Metadata").unwrap().as_reference().unwrap())
        .unwrap()
        .as_stream()
        .unwrap();
    assert!(!meta.dict.has(b"Filter"));
    let xmp = String::from_utf8_lossy(&meta.content);
    assert!(xmp.contains("<pdfaid:part>2</pdfaid:part>") && xmp.contains("<pdfaid:conformance>B"));
    assert!(fonts_embedded(&pdf));
    // Text unverändert lesbar.
    let doc = foliocraft_core::Document::from_bytes({
        let mut b = Vec::new();
        pdf.clone().save_to(&mut b).unwrap();
        b
    })
    .unwrap();
    assert!(
        doc.text_blocks(0).unwrap()[0]
            .text
            .contains("FolioCraft Testdokument")
    );
}

#[test]
fn forms_and_annotations_get_appearances() {
    for name in ["form.pdf", "with_annotations.pdf"] {
        let (pdf, report) = convert(name);
        assert!(report.problems.is_empty(), "{name}: {report:?}");
        assert!(fonts_embedded(&pdf), "{name}");
        for d in pdf.objects.values().filter_map(|o| o.as_dict().ok()) {
            if d.get(b"Type").and_then(|t| t.as_name()).ok() != Some(b"Annot") {
                continue;
            }
            let sub = d.get(b"Subtype").unwrap().as_name().unwrap();
            if sub != b"Popup" && sub != b"Link" {
                assert!(d.has(b"AP"), "{name}: {}", String::from_utf8_lossy(sub));
            }
            assert_eq!(d.get(b"F").unwrap().as_i64().unwrap() & 4, 4, "druckbar");
        }
    }
}

#[test]
fn unfixable_problems_are_reported() {
    let (_, report) = convert("content_mix.pdf");
    assert!(
        report.problems.iter().any(|p| p.contains("CID-Schrift")),
        "{report:?}"
    );
}
