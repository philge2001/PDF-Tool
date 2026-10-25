//! Absätze: erkennen → neu setzen (Text, Kasten verschieben, Breite ändern) → speichern → neu laden.

use pdftool_core::{Document, ParaFrame, Paragraph, Point};

fn fixture(name: &str) -> Document {
    let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    Document::open(path).unwrap()
}

fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

fn para<'a>(paras: &'a [Paragraph], starts: &str) -> &'a Paragraph {
    paras
        .iter()
        .find(|p| p.text.starts_with(starts))
        .unwrap_or_else(|| panic!("kein Absatz mit {starts:?}: {paras:#?}"))
}

#[test]
fn detects_paragraph_of_four_lines() {
    let doc = fixture("text_two_pages.pdf");
    let paras = doc.paragraphs(0).unwrap();
    let p = para(&paras, "Lorem");
    assert_eq!(p.lines, 4, "{paras:#?}");
    assert!(p.text.contains("magna. Grüße aus Köln"), "{}", p.text);
    assert!((p.line_height - 20.0).abs() < 0.01);
    assert!((p.origin.x - 72.0).abs() < 0.01 && (p.origin.y - 720.0).abs() < 0.01);
    // Die Überschrift (24 pt) ist ein eigener Absatz.
    assert_eq!(para(&paras, "PDF Tool").lines, 1);
}

#[test]
fn rewrite_wraps_to_frame_and_can_be_moved_and_resized() {
    let mut doc = fixture("text_two_pages.pdf");
    let p = para(&doc.paragraphs(0).unwrap(), "Lorem").clone();
    let text = "Ein ganz neuer Absatz mit Umlauten (äöü), Kyrillisch (Привет) und so viel Text, \
                dass er in einem schmalen Kasten über mehrere Zeilen umbrochen werden muss.";
    let frame = ParaFrame {
        origin: p.origin,
        width: Some(200.0),
        justify: false,
    };
    doc.set_paragraph(0, &p, text, frame).unwrap();

    let saved = reload(&doc);
    let paras = saved.paragraphs(0).unwrap();
    let np = para(&paras, "Ein ganz").clone();
    assert_eq!(np.text, text);
    assert!(np.lines >= 4, "umbrochen: {np:#?}");
    assert!(np.width <= 200.5, "passt in den Kasten: {}", np.width);
    assert!((np.line_height - 20.0).abs() < 0.01, "Zeilenabstand bleibt");
    assert!(np.origin.distance(p.origin) < 0.01);
    assert!(
        !paras.iter().any(|q| q.text.contains("Lorem")),
        "alter Text weg"
    );
    // Die Überschrift bleibt, wo sie ist.
    let head = para(&paras, "PDF Tool");
    assert!((head.origin.y - 760.0).abs() < 0.01);

    // Erneut setzen mit dem gemessenen Kasten: gleicher Umbruch (stabil).
    let mut again = saved;
    again
        .set_paragraph(0, &np, &np.text.clone(), np.frame())
        .unwrap();
    let stable = para(&again.paragraphs(0).unwrap(), "Ein ganz").clone();
    assert_eq!(stable.lines, np.lines);
    assert!((stable.width - np.width).abs() < 0.01);
    let saved = again;

    // Kasten verschieben und breiter machen: weniger Zeilen, neuer Ort, kein Rest des alten Absatzes.
    let mut saved = reload(&saved);
    let np = para(&saved.paragraphs(0).unwrap(), "Ein ganz").clone();
    let target = Point::new(100.0, 500.0);
    saved
        .set_paragraph(
            0,
            &np,
            &np.text.clone(),
            ParaFrame {
                origin: target,
                width: Some(420.0),
                justify: false,
            },
        )
        .unwrap();
    let moved = reload(&saved);
    let paras = moved.paragraphs(0).unwrap();
    let mp = para(&paras, "Ein ganz");
    assert!(mp.origin.distance(target) < 0.01, "{:?}", mp.origin);
    assert!(mp.lines < np.lines);
    assert_eq!(
        paras
            .iter()
            .filter(|q| q.text.starts_with("Ein ganz"))
            .count(),
        1
    );
    let content = moved.page_content(0).unwrap().content;
    let marks = content.windows(6).filter(|w| w == b"FcPara").count();
    assert_eq!(marks, 1, "der alte /FcPara-Abschnitt wurde ersetzt");
}

#[test]
fn justified_lines_end_at_frame_edge() {
    let mut doc = fixture("text_two_pages.pdf");
    let p = para(&doc.paragraphs(0).unwrap(), "Lorem").clone();
    let text = "Blocksatz verteilt den freien Platz einer Zeile gleichmäßig auf die Wortabstände, \
                damit alle Zeilen außer der letzten genau am rechten Rand des Kastens enden.";
    doc.set_paragraph(
        0,
        &p,
        text,
        ParaFrame {
            origin: p.origin,
            width: Some(250.0),
            justify: true,
        },
    )
    .unwrap();
    let saved = reload(&doc);
    let np = para(&saved.paragraphs(0).unwrap(), "Blocksatz").clone();
    assert!(np.justified, "{np:#?}");
    assert!((np.width - 250.0).abs() < 0.5, "{}", np.width);
    assert_eq!(np.text, text, "Leerzeichen bleiben als Zeichen erhalten");
}

#[test]
fn undo_restores_original_paragraph() {
    let mut doc = fixture("text_two_pages.pdf");
    let p = para(&doc.paragraphs(0).unwrap(), "Lorem").clone();
    doc.set_paragraph(0, &p, "Kurz.", p.frame()).unwrap();
    assert_eq!(para(&doc.paragraphs(0).unwrap(), "Kurz").lines, 1);
    doc.undo().unwrap();
    assert_eq!(
        para(&reload(&doc).paragraphs(0).unwrap(), "Lorem").text,
        p.text
    );
}

#[test]
fn hard_line_breaks_survive_resizing() {
    let mut doc = fixture("text_two_pages.pdf");
    let p = para(&doc.paragraphs(0).unwrap(), "Lorem").clone();
    let text = "Erste Zeile\nZweite\n\nNach einer Leerzeile folgt hier noch etwas mehr Text, \
                der im Kasten umbrochen wird.";
    let frame = ParaFrame {
        origin: p.origin,
        width: Some(220.0),
        justify: false,
    };
    doc.set_paragraph(0, &p, text, frame).unwrap();
    let saved = reload(&doc);
    let np = para(&saved.paragraphs(0).unwrap(), "Erste").clone();
    assert_eq!(np.text, text, "{np:#?}");
    assert_eq!(&np.breaks[..4], &[0, 1, 2, 0]);
    assert_eq!(np.rows(), np.lines + 1, "Leerzeile zählt für die Höhe");
    assert!(
        (np.line_height - 20.0).abs() < 0.01,
        "Leerzeile zählt nicht als Abstand"
    );

    // Breite ändern (wie mit dem Anfasser): Umbrüche bleiben, Fließtext wird neu umbrochen.
    for width in [400.0, 150.0] {
        let mut d = reload(&saved);
        let q = para(&d.paragraphs(0).unwrap(), "Erste").clone();
        let frame = ParaFrame {
            width: Some(width),
            ..q.frame()
        };
        d.set_paragraph(0, &q, &q.text.clone(), frame).unwrap();
        let r = para(&d.paragraphs(0).unwrap(), "Erste").clone();
        assert_eq!(r.text, text, "Breite {width}");
        assert!(r.width <= width + 0.5);
    }

    // Formatierte Läufe (Editor) enthalten die Umbrüche ebenfalls.
    let runs = saved.paragraph_runs(0, &np).unwrap();
    assert_eq!(pdftool_core::rich_text::plain_text(&runs), text);
}

/// Fremde PDF ohne Markierungen: kurze Zeilen, deren nächstes Wort noch gepasst hätte,
/// enden mit einem harten Umbruch (Adresse); volle Zeilen bleiben Fließtext.
#[test]
fn short_lines_of_foreign_pdf_are_hard_breaks() {
    use lopdf::{Object, Stream, content::Content, content::Operation, dictionary};
    let mut pdf = lopdf::Document::with_version("1.7");
    let font = pdf.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let lines = [
        "Herr Max Mustermann wohnt seit vielen Jahren unter",
        "dieser Adresse:",
        "Hauptstrasse 5",
        "12345 Koeln",
    ];
    let mut ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("TL", vec![14.into()]),
        Operation::new("Td", vec![72.into(), 700.into()]),
    ];
    for l in lines {
        ops.push(Operation::new("Tj", vec![Object::string_literal(l)]));
        ops.push(Operation::new("T*", vec![]));
    }
    ops.push(Operation::new("ET", vec![]));
    let content = pdf.add_object(Stream::new(
        dictionary! {},
        Content { operations: ops }.encode().unwrap(),
    ));
    let pages = pdf.new_object_id();
    let page = pdf.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Contents" => content, "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
    });
    pdf.objects.insert(
        pages,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
        ),
    );
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();

    let doc = Document::from_bytes(bytes).unwrap();
    let paras = doc.paragraphs(0).unwrap();
    let p = para(&paras, "Herr");
    assert_eq!(
        p.text,
        "Herr Max Mustermann wohnt seit vielen Jahren unter dieser Adresse:\nHauptstrasse 5\n12345 Koeln",
        "{paras:#?}"
    );
}
