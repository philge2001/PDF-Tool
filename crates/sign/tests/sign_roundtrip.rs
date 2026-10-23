//! Signieren -> Prüfen: selbst ausgestellt, mit Zertifizierungsstelle, manipuliert,
//! nachträglich geändert und doppelt signiert.

use der::Encode;
use pdftool_core::signature_prep::{SigPlacement, SignatureSpec};
use pdftool_core::{Document, FieldValue, Rect, UpdateField};
use pdftool_sign::signer::PrivateKey;
use pdftool_sign::{
    Integrity, KeyKind, Signer, Status, Trust, TrustStore, sign_pdf, verify_document,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::Validity;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap()
}

fn spec(placement: SigPlacement) -> SignatureSpec {
    SignatureSpec {
        placement,
        sub_filter: String::new(),
        name: String::new(),
        reason: "Freigabe".into(),
        location: "Köln".into(),
        contact: String::new(),
        date: pdftool_core::date::pdf_date_now(),
        appearance_lines: vec!["Digital signiert".into()],
        contents_len: 0,
    }
}

fn invisible(name: &str) -> SigPlacement {
    SigPlacement::New {
        page: 0,
        rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        name: name.into(),
    }
}

fn trust_of(certs: &[&x509_cert::Certificate]) -> TrustStore {
    let ders: Vec<Vec<u8>> = certs.iter().map(|c| c.to_der().unwrap()).collect();
    TrustStore::from_der_list(ders.iter().map(Vec::as_slice))
}

fn verify(bytes: Vec<u8>, trust: &TrustStore) -> Vec<pdftool_sign::SignatureReport> {
    verify_document(&Document::from_bytes(bytes).unwrap(), trust)
}

#[test]
fn self_signed_p256_into_existing_field() {
    let signer = Signer::self_signed("Erika Musterfrau", "", "", KeyKind::P256, 2).unwrap();
    let signed = sign_pdf(
        &read("form.pdf"),
        spec(SigPlacement::Existing("unterschrift".into())),
        &signer,
        None,
    )
    .unwrap();

    let reports = verify(signed.clone(), &TrustStore::default());
    assert_eq!(reports.len(), 1);
    let r = &reports[0];
    assert_eq!(r.field_name, "unterschrift");
    assert_eq!(r.integrity, Integrity::Intact);
    assert!(matches!(r.trust, Trust::Unknown(_)));
    assert_eq!(r.status(), Status::UnknownSigner);
    assert_eq!(r.signer, "Erika Musterfrau");
    assert_eq!(r.reason, "Freigabe");
    assert_eq!(r.sub_filter, "ETSI.CAdES.detached");
    assert_eq!(r.later_revisions, 0);

    let trusted = verify(signed, &trust_of(&[&signer.cert]));
    assert_eq!(trusted[0].status(), Status::Valid);
}

#[test]
fn rsa_signature_in_new_visible_field() {
    let signer = Signer::self_signed("Max RSA", "Test GmbH", "", KeyKind::Rsa2048, 1).unwrap();
    let placement = SigPlacement::New {
        page: 1,
        rect: Rect::new(300.0, 100.0, 520.0, 160.0),
        name: "Signatur1".into(),
    };
    let signed = sign_pdf(&read("text_two_pages.pdf"), spec(placement), &signer, None).unwrap();
    let r = &verify(signed, &trust_of(&[&signer.cert]))[0];
    assert_eq!(r.status(), Status::Valid, "{:?} {:?}", r.integrity, r.trust);
    assert_eq!(r.page, Some(1));
}

/// Stammzertifikat (CA) und davon ausgestelltes Unterzeichnerzertifikat.
fn ca_and_leaf() -> (x509_cert::Certificate, Signer) {
    let validity = Validity::from_now(Duration::from_secs(86_400 * 30)).unwrap();
    let root_key = p256::ecdsa::SigningKey::random(&mut rand_core::OsRng);
    let root_name = Name::from_str("CN=Test Root CA,O=PDF Tool Test").unwrap();
    let spki = |k: &p256::ecdsa::SigningKey| {
        use p256::pkcs8::EncodePublicKey;
        spki::SubjectPublicKeyInfoOwned::try_from(
            k.verifying_key().to_public_key_der().unwrap().as_bytes(),
        )
        .unwrap()
    };
    let root = CertificateBuilder::new(
        Profile::Root,
        SerialNumber::from(1u32),
        validity,
        root_name.clone(),
        spki(&root_key),
        &root_key,
    )
    .unwrap()
    .build::<p256::ecdsa::DerSignature>()
    .unwrap();
    let leaf_key = p256::ecdsa::SigningKey::random(&mut rand_core::OsRng);
    let leaf = CertificateBuilder::new(
        Profile::Leaf {
            issuer: root_name,
            enable_key_agreement: false,
            enable_key_encipherment: false,
        },
        SerialNumber::from(2u32),
        validity,
        Name::from_str("CN=Klara Kette").unwrap(),
        spki(&leaf_key),
        &root_key,
    )
    .unwrap()
    .build::<p256::ecdsa::DerSignature>()
    .unwrap();
    let signer = Signer {
        key: PrivateKey::P256(leaf_key),
        cert: leaf,
        chain: vec![root.clone()],
    };
    (root, signer)
}

#[test]
fn chain_to_trusted_root() {
    let (root, signer) = ca_and_leaf();
    let signed = sign_pdf(
        &read("text_two_pages.pdf"),
        spec(invisible("S")),
        &signer,
        None,
    )
    .unwrap();

    let r = &verify(signed.clone(), &TrustStore::default())[0];
    assert_eq!(r.chain.len(), 2, "Kette aus der Signatur gebildet");
    match &r.trust {
        Trust::Unknown(msg) => assert!(msg.contains("Test Root CA"), "{msg}"),
        t => panic!("{t:?}"),
    }
    let r = &verify(signed, &trust_of(&[&root]))[0];
    assert_eq!(r.status(), Status::Valid);
    assert_eq!(r.signer, "Klara Kette");
}

#[test]
fn signature_survives_pkcs12_roundtrip_with_chain() {
    let (root, signer) = ca_and_leaf();
    let p12 = signer.to_pkcs12("pw").unwrap();
    let loaded = Signer::from_pkcs12(&p12, "pw").unwrap();
    assert_eq!(loaded.cert, signer.cert);
    assert_eq!(loaded.chain, vec![root.clone()]);
    let signed = sign_pdf(&read("form.pdf"), spec(invisible("S")), &loaded, None).unwrap();
    assert_eq!(
        verify(signed, &trust_of(&[&root]))[0].status(),
        Status::Valid
    );
}

#[test]
fn tampering_breaks_the_signature() {
    let signer = Signer::self_signed("T", "", "", KeyKind::P256, 1).unwrap();
    let mut signed = sign_pdf(&read("form.pdf"), spec(invisible("S")), &signer, None).unwrap();
    // Ein Byte im signierten Bereich ändern: die Versionsnummer im Dateikopf ("%PDF-1.x").
    assert!(signed.starts_with(b"%PDF-1."));
    signed[7] = if signed[7] == b'4' { b'5' } else { b'4' };
    let r = &verify(signed, &trust_of(&[&signer.cert]))[0];
    assert!(
        matches!(r.integrity, Integrity::Broken(_)),
        "{:?}",
        r.integrity
    );
    assert_eq!(r.status(), Status::Invalid);
}

#[test]
fn filling_after_signing_keeps_signature_but_reports_change() {
    let signer = Signer::self_signed("T", "", "", KeyKind::P256, 1).unwrap();
    let signed = sign_pdf(
        &read("form.pdf"),
        spec(SigPlacement::Existing("unterschrift".into())),
        &signer,
        None,
    )
    .unwrap();
    let mut doc = Document::from_bytes(signed).unwrap();
    let f = doc.forms().by_name("pflicht").unwrap().clone();
    doc.execute(Box::new(UpdateField::value(
        &f,
        FieldValue::Text("später".into()),
    )))
    .unwrap();
    let r = &verify(doc.to_bytes().unwrap(), &trust_of(&[&signer.cert]))[0];
    assert_eq!(r.integrity, Integrity::Intact);
    assert_eq!(r.later_revisions, 1);
    assert_eq!(r.summary(), "Gültig – danach geändert");
}

#[test]
fn second_signature_keeps_first_valid() {
    let a = Signer::self_signed("Erste", "", "", KeyKind::P256, 1).unwrap();
    let b = Signer::self_signed("Zweiter", "", "", KeyKind::P256, 1).unwrap();
    let once = sign_pdf(
        &read("form.pdf"),
        spec(SigPlacement::Existing("unterschrift".into())),
        &a,
        None,
    )
    .unwrap();
    let twice = sign_pdf(&once, spec(invisible("Gegenzeichnung")), &b, None).unwrap();
    let trust = trust_of(&[&a.cert, &b.cert]);
    let reports = verify(twice, &trust);
    assert_eq!(reports.len(), 2);
    let first = reports.iter().find(|r| r.signer == "Erste").unwrap();
    let second = reports.iter().find(|r| r.signer == "Zweiter").unwrap();
    assert_eq!(first.integrity, Integrity::Intact);
    assert_eq!(first.later_revisions, 1);
    assert_eq!(second.later_revisions, 0);
    assert_eq!(second.status(), Status::Valid);
    // Das schon signierte Feld kann nicht ein zweites Mal signiert werden.
    let err = sign_pdf(
        &read("form.pdf"),
        spec(SigPlacement::Existing("person.name".into())),
        &a,
        None,
    );
    assert!(err.is_err());
}
