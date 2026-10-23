//! Setzt den Absatz "Lorem…" der Test-PDF formatiert neu (fett, kursiv, unterstrichen,
//! durchgestrichen, andere Größe/Farbe/Schrift, Kyrillisch aus der Systemschrift).
//!
//! `cargo run -p pdftool-core --example rich_text_demo -- in.pdf out.pdf [Familie]`

use pdftool_core::rich_text::{FontRef, StyledRun, TextStyle};
use pdftool_core::{Color, Document};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(input), Some(output)) = (args.first(), args.get(1)) else {
        eprintln!("Aufruf: rich_text_demo in.pdf out.pdf [Schriftfamilie]");
        std::process::exit(2);
    };
    let mut doc = Document::open(input)?;
    let para = doc
        .paragraphs(0)?
        .into_iter()
        .max_by_key(|p| p.text.len())
        .ok_or("keine Absätze auf Seite 1")?;
    let base = doc.paragraph_runs(0, &para)?[0].style.clone();
    let family = args.get(2).cloned();
    let s = |f: &dyn Fn(&mut TextStyle)| {
        let mut st = base.clone();
        f(&mut st);
        st
    };
    let runs = vec![
        StyledRun {
            text: "Normal, ".into(),
            style: base.clone(),
        },
        StyledRun {
            text: "fett".into(),
            style: s(&|t| t.bold = true),
        },
        StyledRun {
            text: ", ".into(),
            style: base.clone(),
        },
        StyledRun {
            text: "kursiv".into(),
            style: s(&|t| t.italic = true),
        },
        StyledRun {
            text: ", ".into(),
            style: base.clone(),
        },
        StyledRun {
            text: "unterstrichen".into(),
            style: s(&|t| t.underline = true),
        },
        StyledRun {
            text: ", ".into(),
            style: base.clone(),
        },
        StyledRun {
            text: "durchgestrichen".into(),
            style: s(&|t| t.strike = true),
        },
        StyledRun {
            text: ", ".into(),
            style: base.clone(),
        },
        StyledRun {
            text: "groß und rot".into(),
            style: s(&|t| {
                t.size *= 1.6;
                t.color = Color::rgb(0.85, 0.1, 0.1);
            }),
        },
        StyledRun {
            text: ", Привет Ω ".into(),
            style: base.clone(),
        },
        StyledRun {
            text: "in anderer Schrift".into(),
            style: s(&|t| {
                t.font = match &family {
                    Some(f) => FontRef::Family(f.clone()),
                    None => FontRef::Noto,
                }
            }),
        },
        StyledRun {
            text: " – Ende.".into(),
            style: base.clone(),
        },
    ];
    doc.set_paragraph_rich(
        0,
        &para,
        &runs,
        pdftool_core::ParaFrame {
            width: Some(380.0),
            ..para.frame()
        },
    )?;
    doc.save_as(output)?;
    println!("geschrieben: {output}");
    Ok(())
}
