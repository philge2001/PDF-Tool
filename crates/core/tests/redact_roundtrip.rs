//! Schwärzen: markieren -> anwenden -> der Inhalt darf NIRGENDS mehr in der Datei stehen.

use pdftool_core::content::{Item, TextShow, interpret_page};
use pdftool_core::raster::Raster;
use pdftool_core::redact::redact_pdf;
use pdftool_core::{AddAnnotation, AnnotKind, Annotation, Color, Document, Quad, Rect};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Markiert die Bereiche zum Schwärzen und wendet sie an.
fn redact(name: &str, areas: &[(usize, Rect)]) -> (Vec<u8>, pdftool_core::redact::RedactionReport) {
    let mut doc = Document::open(fixture(name)).unwrap();
    for &(page, r) in areas {
        let mut a = Annotation::new(
            page,
            AnnotKind::Redact {
                quads: vec![Quad::from_rect(r)],
            },
            Color::RED,
        );
        a.id = doc.allocate_annotation_id();
        doc.execute(Box::new(AddAnnotation::new(a))).unwrap();
    }
    let (bytes, map) = doc.redaction_input().unwrap();
    redact_pdf(&bytes, &map).unwrap()
}

/// Rohbytes plus alle entpackten Streams – hier darf Geschwärztes nicht vorkommen.
fn everything(bytes: &[u8]) -> Vec<u8> {
    let mut all = bytes.to_vec();
    let pdf = lopdf::Document::load_mem(bytes).unwrap();
    for obj in pdf.objects.values() {
        if let Ok(s) = obj.as_stream() {
            all.extend(
                s.decompressed_content()
                    .unwrap_or_else(|_| s.content.clone()),
            );
        }
        // Strings in Dictionaries (z. B. /V eines Felds) stecken in den Rohbytes nur komprimiert
        // (Objekt-Streams) – deshalb auch die Objekte selbst als Text anhängen.
        all.extend(format!("{obj:?}").into_bytes());
    }
    all
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn texts(bytes: &[u8], page: u32) -> Vec<TextShow> {
    let pdf = lopdf::Document::load_mem(bytes).unwrap();
    interpret_page(&pdf, pdf.get_pages()[&page])
        .unwrap()
        .items
        .into_iter()
        .filter_map(|i| match i {
            Item::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

/// Rechteck um bestimmte Zeichen einer Textausgabe der Originaldatei.
fn glyph_area(page: u32, show: usize, range: std::ops::Range<usize>) -> Rect {
    let bytes = std::fs::read(fixture("content_mix.pdf")).unwrap();
    let t = &texts(&bytes, page)[show];
    t.glyphs[range]
        .iter()
        .map(|g| g.rect)
        .reduce(|a, b| a.union(&b))
        .unwrap()
        .expand(-0.5)
}

#[test]
fn removes_exactly_the_marked_characters() {
    let area = glyph_area(1, 0, 8..12); // "4711"
    let original = std::fs::read(fixture("content_mix.pdf")).unwrap();
    assert!(contains(&everything(&original), b"4711"));
    let (out, report) = redact("content_mix.pdf", &[(0, area)]);
    assert_eq!(report.glyphs, 4);
    assert!(
        !contains(&everything(&out), b"4711"),
        "Geschwärzter Text steht noch in der Datei"
    );

    let before = &texts(&original, 1)[0];
    let after = &texts(&out, 1)[0];
    assert_eq!(after.text(), "Geheim: -0815");
    // Der Rest steht genau an der alten Stelle.
    let dash_before = before.glyphs[12].rect.x0;
    let dash_after = after.glyphs.iter().find(|g| g.text == "-").unwrap().rect.x0;
    assert!(
        (dash_before - dash_after).abs() < 0.01,
        "{dash_before} vs {dash_after}"
    );
}

#[test]
fn tj_arrays_invisible_text_and_line_operators() {
    let rning = glyph_area(1, 1, 2..7);
    let ocr = glyph_area(1, 4, 0..14);
    let zwei = glyph_area(1, 6, 6..10);
    let (out, _) = redact("content_mix.pdf", &[(0, rning), (0, ocr), (0, zwei)]);
    let all = everything(&out);
    assert!(!contains(&all, b"rning"));
    assert!(!contains(&all, b"Unsichtbar"));
    assert!(!contains(&all, b"zwei"));
    let t: Vec<String> = texts(&out, 1).iter().map(TextShow::text).collect();
    assert!(t.contains(&"KeTJ-Test".to_string()), "{t:?}");
    assert!(
        t.contains(&"Zeile ".to_string()),
        "Zeilenwechsel von ' bleibt erhalten: {t:?}"
    );
    // "Zeile drei" bleibt auf ihrer Zeile (y = 582).
    let drei = texts(&out, 1)
        .into_iter()
        .find(|s| s.text() == "Zeile drei")
        .unwrap();
    assert!((drei.glyphs[0].rect.y0 + 0.2 * 12.0 - 582.0).abs() < 0.01);
}

#[test]
fn paths_images_and_forms() {
    let line = Rect::new(240.0, 515.0, 260.0, 530.0); // auf der roten Linie
    let blue = Rect::new(90.0, 510.0, 110.0, 530.0); // im blauen Rechteck
    let image_left = Rect::new(350.0, 470.0, 400.0, 550.0); // linke Bildhälfte
    let inline_left = Rect::new(470.0, 470.0, 500.0, 530.0);
    let form = Rect::new(60.0, 418.0, 300.0, 440.0);
    let (out, report) = redact(
        "content_mix.pdf",
        &[
            (0, line),
            (0, blue),
            (0, image_left),
            (0, inline_left),
            (0, form),
        ],
    );
    assert_eq!(report.paths, 1, "nur die Linie, reine Rechtecke bleiben");
    assert_eq!(report.images_changed, 2);
    assert!(!contains(&everything(&out), b"Formular-XObject"));

    let pdf = lopdf::Document::load_mem(&out).unwrap();
    let page = interpret_page(&pdf, pdf.get_pages()[&1]).unwrap();
    for item in &page.items {
        match item {
            Item::Image { name: Some(n), .. } => {
                let res = pdftool_core::content::page_resources(&pdf, pdf.get_pages()[&1]).unwrap();
                let xo = res.get(b"XObject").unwrap().as_dict().unwrap();
                let id = xo.get(n).unwrap().as_reference().unwrap();
                let s = pdf.get_object(id).unwrap().as_stream().unwrap();
                let r = Raster::decode(&pdf, s).unwrap();
                // 4x4 Graustufen: Spalten 0 und 1 schwarz, 2 und 3 unverändert.
                for row in 0..4 {
                    let px = &r.data[row * 4..row * 4 + 4];
                    assert_eq!(&px[..2], &[0, 0], "Zeile {row}");
                    assert_eq!(px[2], ((row * 4 + 2) * 16) as u8);
                }
            }
            Item::Image { name: None, op, .. } => {
                let lopdf::Object::Stream(s) = &page.ops[*op].operands[0] else {
                    panic!()
                };
                let mut s = s.clone();
                s.dict = pdftool_core::raster::normalize_inline(&s.dict);
                let r = Raster::decode(&pdf, &s).unwrap();
                assert_eq!(r.data, vec![0x00, 0x55, 0x00, 0xFF]);
            }
            Item::Path { paint, .. } if page.ops[*paint].operator == "S" => {
                panic!("Die rote Linie wird noch gezeichnet")
            }
            _ => {}
        }
    }
    // Schwarze Kästen am Ende.
    let n = page.ops.len();
    assert_eq!(page.ops[n - 2].operator, "f");
}

#[test]
fn removes_annotations_and_form_fields_in_area() {
    let doc = Document::open(fixture("form.pdf")).unwrap();
    let name = doc.forms().by_name("person.name").unwrap().widgets[0].rect;
    let (out, report) = redact("form.pdf", &[(0, name.expand(-1.0))]);
    assert!(report.fields >= 1 && report.annotations >= 1, "{report:?}");
    assert!(!contains(&everything(&out), b"Max Mustermann"));
    let back = Document::from_bytes(out.clone()).unwrap();
    assert!(back.forms().by_name("person.name").is_none());
    assert!(back.forms().by_name("person.email").is_some());
    // Keine /Redact-Markierungen mehr.
    assert!(!contains(&out, b"/Redact"));
}

#[test]
fn cid_font_glyphs_keep_positions() {
    let area = glyph_area(2, 0, 0..2);
    let original = std::fs::read(fixture("content_mix.pdf")).unwrap();
    let x_before = texts(&original, 2)[0].glyphs[2].rect.x0;
    let (out, report) = redact("content_mix.pdf", &[(1, area)]);
    assert_eq!(report.glyphs, 2);
    let after = &texts(&out, 2)[0];
    let codes: Vec<u32> = after.glyphs.iter().map(|g| g.code).collect();
    assert_eq!(codes, vec![0x26, 0x28, 0x2A, 0x01]);
    assert!((after.glyphs[0].rect.x0 - x_before).abs() < 0.01);
}

#[test]
fn old_revisions_are_gone_after_rewrite() {
    // Erst "4711" in einer Änderung speichern (inkrementell), dann schwärzen:
    // auch die alte Revision darf nicht mehr in der Datei stehen.
    let area = glyph_area(1, 0, 8..12);
    let (out, _) = redact("content_mix.pdf", &[(0, area)]);
    assert_eq!(
        out.windows(5).filter(|w| w == b"%%EOF").count(),
        1,
        "genau eine Revision"
    );
}
