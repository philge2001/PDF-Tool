//! PDF komprimieren – zur Gegenprobe (`pdfimages -list`, `pdftoppm`, `qpdf --check`).
//!
//! `cargo run -p foliocraft-core --example compress_demo -- in.pdf out.pdf [dpi] [qualität]`
use foliocraft_core::compress::{CompressOptions, compress_pdf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("Aufruf: compress_demo in.pdf out.pdf [dpi|0] [qualität]");
        std::process::exit(2);
    }
    let bytes = std::fs::read(&args[0]).expect("lesen");
    let dpi = args.get(2).and_then(|d| d.parse().ok()).unwrap_or(150);
    let options = CompressOptions {
        image_dpi: (dpi > 0).then_some(dpi),
        jpeg_quality: args.get(3).and_then(|q| q.parse().ok()).unwrap_or(75),
    };
    let (out, report) = compress_pdf(&bytes, &options, &|_, _| true).expect("komprimieren");
    std::fs::write(&args[1], out).expect("schreiben");
    println!("{}", report.summary());
}
