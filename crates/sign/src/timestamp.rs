//! Zeitstempel nach RFC 3161 (für PAdES B-T).
//!
//! Ein Zeitstempeldienst (TSA) bekommt den Hash der Signatur und antwortet mit einem
//! signierten "TSTInfo": Hash + Uhrzeit. Dieses Token kommt als nicht signiertes Attribut
//! (id-aa-signatureTimeStampToken) an den SignerInfo. Damit ist unabhängig belegt, dass die
//! Signatur spätestens zu diesem Zeitpunkt existierte.
//!
//! Netzwerk gibt es hier bewusst nicht: Die HTTP-Übertragung reicht der Aufrufer als
//! Funktion herein ([`TsaTransport`]). So bleibt die Crate ohne Netzwerk testbar.

use crate::cert::{self, Trust, TrustStore};
use crate::crypto::{self, Hash};
use cms::cert::CertificateChoices;
use cms::content_info::ContentInfo;
use cms::signed_data::SignedData;
use der::asn1::{Any, GeneralizedTime, ObjectIdentifier, OctetString, OctetStringRef};
use der::{Decode, Encode, Reader, Sequence, SliceReader, Tag, Tagged};
use rand_core::{OsRng, RngCore};
use spki::AlgorithmIdentifierOwned;
use std::time::SystemTime;
use x509_cert::Certificate;

/// id-aa-signatureTimeStampToken
pub const OID_TIMESTAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");
/// id-ct-TSTInfo (Inhaltstyp des Tokens)
pub const OID_TST_INFO: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");
const OID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");

/// Überträgt eine Anfrage (DER) an den Zeitstempeldienst und liefert die Antwort (DER).
/// `dyn Fn` = "irgendeine Funktion mit dieser Signatur" – z. B. ein HTTP-Aufruf in der App.
pub type TsaTransport<'a> = &'a dyn Fn(&[u8]) -> Result<Vec<u8>, String>;

/// MessageImprint ::= SEQUENCE { hashAlgorithm, hashedMessage }
#[derive(Clone, Debug, Sequence)]
pub(crate) struct MessageImprint {
    pub hash_algorithm: AlgorithmIdentifierOwned,
    pub hashed_message: OctetString,
}

/// TimeStampReq ::= SEQUENCE { version, messageImprint, reqPolicy?, nonce?, certReq }
#[derive(Clone, Debug, Sequence)]
struct TimeStampReq {
    version: u8,
    message_imprint: MessageImprint,
    #[asn1(optional = "true")]
    nonce: Option<u64>,
    /// TSA soll ihr Zertifikat mitschicken (sonst können Prüfer es nicht finden).
    cert_req: bool,
}

/// PKIStatusInfo ::= SEQUENCE { status, statusString?, failInfo? }
#[derive(Clone, Debug, Sequence)]
struct PkiStatusInfo {
    status: u8,
    #[asn1(optional = "true")]
    status_string: Option<Vec<String>>,
    #[asn1(optional = "true")]
    fail_info: Option<der::asn1::BitString>,
}

/// TimeStampResp ::= SEQUENCE { status, timeStampToken? }
#[derive(Clone, Debug, Sequence)]
struct TimeStampResp {
    status: PkiStatusInfo,
    #[asn1(optional = "true")]
    token: Option<ContentInfo>,
}

/// Holt einen Zeitstempel für `signature` (den Signaturwert aus dem SignerInfo).
/// Liefert das Token (ContentInfo) als `Any`, bereit für das unsignierte Attribut.
pub fn request(signature: &[u8], transport: TsaTransport) -> Result<Any, String> {
    let hash = Hash::Sha256;
    let imprint = hash.digest([signature]);
    let nonce = OsRng.next_u64() >> 1; // positiv halten
    let req = TimeStampReq {
        version: 1,
        message_imprint: MessageImprint {
            hash_algorithm: AlgorithmIdentifierOwned {
                oid: hash.oid(),
                parameters: None,
            },
            hashed_message: OctetString::new(imprint.clone()).map_err(|e| e.to_string())?,
        },
        nonce: Some(nonce),
        cert_req: true,
    };
    let answer = transport(&req.to_der().map_err(|e| e.to_string())?)?;
    let resp = TimeStampResp::from_der(&answer)
        .map_err(|e| format!("Antwort des Zeitstempeldienstes unlesbar: {e}"))?;
    // 0 = erteilt, 1 = erteilt mit Änderungen; alles andere ist eine Ablehnung.
    if resp.status.status > 1 {
        let text = resp.status.status_string.unwrap_or_default().join("; ");
        return Err(format!(
            "Zeitstempeldienst hat abgelehnt (Status {}){}",
            resp.status.status,
            if text.is_empty() {
                String::new()
            } else {
                format!(": {text}")
            }
        ));
    }
    let token = resp
        .token
        .ok_or("Zeitstempeldienst hat kein Token geliefert")?;
    // Gleich prüfen: richtiger Hash und unsere Nonce (schützt vor vertauschten Antworten).
    let tst = token_tst_info(&token)?;
    if tst.imprint != imprint {
        return Err("Zeitstempel gehört nicht zu dieser Signatur".into());
    }
    if tst.nonce.is_some_and(|n| n != nonce) {
        return Err("Zeitstempel: Nonce stimmt nicht".into());
    }
    let der = token.to_der().map_err(|e| e.to_string())?;
    Any::from_der(&der).map_err(|e| e.to_string())
}

/// Die für uns wichtigen Felder aus TSTInfo.
struct TstFields {
    imprint: Vec<u8>,
    imprint_hash: Option<Hash>,
    time: GeneralizedTime,
    nonce: Option<u64>,
    /// DER des TSTInfo (das ist der signierte Inhalt des Tokens).
    raw: Vec<u8>,
}

fn token_signed_data(token: &ContentInfo) -> Result<SignedData, String> {
    token
        .content
        .decode_as::<SignedData>()
        .map_err(|e| format!("Zeitstempel unlesbar: {e}"))
}

fn token_tst_info(token: &ContentInfo) -> Result<TstFields, String> {
    let sd = token_signed_data(token)?;
    if sd.encap_content_info.econtent_type != OID_TST_INFO {
        return Err("Kein RFC-3161-Zeitstempel".into());
    }
    let raw = sd
        .encap_content_info
        .econtent
        .as_ref()
        .and_then(|e| e.decode_as::<OctetStringRef>().ok())
        .map(|o| o.as_bytes().to_vec())
        .ok_or("Zeitstempel ohne Inhalt")?;
    parse_tst_info(&raw)
}

/// TSTInfo ::= SEQUENCE { version, policy, messageImprint, serialNumber, genTime,
/// accuracy?, ordering?, nonce?, tsa [0]?, extensions [1]? }
/// Die optionalen Felder werden einzeln am Tag erkannt.
fn parse_tst_info(raw: &[u8]) -> Result<TstFields, String> {
    let err = |e: der::Error| format!("TSTInfo unlesbar: {e}");
    let seq = Any::from_der(raw).map_err(err)?;
    let mut r = SliceReader::new(seq.value()).map_err(err)?;
    let _version = u8::decode(&mut r).map_err(err)?;
    let _policy = ObjectIdentifier::decode(&mut r).map_err(err)?;
    let imprint = MessageImprint::decode(&mut r).map_err(err)?;
    let _serial = der::asn1::Int::decode(&mut r).map_err(err)?;
    let time = GeneralizedTime::decode(&mut r).map_err(err)?;
    let mut nonce = None;
    while !r.is_finished() {
        let item = Any::decode(&mut r).map_err(err)?;
        if item.tag() == Tag::Integer {
            nonce = item.decode_as::<u64>().ok();
        }
    }
    Ok(TstFields {
        imprint_hash: Hash::from_oid(imprint.hash_algorithm.oid),
        imprint: imprint.hashed_message.as_bytes().to_vec(),
        time,
        nonce,
        raw: raw.to_vec(),
    })
}

/// Ergebnis der Zeitstempelprüfung.
#[derive(Debug, Clone)]
pub struct TimestampInfo {
    /// Zeit laut Zeitstempeldienst (lesbar).
    pub time: String,
    pub system_time: SystemTime,
    /// Name des Zeitstempeldienstes (aus seinem Zertifikat).
    pub tsa: String,
    /// Passt der Zeitstempel zur Signatur und ist er korrekt signiert?
    pub integrity: Result<(), String>,
    pub trust: Trust,
}

/// Prüft ein Zeitstempel-Token über `stamped` (z. B. den Signaturwert).
pub fn verify_token(token: &Any, stamped: &[u8], trust: &TrustStore) -> TimestampInfo {
    let mut info = TimestampInfo {
        time: String::new(),
        system_time: SystemTime::UNIX_EPOCH,
        tsa: String::new(),
        integrity: Err(String::new()),
        trust: Trust::Unknown(String::new()),
    };
    if let Err(e) = check_token(token, stamped, trust, &mut info) {
        info.integrity = Err(e);
    }
    info
}

fn check_token(
    token: &Any,
    stamped: &[u8],
    trust: &TrustStore,
    info: &mut TimestampInfo,
) -> Result<(), String> {
    let ci: ContentInfo = token
        .decode_as()
        .map_err(|e| format!("Zeitstempel unlesbar: {e}"))?;
    let tst = token_tst_info(&ci)?;
    info.system_time = tst.time.to_system_time();
    info.time = cert::format_time(info.system_time);
    let hash = tst
        .imprint_hash
        .ok_or("Zeitstempel: unbekanntes Hashverfahren")?;
    if hash.digest([stamped]) != tst.imprint {
        return Err("Der Zeitstempel gehört nicht zu dieser Signatur".into());
    }

    // Das Token ist selbst eine CMS-Signatur über TSTInfo.
    let sd = token_signed_data(&ci)?;
    let si = sd
        .signer_infos
        .0
        .iter()
        .next()
        .ok_or("Zeitstempel ohne Unterzeichner")?;
    let certs: Vec<Certificate> = sd
        .certificates
        .iter()
        .flat_map(|s| s.0.iter())
        .filter_map(|c| match c {
            CertificateChoices::Certificate(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let tsa_cert = crate::verify::find_signer(&certs, &si.sid)
        .ok_or("Zertifikat des Zeitstempeldienstes fehlt")?;
    info.tsa = cert::display_name(&tsa_cert.tbs_certificate.subject);
    let digest_hash =
        Hash::from_oid(si.digest_alg.oid).ok_or("Zeitstempel: unbekanntes Hashverfahren")?;
    let attrs = si
        .signed_attrs
        .as_ref()
        .ok_or("Zeitstempel ohne signierte Attribute")?;
    let md = attrs
        .iter()
        .find(|a| a.oid == OID_MESSAGE_DIGEST)
        .and_then(|a| a.values.iter().next())
        .and_then(|v| v.decode_as::<OctetStringRef>().ok())
        .ok_or("Zeitstempel ohne Hash-Attribut")?;
    if md.as_bytes() != digest_hash.digest([tst.raw.as_slice()]) {
        return Err("Zeitstempel wurde verändert".into());
    }
    let der = attrs.to_der().map_err(|e| e.to_string())?;
    crypto::verify(
        &tsa_cert.tbs_certificate.subject_public_key_info,
        &si.signature_algorithm,
        Some(digest_hash),
        &der,
        si.signature.as_bytes(),
    )?;
    info.integrity = Ok(());
    info.trust = cert::build_chain(tsa_cert, &certs, trust, info.system_time).trust;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    //! Ein Mini-Zeitstempeldienst für Tests: signiert TSTInfo mit einem Testschlüssel.
    use super::*;
    use crate::create::build_signed_data;
    use crate::signer::{KeyKind, Signer};

    /// TSTInfo zum Erzeugen (nur die Pflichtfelder + Nonce).
    #[derive(Sequence)]
    struct TstInfoOut {
        version: u8,
        policy: ObjectIdentifier,
        message_imprint: MessageImprint,
        serial_number: u64,
        gen_time: GeneralizedTime,
        #[asn1(optional = "true")]
        nonce: Option<u64>,
    }

    /// Antwortet wie ein echter Zeitstempeldienst auf eine TimeStampReq.
    pub fn fake_tsa(tsa: &Signer, req: &[u8]) -> Result<Vec<u8>, String> {
        let req = TimeStampReq::from_der(req).map_err(|e| e.to_string())?;
        let tst = TstInfoOut {
            version: 1,
            policy: ObjectIdentifier::new_unwrap("1.2.3.4.1"),
            message_imprint: req.message_imprint,
            serial_number: 42,
            gen_time: GeneralizedTime::from_system_time(SystemTime::now())
                .map_err(|e| e.to_string())?,
            nonce: req.nonce,
        }
        .to_der()
        .map_err(|e| e.to_string())?;
        let token =
            build_signed_data(tsa, OID_TST_INFO, Some(tst), None).map_err(|e| e.to_string())?;
        let resp = TimeStampResp {
            status: PkiStatusInfo {
                status: 0,
                status_string: None,
                fail_info: None,
            },
            token: Some(ContentInfo::from_der(&token).map_err(|e| e.to_string())?),
        };
        resp.to_der().map_err(|e| e.to_string())
    }

    #[test]
    fn request_and_verify_token() {
        let tsa = Signer::self_signed("Test-TSA", "", "", KeyKind::P256, 1).unwrap();
        let sig = b"irgendein Signaturwert";
        let token = request(sig, &|req| fake_tsa(&tsa, req)).unwrap();
        let info = verify_token(&token, sig, &TrustStore::default());
        assert_eq!(info.integrity, Ok(()));
        assert_eq!(info.tsa, "Test-TSA");
        assert!(matches!(info.trust, Trust::Unknown(_)));
        // Anderer Inhalt -> passt nicht.
        assert!(
            verify_token(&token, b"anders", &TrustStore::default())
                .integrity
                .is_err()
        );
    }

    #[test]
    fn signed_pdf_with_timestamp_roundtrip() {
        use foliocraft_core::Document;
        use foliocraft_core::signature_prep::{SigPlacement, SignatureSpec};
        let signer = Signer::self_signed("Erika", "", "", KeyKind::P256, 1).unwrap();
        let tsa = Signer::self_signed("Test-TSA", "", "", KeyKind::P256, 1).unwrap();
        let base = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/form.pdf"),
        )
        .unwrap();
        let spec = SignatureSpec {
            placement: SigPlacement::Existing("unterschrift".into()),
            sub_filter: String::new(),
            name: String::new(),
            reason: String::new(),
            location: String::new(),
            contact: String::new(),
            date: foliocraft_core::date::pdf_date_now(),
            appearance_lines: vec![],
            contents_len: 0,
        };
        let signed =
            crate::sign_pdf(&base, spec, &signer, Some(&|req| fake_tsa(&tsa, req))).unwrap();
        let doc = Document::from_bytes(signed).unwrap();
        let ders: Vec<Vec<u8>> = [&signer.cert, &tsa.cert]
            .iter()
            .map(|c| c.to_der().unwrap())
            .collect();
        let trust = TrustStore::from_der_list(ders.iter().map(Vec::as_slice));
        let r = &crate::verify_document(&doc, &trust)[0];
        assert_eq!(r.status(), crate::Status::Valid);
        let ts = r.timestamp.as_ref().expect("Zeitstempel");
        assert_eq!(ts.integrity, Ok(()));
        assert_eq!(ts.tsa, "Test-TSA");
        assert_eq!(ts.trust, Trust::Trusted);
    }

    #[test]
    fn rejection_is_reported() {
        let resp = TimeStampResp {
            status: PkiStatusInfo {
                status: 2,
                status_string: Some(vec!["bad request".into()]),
                fail_info: None,
            },
            token: None,
        }
        .to_der()
        .unwrap();
        let err = request(b"x", &|_| Ok(resp.clone())).unwrap_err();
        assert!(
            err.contains("abgelehnt") && err.contains("bad request"),
            "{err}"
        );
    }
}
