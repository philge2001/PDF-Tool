//! Barrierefreiheit prüfen und automatisch taggen.
//!
//! `cargo run -p pdftool-core --example autotag_demo -- in.pdf [out.pdf] [sprache] [titel]`
//! Ohne `out.pdf` wird nur geprüft.

use pdftool_core::Document;
use pdftool_core::accessibility::CheckStatus;
use pdftool_core::autotag::{AutoTagOptions, auto_tag_pdf};

fn print_check(doc: &Document) -> Result<(), Box<dyn std::error::Error>> {
    for c in doc.accessibility_check()? {
        let s = match c.status {
            CheckStatus::Pass => "OK     ",
            CheckStatus::Warn => "HINWEIS",
            CheckStatus::Fail => "FEHLER ",
        };
        println!("{s} {}: {}", c.title, c.detail);
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = args.first() else {
        eprintln!("Aufruf: autotag_demo in.pdf [out.pdf] [sprache] [titel]");
        std::process::exit(2);
    };
    let doc = Document::open(input)?;
    println!("--- Prüfung {input}");
    print_check(&doc)?;
    let Some(output) = args.get(1) else {
        return Ok(());
    };
    let opts = AutoTagOptions {
        lang: args.get(2).cloned().unwrap_or_else(|| "de-DE".into()),
        title: args.get(3).cloned().unwrap_or_default(),
        embed_fonts: true,
    };
    let (bytes, report) = auto_tag_pdf(&std::fs::read(input)?, &opts, &|_, _| true)?;
    std::fs::write(output, &bytes)?;
    println!("--- {}", report.summary());
    println!("--- Prüfung {output}");
    print_check(&Document::open(output)?)?;
    Ok(())
}
