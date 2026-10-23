//! PDF signieren – zur Gegenprobe mit anderen Programmen.
//!
//! `cargo run -p pdftool-sign --example sign_demo -- in.pdf out.pdf [id.p12 passwort]`
//! Ohne .p12 wird ein Testzertifikat erzeugt und als `out.pdf.p12` (Passwort "test")
//! sowie `out.pdf.cer` gespeichert.
//! `--tsa-openssl tsa.cnf` holt einen Zeitstempel bei einem lokalen OpenSSL-Zeitstempeldienst
//! (`openssl ts -reply`), `--tsa-http URL` per `curl` bei einem echten Dienst.

use pdftool_core::Rect;
use pdftool_core::signature_prep::{SigPlacement, SignatureSpec};
use pdftool_sign::{KeyKind, Signer, sign_pdf};

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
            let s = Signer::self_signed("PDF Tool Test", "PDF Tool", "", kind, 1)
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
            name: "PdfToolSignatur".into(),
        },
        sub_filter: String::new(),
        name: String::new(),
        reason: "Gegenprobe".into(),
        location: "Container".into(),
        contact: String::new(),
        date: pdftool_core::date::pdf_date_now(),
        appearance_lines: vec![
            format!("Digital signiert von {}", signer.info().name),
            pdftool_core::date::display_pdf_date(&pdftool_core::date::pdf_date_now()),
            "Grund: Gegenprobe".into(),
        ],
        contents_len: 0,
    };
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    // Zeitstempel über externe Programme – die Crate selbst macht kein Netzwerk.
    let tsa_openssl = flag("--tsa-openssl").map(|cnf| {
        move |req: &[u8]| {
            run_tsa(req, |q, a| {
                let mut c = std::process::Command::new("openssl");
                c.args(["ts", "-reply", "-config", &cnf, "-queryfile"])
                    .arg(q)
                    .arg("-out")
                    .arg(a);
                c
            })
        }
    });
    let tsa_http = flag("--tsa-http").map(|url| {
        move |req: &[u8]| {
            run_tsa(req, |q, a| {
                let mut c = std::process::Command::new("curl");
                c.args([
                    "-s",
                    "-H",
                    "Content-Type: application/timestamp-query",
                    "--data-binary",
                ])
                .arg(format!("@{}", q.display()))
                .arg("-o")
                .arg(a)
                .arg(&url);
                c
            })
        }
    });
    let tsa: Option<pdftool_sign::timestamp::TsaTransport> = match (&tsa_openssl, &tsa_http) {
        (Some(f), _) => Some(f),
        (_, Some(f)) => Some(f),
        _ => None,
    };
    let out = sign_pdf(&base, spec, &signer, tsa).expect("signieren");
    std::fs::write(&args[1], out).expect("schreiben");
    println!("signiert: {}", args[1]);
}

/// Schreibt die Anfrage in eine Datei, ruft das Programm auf und liest die Antwort.
fn run_tsa(
    req: &[u8],
    command: impl FnOnce(&std::path::Path, &std::path::Path) -> std::process::Command,
) -> Result<Vec<u8>, String> {
    let dir = std::env::temp_dir();
    let (q, a) = (dir.join("pdftool-demo.tsq"), dir.join("pdftool-demo.tsr"));
    std::fs::write(&q, req).map_err(|e| e.to_string())?;
    let status = command(&q, &a).status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Zeitstempeldienst-Aufruf fehlgeschlagen".into());
    }
    std::fs::read(&a).map_err(|e| e.to_string())
}
