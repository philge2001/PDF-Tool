//! Bereiche schwärzen – zur Gegenprobe mit anderen Readern.
//!
//! `cargo run -p pdftool-core --example redact_demo -- in.pdf out.pdf SEITE:x0,y0,x1,y1 …`
//! (Seite 1-basiert, Koordinaten in Punkt, Ursprung unten links)

use pdftool_core::{AddAnnotation, AnnotKind, Annotation, Color, Document, Quad, Rect};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("Aufruf: redact_demo in.pdf out.pdf SEITE:x0,y0,x1,y1 …");
        std::process::exit(2);
    }
    let mut doc = Document::open(&args[0]).expect("PDF öffnen");
    for spec in &args[2..] {
        let (page, coords) = spec.split_once(':').expect("Format SEITE:x0,y0,x1,y1");
        let v: Vec<f32> = coords
            .split(',')
            .map(|c| c.trim().parse().expect("Zahl"))
            .collect();
        let page: usize = page.parse::<usize>().expect("Seite") - 1;
        let quad = Quad::from_rect(Rect::new(v[0], v[1], v[2], v[3]).normalized());
        let mut a = Annotation::new(page, AnnotKind::Redact { quads: vec![quad] }, Color::RED);
        a.id = doc.allocate_annotation_id();
        doc.execute(Box::new(AddAnnotation::new(a)))
            .expect("markieren");
    }
    let (bytes, areas) = doc.redaction_input().expect("vorbereiten");
    let (out, report) = pdftool_core::redact::redact_pdf(&bytes, &areas).expect("schwärzen");
    std::fs::write(&args[1], out).expect("schreiben");
    println!("{}", report.summary());
}
