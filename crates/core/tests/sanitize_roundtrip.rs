//! Bereinigen: finden -> entfernen -> nichts davon darf mehr in der Datei stehen.

use foliocraft_core::sanitize::{SanitizeOptions, inspect, sanitize_pdf};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Rohbytes, entpackte Streams und Objekte als Text.
fn everything(bytes: &[u8]) -> Vec<u8> {
    let mut all = bytes.to_vec();
    let pdf = lopdf::Document::load_mem(bytes).unwrap();
    for obj in pdf.objects.values() {
        if let Ok(s) = obj.as_stream() {
            all.extend(
                s.decompressed_content()
                    .unwrap_or_else(|_| s.content.clone()),
            );
        }
        all.extend(format!("{obj:?}").into_bytes());
    }
    all.extend(format!("{:?}", pdf.trailer).into_bytes());
    all
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn finds_everything() {
    let bytes = std::fs::read(fixture("sanitize.pdf")).unwrap();
    let f = inspect(&bytes).unwrap();
    assert!(f.metadata >= 2, "Info + XMP: {f:?}");
    assert!(f.javascript >= 1);
    assert_eq!(f.embedded_files, 1);
    assert_eq!(f.hidden_layers, 1);
    assert_eq!(f.comments, 1);
    assert_eq!(f.form_values, 1);
}

#[test]
fn removes_everything_selected() {
    let bytes = std::fs::read(fixture("sanitize.pdf")).unwrap();
    let all = everything(&bytes);
    for secret in [
        "Erika Interna",
        "app.alert",
        "4711 EUR",
        "Geheime Ebene",
        "Interner Kommentar",
        "Geheimkunde",
    ] {
        assert!(contains(&all, secret), "Fixture enthält {secret}");
    }
    let opts = SanitizeOptions {
        metadata: true,
        javascript: true,
        embedded_files: true,
        hidden_layers: true,
        comments: true,
        form_data: true,
    };
    let (out, done) = sanitize_pdf(&bytes, opts).unwrap();
    assert!(done.len() >= 6, "{done:?}");
    let all = everything(&out);
    for secret in [
        "Erika Interna",
        "Vertraulich",
        "app.alert",
        "4711 EUR",
        "Geheime Ebene",
        "Interner Kommentar",
        "Geheimkunde",
    ] {
        assert!(!contains(&all, secret), "{secret} steht noch in der Datei");
    }
    assert!(contains(&all, "Sichtbarer Text"), "Sichtbares bleibt");
    let f = inspect(&out).unwrap();
    assert_eq!(f, Default::default(), "nichts mehr zu finden");
    // Das Formularfeld selbst bleibt, nur ohne Wert.
    let doc = foliocraft_core::Document::from_bytes(out).unwrap();
    assert!(doc.forms().by_name("kunde").is_some());
}

#[test]
fn keeps_what_was_not_selected() {
    let bytes = std::fs::read(fixture("sanitize.pdf")).unwrap();
    let opts = SanitizeOptions {
        metadata: true,
        javascript: false,
        embedded_files: false,
        hidden_layers: false,
        comments: false,
        form_data: false,
    };
    let (out, _) = sanitize_pdf(&bytes, opts).unwrap();
    let all = everything(&out);
    assert!(!contains(&all, "Erika Interna"));
    assert!(contains(&all, "app.alert"));
    assert!(contains(&all, "4711 EUR"));
    assert!(contains(&all, "Interner Kommentar"));
}
