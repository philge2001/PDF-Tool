//! Dokument bereinigen – zur Gegenprobe mit anderen Programmen.
//!
//! `cargo run -p pdftool-core --example sanitize_demo -- in.pdf [out.pdf]`
//! Ohne out.pdf wird nur angezeigt, was gefunden wurde.

use pdftool_core::sanitize::{SanitizeOptions, inspect, sanitize_pdf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bytes = std::fs::read(&args[0]).expect("PDF lesen");
    println!("{:?}", inspect(&bytes).expect("untersuchen"));
    if let Some(out) = args.get(1) {
        let opts = SanitizeOptions {
            comments: true,
            form_data: true,
            ..Default::default()
        };
        let (clean, done) = sanitize_pdf(&bytes, opts).expect("bereinigen");
        std::fs::write(out, clean).expect("schreiben");
        for d in done {
            println!("- {d}");
        }
    }
}
