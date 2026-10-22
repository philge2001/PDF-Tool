//! PDF signieren – zur Gegenprobe mit anderen Programmen.
//!
//! `cargo run -p foliocraft-sign --example sign_demo -- in.pdf out.pdf [id.p12 passwort]`
//! Ohne .p12 wird ein Testzertifikat erzeugt und als `out.pdf.p12` (Passwort "test")
//! sowie `out.pdf.cer` gespeichert.

use foliocraft_core::Rect;
use foliocraft_core::signature_prep::{SigPlacement, SignatureSpec};
use foliocraft_sign::{KeyKind, Signer, sign_pdf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("Aufruf: sign_demo in.pdf out.pdf [id.p12 passwort] [rsa]");
        std::process::exit(2);
    }
    let signer = match args
        .get(2)
        .filter(|a| a.ends_with(".p12") || a.ends_with(".pfx"))
    {
        Some(p12) => {
            let data = std::fs::read(p12).expect("PKCS#12 lesen");
            Signer::from_pkcs12(&data, args.get(3).map_or("", String::as_str))
                .expect("PKCS#12 öffnen")
        }
        None => {
            let kind = if args.iter().any(|a| a == "rsa") {
                KeyKind::Rsa2048
            } else {
                KeyKind::P256
            };
            let s = Signer::self_signed("FolioCraft Test", "FolioCraft", "", kind, 1)
                .expect("Testzertifikat");
            std::fs::write(
                format!("{}.p12", args[1]),
                s.to_pkcs12("test").expect("p12"),
            )
            .expect("schreiben");
            std::fs::write(format!("{}.cer", args[1]), &s.info().der).expect("schreiben");
            s
        }
    };
    let base = std::fs::read(&args[0]).expect("PDF lesen");
    let spec = SignatureSpec {
        placement: SigPlacement::New {
            page: 0,
            rect: Rect::new(320.0, 60.0, 540.0, 120.0),
            name: "FolioCraftSignatur".into(),
        },
        sub_filter: String::new(),
        name: String::new(),
        reason: "Gegenprobe".into(),
        location: "Container".into(),
        contact: String::new(),
        date: foliocraft_core::date::pdf_date_now(),
        appearance_lines: vec![
            format!("Digital signiert von {}", signer.info().name),
            foliocraft_core::date::display_pdf_date(&foliocraft_core::date::pdf_date_now()),
            "Grund: Gegenprobe".into(),
        ],
        contents_len: 0,
    };
    let out = sign_pdf(&base, spec, &signer).expect("signieren");
    std::fs::write(&args[1], out).expect("schreiben");
    println!("signiert: {}", args[1]);
}
