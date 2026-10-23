//! Formatierter Text: Läufe lesen → mit anderer Schrift/Größe/Stil setzen → speichern → neu laden.

use pdftool_core::rich_text::{FontRef, StyledRun, TextStyle, plain_text};
use pdftool_core::{Color, Document, ParaFrame, Paragraph, Point};

fn fixture(name: &str) -> Document {
    let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    Document::open(path).unwrap()
}

fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

fn para(doc: &Document, starts: &str) -> Paragraph {
    doc.paragraphs(0)
        .unwrap()
        .into_iter()
        .find(|p| p.text.starts_with(starts))
        .unwrap_or_else(|| panic!("kein Absatz mit {starts:?}"))
}

/// Alle /BaseFont-Namen der gespeicherten Datei.
fn base_fonts(doc: &Document) -> Vec<String> {
    doc.pdf()
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter(|d| d.get(b"Type").and_then(|t| t.as_name()).ok() == Some(b"Font"))
        .filter_map(|d| d.get(b"BaseFont").and_then(|b| b.as_name()).ok())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .collect()
}

#[test]
fn reads_runs_of_a_paragraph() {
    let doc = fixture("text_two_pages.pdf");
    let p = para(&doc, "Lorem");
    let runs = doc.paragraph_runs(0, &p).unwrap();
    assert_eq!(plain_text(&runs), p.text);
    assert_eq!(runs.len(), 1, "ein Stil: {runs:#?}");
    let s = &runs[0].style;
    assert!(matches!(s.font, FontRef::Original(_)));
    assert!((s.size - 12.0).abs() < 0.01);
    assert!(!s.bold && !s.italic);
    assert_eq!(s.color, Color::BLACK);
    assert_eq!(doc.font_display_name(0, b"F1"), "Helvetica");
}

#[test]
fn formats_runs_and_keeps_them_after_save() {
    let mut doc = fixture("text_two_pages.pdf");
    let p = para(&doc, "Lorem");
    let base = doc.paragraph_runs(0, &p).unwrap()[0].style.clone();
    let bold = TextStyle {
        bold: true,
        size: 16.0,
        ..base.clone()
    };
    let under = TextStyle {
        underline: true,
        italic: true,
        color: Color::rgb(0.8, 0.0, 0.0),
        font: FontRef::Noto,
        ..base.clone()
    };
    let runs = vec![
        StyledRun {
            text: "Normal ".into(),
            style: base.clone(),
        },
        StyledRun {
            text: "Fett".into(),
            style: bold,
        },
        StyledRun {
            text: " und ".into(),
            style: base.clone(),
        },
        StyledRun {
            text: "unterstrichen".into(),
            style: under,
        },
        StyledRun {
            text: " Ende.".into(),
            style: base,
        },
    ];
    doc.set_paragraph_rich(
        0,
        &p,
        &runs,
        ParaFrame {
            width: Some(400.0),
            ..p.frame()
        },
    )
    .unwrap();
    let saved = reload(&doc);
    let np = para(&saved, "Normal");
    assert_eq!(np.text, "Normal Fett und unterstrichen Ende.");
    let back = saved.paragraph_runs(0, &np).unwrap();
    assert_eq!(plain_text(&back), np.text);
    let fett = back
        .iter()
        .find(|r| r.text.contains("Fett"))
        .expect("Lauf Fett");
    assert!(fett.style.bold, "{back:#?}");
    assert!((fett.style.size - 16.0).abs() < 0.05);
    let rot = back
        .iter()
        .find(|r| r.text.contains("unterstrichen"))
        .expect("Lauf unterstrichen");
    assert!(
        rot.style.italic && (rot.style.color.r - 0.8).abs() < 0.01,
        "{rot:#?}"
    );
    assert!(
        base_fonts(&saved)
            .iter()
            .any(|n| n.ends_with("+NotoSans-Regular"))
    );
}

#[test]
fn missing_characters_come_from_system_font() {
    let mut doc = fixture("text_two_pages.pdf");
    let p = para(&doc, "Lorem");
    // Kyrillisch gibt es in WinAnsi (Helvetica) nicht.
    doc.set_paragraph(0, &p, "Привет aus Köln", p.frame())
        .unwrap();
    let saved = reload(&doc);
    assert!(
        saved
            .paragraphs(0)
            .unwrap()
            .iter()
            .any(|q| q.text == "Привет aus Köln")
    );
    let fonts = base_fonts(&saved);
    match pdftool_core::system_fonts::find_for_base_font("Helvetica") {
        // Helvetica → Arial bzw. Liberation Sans: die fehlenden Zeichen kommen von dort.
        Some(sys) => assert!(
            fonts
                .iter()
                .any(|n| n.ends_with(&format!("+{}", sys.postscript))),
            "{fonts:?} enthält nicht {}",
            sys.postscript
        ),
        None => assert!(fonts.iter().any(|n| n.ends_with("+NotoSans-Regular"))),
    }
}

#[test]
fn adds_rich_text() {
    let mut doc = fixture("text_two_pages.pdf");
    let style = TextStyle {
        strike: true,
        ..TextStyle::noto(14.0, Color::BLUE)
    };
    let runs = vec![StyledRun {
        text: "Neu und durchgestrichen".into(),
        style,
    }];
    doc.add_rich_text(0, Point::new(72.0, 300.0), &runs, None)
        .unwrap();
    let saved = reload(&doc);
    let np = para(&saved, "Neu und");
    assert!((np.size_on_page - 14.0).abs() < 0.05);
}
