//! Schreibt eine Annotation jedes Typs in eine Kopie einer PDF – zum Prüfen in anderen Readern.
//!
//!   cargo run -p pdftool-core --example annot_demo -- eingabe.pdf ausgabe.pdf

use pdftool_core::{AddAnnotation, AnnotKind, Annotation, Color, Document, Point, Quad, Rect};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output) = match args.as_slice() {
        [i, o] => (i.clone(), o.clone()),
        _ => {
            eprintln!("Aufruf: annot_demo <eingabe.pdf> <ausgabe.pdf>");
            std::process::exit(2);
        }
    };
    let mut doc = Document::open(&input)?;
    let quad = |x: f32, y: f32, w: f32| Quad::from_rect(Rect::new(x, y - 3.0, x + w, y + 11.0));

    let add = |doc: &mut Document, kind: AnnotKind, color: Color, text: &str| {
        let mut a = Annotation::new(0, kind, color);
        a.id = doc.allocate_annotation_id();
        a.author = "PDF-Tool-Demo".into();
        a.contents = text.into();
        doc.execute(Box::new(AddAnnotation::new(a)))
            .expect("hinzufügen");
    };

    // Positionen passend zu tests/fixtures/text_two_pages.pdf (Seite 1).
    add(
        &mut doc,
        AnnotKind::Highlight {
            quads: vec![quad(72.0, 720.0, 330.0)],
        },
        Color::YELLOW,
        "Hervorhebung",
    );
    add(
        &mut doc,
        AnnotKind::Underline {
            quads: vec![quad(72.0, 700.0, 345.0)],
        },
        Color::GREEN,
        "Unterstreichung",
    );
    add(
        &mut doc,
        AnnotKind::StrikeOut {
            quads: vec![quad(72.0, 680.0, 210.0)],
        },
        Color::RED,
        "Durchstreichung",
    );
    add(
        &mut doc,
        AnnotKind::Note {
            pos: Point::new(450.0, 735.0),
        },
        Color::YELLOW,
        "Eine Haftnotiz",
    );
    add(
        &mut doc,
        AnnotKind::Ink {
            strokes: vec![
                (0..40)
                    .map(|i| {
                        let t = i as f32 / 39.0;
                        Point::new(80.0 + t * 200.0, 560.0 + (t * 12.0).sin() * 25.0)
                    })
                    .collect(),
            ],
            width: 2.5,
        },
        Color::BLUE,
        "Freihand",
    );
    add(
        &mut doc,
        AnnotKind::Square {
            rect: Rect::new(80.0, 380.0, 230.0, 480.0),
            width: 2.0,
        },
        Color::RED,
        "Rechteck",
    );
    add(
        &mut doc,
        AnnotKind::Circle {
            rect: Rect::new(280.0, 380.0, 450.0, 480.0),
            width: 2.0,
        },
        Color::GREEN,
        "Ellipse",
    );
    add(
        &mut doc,
        AnnotKind::Line {
            start: Point::new(100.0, 300.0),
            end: Point::new(400.0, 340.0),
            width: 2.0,
            arrow_end: true,
        },
        Color::BLUE,
        "Pfeil",
    );
    add(
        &mut doc,
        AnnotKind::FreeText {
            rect: Rect::new(80.0, 150.0, 330.0, 250.0),
            font_size: 14.0,
        },
        Color::rgb(0.6, 0.0, 0.6),
        "Freitext mit Unicode: Grüße aus Köln, Привет, Ωμέγα € – dieser Text ist lang genug für einen Umbruch.",
    );

    doc.save_as(&output)?;
    println!(
        "geschrieben: {output} ({} Annotationen)",
        doc.annotations().len()
    );
    Ok(())
}
