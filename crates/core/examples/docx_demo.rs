//! Word-Export zur Gegenprobe (z. B. mit LibreOffice oder python-docx).
//!
//! `cargo run -p foliocraft-core --example docx_demo -- in.pdf out.docx`
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("Aufruf: docx_demo in.pdf out.docx");
        std::process::exit(2);
    }
    let doc = foliocraft_core::Document::open(&args[0]).expect("PDF öffnen");
    let pages: Vec<usize> = (0..doc.page_count()).collect();
    let bytes =
        foliocraft_core::docx::export_docx(&doc, &pages, &|_, _| true).expect("exportieren");
    std::fs::write(&args[1], bytes).expect("schreiben");
    println!("geschrieben: {}", args[1]);
}
