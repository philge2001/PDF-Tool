//! Roundtrip-Tests: Annotation erstellen -> speichern -> neu laden -> vergleichen.

use pdftool_core::annotation::import::page_annots;
use pdftool_core::{
    AddAnnotation, AnnotKind, Annotation, Color, Document, Point, Quad, Rect, RemoveAnnotation,
    UpdateAnnotation,
};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Eindeutiger Pfad im Temp-Verzeichnis (Tests laufen parallel).
fn temp_pdf(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("pdftool-test-{tag}-{nanos}.pdf"))
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn approx_point(a: Point, b: Point) -> bool {
    approx(a.x, b.x) && approx(a.y, b.y)
}

fn approx_rect(a: Rect, b: Rect) -> bool {
    approx(a.x0, b.x0) && approx(a.y0, b.y0) && approx(a.x1, b.x1) && approx(a.y1, b.y1)
}

/// Vergleicht alles Inhaltliche (nicht ID, Herkunft, Name, Datum) mit Toleranz für Gleitkomma.
fn assert_same(a: &Annotation, b: &Annotation) {
    assert_eq!(a.page, b.page, "Seite");
    assert_eq!(a.contents, b.contents, "Inhalt");
    assert_eq!(a.author, b.author, "Autor");
    assert!(
        approx(a.opacity, b.opacity),
        "Deckkraft {} vs {}",
        a.opacity,
        b.opacity
    );
    let (ca, cb) = (a.color, b.color);
    assert!(
        approx(ca.r, cb.r) && approx(ca.g, cb.g) && approx(ca.b, cb.b),
        "Farbe {ca:?} vs {cb:?}"
    );
    use AnnotKind::*;
    match (&a.kind, &b.kind) {
        (Highlight { quads: q1 }, Highlight { quads: q2 })
        | (Underline { quads: q1 }, Underline { quads: q2 })
        | (StrikeOut { quads: q1 }, StrikeOut { quads: q2 }) => {
            assert_eq!(q1.len(), q2.len());
            for (x, y) in q1.iter().zip(q2) {
                assert!(approx_rect(x.bounds(), y.bounds()), "{x:?} vs {y:?}");
            }
        }
        (Note { pos: p1 }, Note { pos: p2 }) => assert!(approx_point(*p1, *p2)),
        (
            Ink {
                strokes: s1,
                width: w1,
            },
            Ink {
                strokes: s2,
                width: w2,
            },
        ) => {
            assert!(approx(*w1, *w2));
            assert_eq!(s1.len(), s2.len());
            for (x, y) in s1.iter().zip(s2) {
                assert_eq!(x.len(), y.len());
                assert!(x.iter().zip(y).all(|(p, q)| approx_point(*p, *q)));
            }
        }
        (
            Square {
                rect: r1,
                width: w1,
            },
            Square {
                rect: r2,
                width: w2,
            },
        )
        | (
            Circle {
                rect: r1,
                width: w1,
            },
            Circle {
                rect: r2,
                width: w2,
            },
        ) => {
            assert!(approx_rect(*r1, *r2) && approx(*w1, *w2));
        }
        (
            Line {
                start: s1,
                end: e1,
                width: w1,
                arrow_end: a1,
            },
            Line {
                start: s2,
                end: e2,
                width: w2,
                arrow_end: a2,
            },
        ) => {
            assert!(approx_point(*s1, *s2) && approx_point(*e1, *e2) && approx(*w1, *w2));
            assert_eq!(a1, a2);
        }
        (
            FreeText {
                rect: r1,
                font_size: f1,
            },
            FreeText {
                rect: r2,
                font_size: f2,
            },
        ) => {
            assert!(approx_rect(*r1, *r2) && approx(*f1, *f2));
        }
        (x, y) => panic!("Typ unterschiedlich: {x:?} vs {y:?}"),
    }
}

/// Eine Annotation jedes Typs.
fn one_of_each() -> Vec<Annotation> {
    let quad = |x: f32, y: f32| Quad::from_rect(Rect::new(x, y, x + 120.0, y + 14.0));
    let mut list = vec![
        Annotation::new(
            0,
            AnnotKind::Highlight {
                quads: vec![quad(72.0, 716.0), quad(72.0, 696.0)],
            },
            Color::YELLOW,
        ),
        Annotation::new(
            0,
            AnnotKind::Underline {
                quads: vec![quad(72.0, 676.0)],
            },
            Color::GREEN,
        ),
        Annotation::new(
            0,
            AnnotKind::StrikeOut {
                quads: vec![quad(72.0, 656.0)],
            },
            Color::RED,
        ),
        Annotation::new(
            0,
            AnnotKind::Note {
                pos: Point::new(400.0, 700.0),
            },
            Color::YELLOW,
        ),
        Annotation::new(
            1,
            AnnotKind::Ink {
                strokes: vec![
                    vec![
                        Point::new(100.0, 400.0),
                        Point::new(150.0, 450.0),
                        Point::new(200.0, 400.0),
                    ],
                    vec![Point::new(220.0, 420.0)],
                ],
                width: 2.5,
            },
            Color::BLUE,
        ),
        Annotation::new(
            1,
            AnnotKind::Square {
                rect: Rect::new(100.0, 200.0, 250.0, 300.0),
                width: 2.0,
            },
            Color::RED,
        ),
        Annotation::new(
            1,
            AnnotKind::Circle {
                rect: Rect::new(300.0, 200.0, 450.0, 300.0),
                width: 1.5,
            },
            Color::GREEN,
        ),
        Annotation::new(
            1,
            AnnotKind::Line {
                start: Point::new(100.0, 100.0),
                end: Point::new(300.0, 150.0),
                width: 2.0,
                arrow_end: true,
            },
            Color::BLUE,
        ),
        Annotation::new(
            1,
            AnnotKind::FreeText {
                rect: Rect::new(300.0, 500.0, 520.0, 560.0),
                font_size: 14.0,
            },
            Color::BLACK,
        ),
    ];
    for (i, a) in list.iter_mut().enumerate() {
        a.author = "Testautorin".into();
        a.contents = format!("Kommentar Nr. {i} – Grüße aus Köln");
    }
    list
}

#[test]
fn imports_existing_annotations_from_other_tools() {
    let doc = Document::open(fixture("with_annotations.pdf")).unwrap();
    let annots: Vec<_> = doc.annotations().iter().collect();
    assert_eq!(
        annots.len(),
        2,
        "Highlight und Notiz; der Link ist nicht verwaltet"
    );
    assert!(matches!(annots[0].kind, AnnotKind::Highlight { .. }));
    assert_eq!(annots[0].contents, "Vorhandener Kommentar");
    assert_eq!(annots[0].author, "Fremdautor");
    assert_eq!(annots[0].name, "fixture-highlight-1");
    assert!(matches!(annots[1].kind, AnnotKind::Note { .. }));
    assert!(!doc.is_dirty());
}

#[test]
fn every_kind_survives_save_and_reload() {
    let mut doc = Document::open(fixture("text_two_pages.pdf")).unwrap();
    let originals = one_of_each();
    for a in &originals {
        let mut a = a.clone();
        a.id = doc.allocate_annotation_id();
        doc.execute(Box::new(AddAnnotation::new(a))).unwrap();
    }
    assert!(doc.is_dirty());

    let path = temp_pdf("each-kind");
    doc.save_as(&path).unwrap();
    assert!(!doc.is_dirty());

    let reloaded = Document::open(&path).unwrap();
    let loaded: Vec<_> = reloaded.annotations().iter().collect();
    assert_eq!(loaded.len(), originals.len());
    // Reihenfolge pro Seite bleibt erhalten; Seite 0 kommt vor Seite 1.
    for (orig, back) in originals.iter().zip(&loaded) {
        assert_same(orig, back);
        assert!(
            back.name.starts_with("pdftool-"),
            "eindeutiger /NM vergeben"
        );
        assert!(back.modified.starts_with("D:"), "Änderungsdatum gesetzt");
    }

    // Jede gespeicherte Annotation hat eine Appearance (/AP /N als Stream).
    let pdf = reloaded.pdf();
    for page in reloaded.pages() {
        for entry in page_annots(pdf, page.object_id) {
            let id = entry.as_reference().unwrap();
            let d = pdf.get_dictionary(id).unwrap();
            let ap = d.get(b"AP").unwrap().as_dict().unwrap();
            let n = ap.get(b"N").unwrap().as_reference().unwrap();
            assert!(pdf.get_object(n).unwrap().as_stream().is_ok());
        }
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn save_is_incremental() {
    let original = std::fs::read(fixture("text_two_pages.pdf")).unwrap();
    let mut doc = Document::from_bytes(original.clone()).unwrap();
    let mut a = Annotation::new(
        0,
        AnnotKind::Note {
            pos: Point::new(100.0, 100.0),
        },
        Color::YELLOW,
    );
    a.id = doc.allocate_annotation_id();
    doc.execute(Box::new(AddAnnotation::new(a))).unwrap();
    let bytes = doc.to_bytes().unwrap();
    assert!(bytes.len() > original.len());
    assert_eq!(
        &bytes[..original.len()],
        &original[..],
        "Originalbytes unverändert am Anfang"
    );
    // Nachtrag mit eigener xref-Tabelle und Verweis auf die vorherige.
    let tail = String::from_utf8_lossy(&bytes[original.len()..]);
    assert!(
        tail.contains("/Prev"),
        "Trailer verweist auf vorherige xref"
    );
}

#[test]
fn modify_and_delete_imported_annotations() {
    let mut doc = Document::open(fixture("with_annotations.pdf")).unwrap();
    let ids: Vec<_> = doc.annotations().iter().map(|a| a.id).collect();
    let highlight = doc.annotations().get(ids[0]).unwrap().clone();
    let mut changed = highlight.clone();
    changed.color = Color::GREEN;
    changed.contents = "Geändert".into();
    changed.translate(0.0, -20.0);
    doc.execute(Box::new(UpdateAnnotation::new(
        highlight.clone(),
        changed.clone(),
    )))
    .unwrap();
    doc.execute(Box::new(RemoveAnnotation::new(ids[1])))
        .unwrap();

    let path = temp_pdf("modify");
    doc.save_as(&path).unwrap();
    let reloaded = Document::open(&path).unwrap();

    let loaded: Vec<_> = reloaded.annotations().iter().collect();
    assert_eq!(loaded.len(), 1, "Notiz gelöscht");
    assert_same(&changed, loaded[0]);
    assert_eq!(
        loaded[0].name, "fixture-highlight-1",
        "/NM bleibt beim Ändern erhalten"
    );
    // Geändert = gleiche Objektnummer neu geschrieben, kein Duplikat.
    assert_eq!(
        loaded[0].source.unwrap().object_id,
        highlight.source.unwrap().object_id
    );

    // Der fremde Link ist noch da.
    let pdf = reloaded.pdf();
    let annots = page_annots(pdf, reloaded.pages()[0].object_id);
    assert_eq!(annots.len(), 2);
    let has_link = annots.iter().any(|o| {
        let d = pdf.get_dictionary(o.as_reference().unwrap()).unwrap();
        d.get(b"Subtype").unwrap().as_name().unwrap() == b"Link"
    });
    assert!(has_link, "nicht verwaltete Annotationen bleiben erhalten");
    let _ = std::fs::remove_file(path);
}

#[test]
fn unchanged_document_saves_without_touching_pages() {
    let original = std::fs::read(fixture("with_annotations.pdf")).unwrap();
    let doc = Document::from_bytes(original.clone()).unwrap();
    let bytes = doc.to_bytes().unwrap();
    let reloaded = Document::from_bytes(bytes).unwrap();
    assert_eq!(reloaded.annotations().len(), 2);
    let tail = &reloaded.bytes()[original.len()..];
    assert!(
        !String::from_utf8_lossy(tail).contains("/Annots"),
        "keine Seite neu geschrieben"
    );
}

#[test]
fn save_twice_and_undo_after_save() {
    let path = temp_pdf("twice");
    std::fs::copy(fixture("text_two_pages.pdf"), &path).unwrap();
    let mut doc = Document::open(&path).unwrap();

    let mut a = Annotation::new(
        0,
        AnnotKind::Square {
            rect: Rect::new(10.0, 10.0, 50.0, 50.0),
            width: 1.0,
        },
        Color::RED,
    );
    a.id = doc.allocate_annotation_id();
    let id = a.id;
    doc.execute(Box::new(AddAnnotation::new(a))).unwrap();
    doc.save().unwrap();
    let first_len = std::fs::metadata(&path).unwrap().len();

    // Rückgängig nach dem Speichern -> wieder "geändert" -> erneut speichern entfernt sie.
    assert!(doc.undo().unwrap());
    assert!(doc.is_dirty());
    assert!(doc.annotations().get(id).is_none());
    doc.save().unwrap();
    assert!(
        std::fs::metadata(&path).unwrap().len() > first_len,
        "zweiter Nachtrag angehängt"
    );
    assert_eq!(Document::open(&path).unwrap().annotations().len(), 0);

    // Wiederherstellen und speichern -> sie ist wieder da, genau einmal.
    assert!(doc.redo().unwrap());
    doc.save().unwrap();
    assert_eq!(Document::open(&path).unwrap().annotations().len(), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn freetext_with_umlauts() {
    let mut doc = Document::open(fixture("text_two_pages.pdf")).unwrap();
    let mut a = Annotation::new(
        0,
        AnnotKind::FreeText {
            rect: Rect::new(72.0, 400.0, 300.0, 460.0),
            font_size: 12.0,
        },
        Color::BLUE,
    );
    a.id = doc.allocate_annotation_id();
    a.contents = "Übergrößenträger: äöü ÄÖÜ ß € – „Zitat“".into();
    doc.execute(Box::new(AddAnnotation::new(a.clone())))
        .unwrap();
    let reloaded = Document::from_bytes(doc.to_bytes().unwrap()).unwrap();
    let back = reloaded.annotations().iter().next().unwrap();
    assert_same(&a, back);
}
