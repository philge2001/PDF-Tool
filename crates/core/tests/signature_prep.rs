//! Signatur-Revision mit Platzhalter: vorbereiten -> Fake-Signatur einsetzen -> neu laden.

use foliocraft_core::signature_prep::{
    SigPlacement, SignatureSpec, empty_signature_fields, prepare_signature,
};
use foliocraft_core::{Document, Rect};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn spec(placement: SigPlacement) -> SignatureSpec {
    SignatureSpec {
        placement,
        sub_filter: "ETSI.CAdES.detached".into(),
        name: "Erika Musterfrau".into(),
        reason: "Freigabe".into(),
        location: "Köln".into(),
        contact: String::new(),
        date: "D:20261022120000Z".into(),
        appearance_lines: vec!["Digital signiert von Erika".into(), "22.10.2026".into()],
        contents_len: 4096,
    }
}

/// Liest das Signatur-Dictionary des Felds `name`.
fn sig_dict(doc: &Document, name: &str) -> lopdf::Dictionary {
    let f = doc.forms().by_name(name).unwrap();
    let pdf = doc.pdf();
    let v = pdf
        .get_dictionary(f.source.unwrap())
        .unwrap()
        .get(b"V")
        .unwrap();
    pdf.dereference(v).unwrap().1.as_dict().unwrap().clone()
}

fn check(base: &[u8], placement: SigPlacement, field: &str) {
    let prepared = prepare_signature(base, &spec(placement)).unwrap();
    let range = prepared.byte_range();
    let [p1, p2] = prepared.signed_parts();
    assert_eq!(p1.len(), range[1]);
    assert_eq!(p2.len(), range[3]);
    assert!(p1.starts_with(base), "Originalbytes bleiben unverändert");
    let fake = [0x30, 0x03, 0x02, 0x01, 0x05];
    let bytes = prepared.finish(&fake).unwrap();
    assert_eq!(bytes.len(), range[2] + range[3]);

    let doc = Document::from_bytes(bytes).unwrap();
    let d = sig_dict(&doc, field);
    let br: Vec<usize> = d
        .get(b"ByteRange")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_i64().unwrap() as usize)
        .collect();
    assert_eq!(br, range.to_vec());
    let contents = d.get(b"Contents").unwrap().as_str().unwrap();
    assert_eq!(&contents[..fake.len()], &fake);
    assert_eq!(contents.len(), 4096);
    assert_eq!(
        d.get(b"SubFilter").unwrap().as_name().unwrap(),
        b"ETSI.CAdES.detached"
    );
    assert!(empty_signature_fields(&doc).iter().all(|n| n != field));
}

#[test]
fn sign_into_existing_field() {
    let base = std::fs::read(fixture("form.pdf")).unwrap();
    let doc = Document::from_bytes(base.clone()).unwrap();
    assert_eq!(
        empty_signature_fields(&doc),
        vec!["unterschrift".to_string()]
    );
    check(
        &base,
        SigPlacement::Existing("unterschrift".into()),
        "unterschrift",
    );
}

#[test]
fn new_visible_and_invisible_fields() {
    let base = std::fs::read(fixture("text_two_pages.pdf")).unwrap();
    let visible = SigPlacement::New {
        page: 1,
        rect: Rect::new(300.0, 100.0, 500.0, 160.0),
        name: "Signatur1".into(),
    };
    check(&base, visible, "Signatur1");
    let invisible = SigPlacement::New {
        page: 0,
        rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        name: "Signatur1".into(),
    };
    check(&base, invisible, "Signatur1");
}

#[test]
fn too_large_signature_is_rejected() {
    let base = std::fs::read(fixture("text_two_pages.pdf")).unwrap();
    let mut s = spec(SigPlacement::New {
        page: 0,
        rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        name: "S".into(),
    });
    s.contents_len = 8;
    let prepared = prepare_signature(&base, &s).unwrap();
    assert!(prepared.finish(&[0u8; 9]).is_err());
}
