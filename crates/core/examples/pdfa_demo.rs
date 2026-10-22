//! Nach PDF/A-2b umwandeln – zur Gegenprobe mit veraPDF.
//!
//! `cargo run -p foliocraft-core --example pdfa_demo -- in.pdf out.pdf`
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("Aufruf: pdfa_demo in.pdf out.pdf");
        std::process::exit(2);
    }
    let bytes = std::fs::read(&args[0]).expect("lesen");
    let (out, report) = foliocraft_core::pdfa::convert_to_pdfa(&bytes).expect("umwandeln");
    std::fs::write(&args[1], out).expect("schreiben");
    print!("{}", report.summary());
}
