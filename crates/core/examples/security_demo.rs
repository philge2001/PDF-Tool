//! Passwortschutz zur Gegenprobe (qpdf, pdfinfo -upw …).
//!
//! `cargo run -p pdftool-core --example security_demo -- schützen in.pdf out.pdf benutzer besitzer [nur-lesen]`
//! `cargo run -p pdftool-core --example security_demo -- öffnen datei.pdf [passwort]`
use pdftool_core::Document;
use pdftool_core::security::{Permissions, Protection};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("schützen") => {
            let mut doc = Document::open(&a[1]).expect("öffnen");
            let permissions = if a.get(5).is_some() {
                Permissions {
                    print: true,
                    modify: false,
                    copy: false,
                    annotate: false,
                }
            } else {
                Permissions::ALL
            };
            doc.set_protection(Some(&Protection {
                user_password: a[3].clone(),
                owner_password: a[4].clone(),
                permissions,
            }))
            .expect("Schutz");
            doc.save_as(&a[2]).expect("speichern");
        }
        Some("öffnen") => {
            match Document::open_with_password(&a[1], a.get(2).map(String::as_str)) {
                Ok(doc) => {
                    let text = doc
                        .text_blocks(0)
                        .map(|b| b.first().map(|b| b.text.clone()).unwrap_or_default());
                    match doc.security() {
                        Some(s) => println!(
                            "geschützt ({}), Passwort nötig: {}, Besitzer: {}, erlaubt: {} – Text: {:?}",
                            s.method,
                            s.needs_password,
                            s.owner_known,
                            s.permissions.describe(),
                            text
                        ),
                        None => println!("ungeschützt – Text: {text:?}"),
                    }
                }
                Err(e) => println!("Fehler: {e}"),
            }
        }
        _ => eprintln!("Aufruf: security_demo schützen|öffnen …"),
    }
}
