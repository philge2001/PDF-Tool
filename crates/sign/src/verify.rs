//! Signaturen eines Dokuments finden und prüfen.
//!
//! Aufbau einer PDF-Signatur:
//! - Das Signaturfeld zeigt mit /V auf ein Signatur-Dictionary.
//! - Dort steht /ByteRange [a b c d]: Signiert sind die Bytes a..a+b und c..c+d der Datei –
//!   also alles außer der Lücke dazwischen. In der Lücke steht /Contents <…>: die eigentliche
//!   Signatur als CMS-Struktur (Hex-kodiert).
//! - Wird die Datei später inkrementell erweitert, hängen neue Bytes hinten an; der signierte
//!   Teil bleibt gleich. Daran erkennen wir "nach dem Signieren geändert".

use crate::cert::{self, CertInfo, Trust, TrustStore};
use crate::crypto::{self, Hash};
use crate::timestamp::{self, TimestampInfo};
use cms::cert::CertificateChoices;
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use der::asn1::OctetStringRef;
use der::oid::ObjectIdentifier;
use der::{Decode, Encode, Reader, SliceReader};
use lopdf::{Dictionary, Object};
use pdftool_core::geometry::Rect;
use pdftool_core::{Document, FieldKind, pdf_string};
use std::time::SystemTime;
use x509_cert::Certificate;

const OID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const OID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const OID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const OID_SUBJECT_KEY_ID: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.14");

/// Ist der signierte Inhalt unverändert und die Signatur mathematisch korrekt?
#[derive(Debug, Clone, PartialEq)]
pub enum Integrity {
    Intact,
    /// Inhalt oder Signatur passen nicht zusammen – die Signatur ist UNGÜLTIG.
    Broken(String),
    /// Können wir nicht prüfen (unbekanntes Verfahren, kaputte Struktur …).
    Unsupported(String),
}

/// Zusammenfassung für die Anzeige.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Unverändert, Unterzeichner vertrauenswürdig.
    Valid,
    /// Unverändert, aber Identität nicht bestätigt (Zertifikat unbekannt).
    UnknownSigner,
    /// Ungültig oder nicht prüfbar.
    Invalid,
}

/// Ergebnis der Prüfung einer Signatur.
#[derive(Debug, Clone)]
pub struct SignatureReport {
    pub field_name: String,
    /// Seite (Index) und Rechteck des Signaturfelds; unsichtbare Signaturen haben ein leeres Rechteck.
    pub page: Option<usize>,
    pub rect: Option<Rect>,
    /// Anzeigename des Unterzeichners (aus dem Zertifikat, sonst /Name).
    pub signer: String,
    pub reason: String,
    pub location: String,
    pub contact: String,
    /// Signaturzeit, wie vom Unterzeichner angegeben (nicht unabhängig bestätigt).
    pub time: Option<String>,
    pub sub_filter: String,
    pub hash: Option<Hash>,
    /// Zertifizierungssignatur (DocMDP) mit erlaubten Änderungen 1–3.
    pub certification: Option<i64>,
    pub integrity: Integrity,
    pub trust: Trust,
    /// Kette: Unterzeichner zuerst.
    pub chain: Vec<CertInfo>,
    /// Länge der signierten Fassung in Bytes (Datei bis zu dieser Stelle).
    pub signed_len: usize,
    /// Anzahl späterer Revisionen (0 = die Signatur deckt die ganze Datei ab).
    pub later_revisions: usize,
    /// Zeitstempel eines Zeitstempeldienstes (PAdES B-T), falls vorhanden.
    pub timestamp: Option<TimestampInfo>,
    /// Dokument-Zeitstempel statt Signatur einer Person (SubFilter ETSI.RFC3161).
    pub document_timestamp: bool,
}

impl SignatureReport {
    pub fn status(&self) -> Status {
        match (&self.integrity, &self.trust) {
            (Integrity::Intact, Trust::Trusted) => Status::Valid,
            (Integrity::Intact, Trust::Unknown(_)) => Status::UnknownSigner,
            _ => Status::Invalid,
        }
    }

    /// Kurztext wie "Gültig", "Unbekannter Unterzeichner", "Ungültig".
    pub fn summary(&self) -> String {
        let base = match self.status() {
            Status::Valid => "Gültig",
            Status::UnknownSigner => "Identität nicht bestätigt",
            Status::Invalid => "Ungültig",
        };
        if self.later_revisions > 0 && self.integrity == Integrity::Intact {
            format!("{base} – danach geändert")
        } else {
            base.to_string()
        }
    }
}

/// Signaturfelder ohne Signatur (zum späteren Signieren).
pub fn unsigned_fields(doc: &Document) -> Vec<String> {
    doc.forms()
        .iter()
        .filter(|f| f.kind == FieldKind::Signature)
        .filter(|f| signature_dict(doc, f.source).is_none())
        .map(|f| f.name.clone())
        .collect()
}

fn signature_dict(doc: &Document, field: Option<lopdf::ObjectId>) -> Option<&Dictionary> {
    let pdf = doc.pdf();
    let v = pdf.get_dictionary(field?).ok()?.get(b"V").ok()?;
    pdf.dereference(v).ok()?.1.as_dict().ok()
}

/// Prüft alle Signaturen des Dokuments (in Dateireihenfolge der Felder).
pub fn verify_document(doc: &Document, trust: &TrustStore) -> Vec<SignatureReport> {
    let bytes = doc.bytes();
    doc.forms()
        .iter()
        .filter(|f| f.kind == FieldKind::Signature)
        .filter_map(|f| {
            let dict = signature_dict(doc, f.source)?;
            let w = f.widgets.first();
            let mut r = verify_signature(bytes, dict, trust);
            r.field_name = f.name.clone();
            r.page = w.and_then(|w| doc.page_index_of(w.page));
            r.rect = w.map(|w| w.rect);
            Some(r)
        })
        .collect()
}

/// Prüft ein einzelnes Signatur-Dictionary gegen die Dateibytes.
pub fn verify_signature(bytes: &[u8], dict: &Dictionary, trust: &TrustStore) -> SignatureReport {
    let text = |key: &[u8]| pdf_string::get(dict, key);
    let mut report = SignatureReport {
        field_name: String::new(),
        page: None,
        rect: None,
        signer: text(b"Name"),
        reason: text(b"Reason"),
        location: text(b"Location"),
        contact: text(b"ContactInfo"),
        time: dict
            .get(b"M")
            .ok()
            .and_then(|o| o.as_str().ok())
            .map(|s| pdftool_core::date::display_pdf_date(&String::from_utf8_lossy(s))),
        sub_filter: dict
            .get(b"SubFilter")
            .and_then(Object::as_name)
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default(),
        hash: None,
        certification: certification(dict),
        integrity: Integrity::Unsupported(String::new()),
        trust: Trust::Unknown(String::new()),
        chain: Vec::new(),
        signed_len: 0,
        later_revisions: 0,
        timestamp: None,
        document_timestamp: false,
    };
    if let Err(e) = check(bytes, dict, trust, &mut report) {
        report.integrity = e;
    }
    report
}

/// /Reference [ << /TransformMethod /DocMDP /TransformParams << /P n >> >> ]
fn certification(dict: &Dictionary) -> Option<i64> {
    let refs = dict.get(b"Reference").ok()?.as_array().ok()?;
    refs.iter().find_map(|r| {
        let r = r.as_dict().ok()?;
        (r.get(b"TransformMethod").ok()?.as_name().ok()? == b"DocMDP").then(|| {
            r.get(b"TransformParams")
                .ok()
                .and_then(|p| p.as_dict().ok())
                .and_then(|p| p.get(b"P").ok())
                .and_then(|p| p.as_i64().ok())
                .unwrap_or(2)
        })
    })
}

/// Die eigentliche Prüfung. `Err` = Integrität nicht gegeben (Grund steht drin).
fn check(
    bytes: &[u8],
    dict: &Dictionary,
    trust: &TrustStore,
    report: &mut SignatureReport,
) -> Result<(), Integrity> {
    let broken = |s: &str| Integrity::Broken(s.to_string());
    let unsupported = |s: String| Integrity::Unsupported(s);

    // 1) /ByteRange prüfen: Beginn bei 0, Lücke genau um /Contents <…>.
    let range: Vec<usize> = dict
        .get(b"ByteRange")
        .and_then(Object::as_array)
        .map_err(|_| broken("/ByteRange fehlt"))?
        .iter()
        .map(|o| o.as_i64().ok().and_then(|v| usize::try_from(v).ok()))
        .collect::<Option<_>>()
        .ok_or_else(|| broken("/ByteRange ist ungültig"))?;
    let [a, b, c, d] = range[..] else {
        return Err(broken("/ByteRange hat nicht vier Einträge"));
    };
    let end = c
        .checked_add(d)
        .ok_or_else(|| broken("/ByteRange ist ungültig"))?;
    if a != 0 || b == 0 || c <= b + 1 || end > bytes.len() {
        return Err(broken("/ByteRange passt nicht zur Datei"));
    }
    if bytes[b] != b'<' || bytes[c - 1] != b'>' {
        return Err(broken("Die Lücke in /ByteRange enthält nicht die Signatur"));
    }
    report.signed_len = end;
    // Steht nach dem signierten Teil noch mehr als Leerraum, wurde später etwas angehängt.
    let tail = &bytes[end..];
    if !tail.iter().all(u8::is_ascii_whitespace) {
        report.later_revisions = count(tail, b"%%EOF").max(1);
    }

    // 2) CMS aus der Lücke lesen (Hex, hinten mit Nullen aufgefüllt).
    let contents =
        hex_decode(&bytes[b + 1..c - 1]).ok_or_else(|| broken("/Contents ist kein Hex"))?;
    let mut reader = SliceReader::new(&contents).map_err(|e| unsupported(e.to_string()))?;
    // `decode` liest genau EIN DER-Objekt; die Füll-Nullen dahinter bleiben ungelesen.
    // Dokument-Zeitstempel: /Contents ist direkt ein RFC-3161-Token über die Bytes.
    if report.sub_filter == "ETSI.RFC3161" {
        report.document_timestamp = true;
        let token = der::Any::decode(&mut reader)
            .map_err(|e| unsupported(format!("Zeitstempel unlesbar: {e}")))?;
        let data = [&bytes[a..a + b], &bytes[c..end]].concat();
        let ts = timestamp::verify_token(&token, &data, trust);
        report.signer = ts.tsa.clone();
        report.time = Some(ts.time.clone());
        report.trust = ts.trust.clone();
        report.integrity = match &ts.integrity {
            Ok(()) => Integrity::Intact,
            Err(e) => Integrity::Broken(e.clone()),
        };
        report.timestamp = Some(ts);
        return Ok(());
    }
    let info = ContentInfo::decode(&mut reader)
        .map_err(|e| unsupported(format!("CMS-Struktur unlesbar: {e}")))?;
    let _ = reader.remaining_len();
    if info.content_type != OID_SIGNED_DATA {
        return Err(unsupported("Kein CMS SignedData".into()));
    }
    let sd: SignedData = info
        .content
        .decode_as()
        .map_err(|e| unsupported(format!("SignedData unlesbar: {e}")))?;
    let si: &SignerInfo = sd
        .signer_infos
        .0
        .iter()
        .next()
        .ok_or_else(|| unsupported("Keine Unterzeichnerangaben".into()))?;
    let certs: Vec<Certificate> = sd
        .certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|c| match c {
            CertificateChoices::Certificate(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let signer = find_signer(&certs, &si.sid)
        .ok_or_else(|| unsupported("Zertifikat des Unterzeichners fehlt in der Signatur".into()))?;
    report.signer = cert::display_name(&signer.tbs_certificate.subject);

    // 3) Hash über die signierten Bytes.
    let hash = Hash::from_oid(si.digest_alg.oid)
        .ok_or_else(|| unsupported(format!("Hashverfahren {} unbekannt", si.digest_alg.oid)))?;
    report.hash = Some(hash);
    let ranges = [&bytes[a..a + b], &bytes[c..end]];
    let content_digest = match report.sub_filter.as_str() {
        // Alt: Der SHA-1-Hash der Bytes ist selbst der signierte Inhalt.
        "adbe.pkcs7.sha1" => {
            let embedded = sd
                .encap_content_info
                .econtent
                .as_ref()
                .and_then(|e| e.decode_as::<OctetStringRef>().ok())
                .map(|o| o.as_bytes().to_vec())
                .ok_or_else(|| unsupported("Eingebetteter Hash fehlt".into()))?;
            if embedded != Hash::Sha1.digest(ranges) {
                return Err(broken("Das Dokument wurde nach dem Signieren verändert"));
            }
            hash.digest([embedded.as_slice()])
        }
        _ => hash.digest(ranges),
    };

    // 4) Signierte Attribute: Hash muss zu den Bytes passen, Signatur gilt den Attributen.
    let sig = si.signature.as_bytes();
    let key = &signer.tbs_certificate.subject_public_key_info;
    match &si.signed_attrs {
        Some(attrs) => {
            let md = attrs
                .iter()
                .find(|a| a.oid == OID_MESSAGE_DIGEST)
                .and_then(|a| a.values.iter().next())
                .and_then(|v| v.decode_as::<OctetStringRef>().ok())
                .ok_or_else(|| broken("Hash-Attribut (messageDigest) fehlt"))?;
            if md.as_bytes() != content_digest {
                return Err(broken("Das Dokument wurde nach dem Signieren verändert"));
            }
            if let Some(t) = attrs
                .iter()
                .find(|a| a.oid == OID_SIGNING_TIME)
                .and_then(|a| a.values.iter().next())
                .and_then(decode_time)
            {
                report.time = Some(cert::format_time(t.to_system_time()));
            }
            // Signiert wurde die DER-Kodierung der Attribute als SET (nicht als [0]).
            let der = attrs.to_der().map_err(|e| unsupported(e.to_string()))?;
            crypto::verify(key, &si.signature_algorithm, Some(hash), &der, sig)
                .map_err(Integrity::Broken)?;
        }
        None => {
            crypto::verify_prehashed(key, hash, &content_digest, sig).map_err(Integrity::Broken)?;
        }
    }
    report.integrity = Integrity::Intact;

    // Zeitstempel über den Signaturwert (nicht signiertes Attribut).
    if let Some(token) = si
        .unsigned_attrs
        .iter()
        .flat_map(|set| set.iter())
        .find(|a| a.oid == timestamp::OID_TIMESTAMP_TOKEN)
        .and_then(|a| a.values.iter().next())
    {
        report.timestamp = Some(timestamp::verify_token(token, sig, trust));
    }

    // 5) Vertrauen: Kette bis zu einem vertrauenswürdigen Zertifikat.
    // Prüfzeitpunkt: bestätigte Zeit aus einem gültigen Zeitstempel, sonst die angegebene
    // Signaturzeit, sonst jetzt.
    let stamped = report
        .timestamp
        .as_ref()
        .filter(|t| t.integrity.is_ok())
        .map(|t| t.system_time);
    let at = stamped
        .or_else(|| signing_time(si))
        .unwrap_or_else(pdftool_core::date::system_now);
    let chain = cert::build_chain(signer, &certs, trust, at);
    report.chain = chain.certs.iter().map(CertInfo::of).collect();
    report.trust = chain.trust;
    Ok(())
}

fn signing_time(si: &SignerInfo) -> Option<SystemTime> {
    si.signed_attrs
        .as_ref()?
        .iter()
        .find(|a| a.oid == OID_SIGNING_TIME)?
        .values
        .iter()
        .next()
        .and_then(decode_time)
        .map(|t| t.to_system_time())
}

/// Zeitangabe (UTCTime oder GeneralizedTime – eine ASN.1-"CHOICE") aus einem Attributwert.
fn decode_time(v: &der::Any) -> Option<x509_cert::time::Time> {
    x509_cert::time::Time::from_der(&v.to_der().ok()?).ok()
}

/// Zertifikat zum SignerIdentifier (Aussteller + Seriennummer oder Schlüsselkennung).
pub(crate) fn find_signer<'a>(
    certs: &'a [Certificate],
    sid: &SignerIdentifier,
) -> Option<&'a Certificate> {
    certs.iter().find(|c| match sid {
        SignerIdentifier::IssuerAndSerialNumber(is) => {
            c.tbs_certificate.issuer == is.issuer
                && c.tbs_certificate.serial_number == is.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(skid) => c
            .tbs_certificate
            .extensions
            .iter()
            .flatten()
            .find(|e| e.extn_id == OID_SUBJECT_KEY_ID)
            .and_then(|e| OctetStringRef::from_der(e.extn_value.as_bytes()).ok())
            .is_some_and(|k| k.as_bytes() == skid.0.as_bytes()),
    })
}

fn hex_decode(s: &[u8]) -> Option<Vec<u8>> {
    let digits: Vec<u8> = s
        .iter()
        .copied()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let val = |c: u8| (c as char).to_digit(16).map(|v| v as u8);
    digits
        .chunks(2)
        .map(|p| Some(val(p[0])? << 4 | p.get(1).map_or(Some(0), |&c| val(c))?))
        .collect()
}

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_count() {
        assert_eq!(hex_decode(b"0A ff\n10"), Some(vec![0x0A, 0xFF, 0x10]));
        assert_eq!(hex_decode(b"zz"), None);
        assert_eq!(count(b"%%EOF\n...%%EOF", b"%%EOF"), 2);
    }

    #[test]
    fn missing_byte_range_is_broken() {
        let dict = lopdf::dictionary! { "Type" => "Sig" };
        let r = verify_signature(b"%PDF-1.7", &dict, &TrustStore::default());
        assert!(matches!(r.integrity, Integrity::Broken(_)));
        assert_eq!(r.status(), Status::Invalid);
    }
}
