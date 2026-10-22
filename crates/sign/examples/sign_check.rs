//! Signaturen einer PDF prüfen und ausgeben.
//!
//! `cargo run -p foliocraft-sign --example sign_check -- datei.pdf [vertrauenswürdig.pem …]`

use foliocraft_sign::{TrustStore, cert, verify_document};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("Aufruf: sign_check datei.pdf [vertrauenswürdig.pem …]");
        std::process::exit(2);
    };
    let doc = foliocraft_core::Document::open(path).expect("PDF öffnen");
    let anchors: Vec<Vec<u8>> = args[1..]
        .iter()
        .map(|p| std::fs::read(p).expect("Zertifikat lesen"))
        .filter_map(|b| cert::parse_certificate(&b))
        .map(|c| der::Encode::to_der(&c).expect("DER"))
        .collect();
    let trust = TrustStore::from_der_list(anchors.iter().map(Vec::as_slice));
    let reports = verify_document(&doc, &trust);
    if reports.is_empty() {
        println!("Keine Signaturen.");
    }
    for r in reports {
        println!("Feld „{}“: {}", r.field_name, r.summary());
        println!("  Unterzeichner: {}", r.signer);
        println!("  Zeit: {}", r.time.as_deref().unwrap_or("–"));
        println!(
            "  Verfahren: {} / {}",
            r.sub_filter,
            r.hash.map_or("?", |h| h.name())
        );
        println!("  Integrität: {:?}", r.integrity);
        println!("  Vertrauen: {:?}", r.trust);
        println!("  Spätere Revisionen: {}", r.later_revisions);
        for c in &r.chain {
            println!("  Kette: {} (gültig bis {})", c.name, c.not_after);
        }
    }
}
