//! Text bearbeiten und hinzufügen – zur Gegenprobe mit anderen Readern.
//!
//! `cargo run -p foliocraft-core --example text_edit_demo -- in.pdf out.pdf "alter Anfang=neuer Text" …`
//! ersetzt auf Seite 1 jeden Textblock, der mit "alter Anfang" beginnt.
//! `+x,y=Text` fügt neuen Text an (x, y) hinzu (Punkt, Ursprung unten links).
//! Ohne Ersetzungen werden die Textblöcke von Seite 1 aufgelistet.

use foliocraft_core::{Color, Document, Point};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = args.first() else {
        eprintln!("Aufruf: text_edit_demo in.pdf [out.pdf \"Anfang=Neu\" \"+x,y=Text\" …]");
        std::process::exit(2);
    };
    let mut doc = Document::open(input).expect("PDF öffnen");
    if args.len() < 3 {
        for b in doc.text_blocks(0).expect("Textblöcke") {
            println!(
                "{:>7.2} {:>7.2}  {:>5.1} pt  {}  {:?}",
                b.origin.x,
                b.origin.y,
                b.size_on_page,
                String::from_utf8_lossy(&b.font),
                b.text
            );
        }
        return;
    }
    for spec in &args[2..] {
        let (what, new) = spec.split_once('=').expect("Format Anfang=Neu");
        if let Some(pos) = what.strip_prefix('+') {
            let (x, y) = pos.split_once(',').expect("Format +x,y=Text");
            let p = Point::new(x.parse().expect("x"), y.parse().expect("y"));
            doc.add_text(0, p, &new.replace("\\n", "\n"), 14.0, Color::BLUE)
                .expect("hinzufügen");
            continue;
        }
        let blocks = doc.text_blocks(0).expect("Textblöcke");
        let block = blocks
            .iter()
            .find(|b| b.text.starts_with(what))
            .unwrap_or_else(|| panic!("kein Block beginnt mit {what:?}"));
        doc.edit_text_block(0, block, new).expect("bearbeiten");
    }
    doc.save_as(&args[1]).expect("speichern");
    println!("geschrieben: {}", args[1]);
}
