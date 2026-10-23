//! Zertifikate: lesbare Zusammenfassung, Vertrauensspeicher und Kettenbildung.
//!
//! Eine Signatur sagt nur "der Inhaber dieses Schlüssels hat signiert". Wer das ist, bestätigt
//! das Zertifikat – und dem glauben wir nur, wenn eine Kette von Zertifikaten bis zu einem
//! Stammzertifikat führt, dem der Nutzer ausdrücklich vertraut (Vertrauensspeicher).

use crate::crypto;
use der::oid::ObjectIdentifier;
use der::{Decode, DecodePem, Encode, EncodePem};
use std::time::{SystemTime, UNIX_EPOCH};
use x509_cert::Certificate;
use x509_cert::name::Name;

const OID_CN: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");
const OID_O: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.10");
const OID_EMAIL: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.1");

/// Maximale Kettenlänge (schützt vor Endlosschleifen bei kaputten Zertifikaten).
const MAX_CHAIN: usize = 10;

/// Lesbare Angaben zu einem Zertifikat.
#[derive(Debug, Clone, PartialEq)]
pub struct CertInfo {
    /// Anzeigename (CN, sonst Organisation, sonst der ganze Name).
    pub name: String,
    pub subject: String,
    pub issuer: String,
    pub serial: String,
    pub not_before: String,
    pub not_after: String,
    /// SHA-256 über das ganze Zertifikat (eindeutige Kennung, z. B. zum Vergleichen).
    pub fingerprint: String,
    pub der: Vec<u8>,
}

impl CertInfo {
    pub fn of(cert: &Certificate) -> Self {
        let tbs = &cert.tbs_certificate;
        let der = cert.to_der().unwrap_or_default();
        Self {
            name: display_name(&tbs.subject),
            subject: tbs.subject.to_string(),
            issuer: tbs.issuer.to_string(),
            serial: crypto::hex(tbs.serial_number.as_bytes()),
            not_before: format_time(tbs.validity.not_before.to_system_time()),
            not_after: format_time(tbs.validity.not_after.to_system_time()),
            fingerprint: crypto::hex(&crypto::Hash::Sha256.digest([der.as_slice()])),
            der,
        }
    }
}

/// Kurzer Anzeigename eines X.509-Namens.
pub fn display_name(name: &Name) -> String {
    let attr = |oid: ObjectIdentifier| {
        name.0
            .iter()
            .flat_map(|rdn| rdn.0.iter())
            .find_map(|a| (a.oid == oid).then(|| attr_text(&a.value)).flatten())
    };
    attr(OID_CN)
        .or_else(|| attr(OID_O))
        .or_else(|| attr(OID_EMAIL))
        .unwrap_or_else(|| name.to_string())
}

/// Textwert eines Namensattributs (UTF8String, PrintableString, IA5String …).
fn attr_text(v: &der::Any) -> Option<String> {
    // Alle diese String-Typen sind im DER einfach Bytes; Text ist (fast) immer UTF-8-kompatibel.
    std::str::from_utf8(v.value()).ok().map(str::to_string)
}

pub fn format_time(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    pdftool_core::date::display_from_unix(secs)
}

/// Zertifikat als PEM-Text ("-----BEGIN CERTIFICATE-----…").
pub fn to_pem(der: &[u8]) -> Option<String> {
    Certificate::from_der(der)
        .ok()?
        .to_pem(der::pem::LineEnding::LF)
        .ok()
}

/// Zertifikat aus DER- oder PEM-Bytes (Dateiendungen .cer/.crt/.der/.pem).
pub fn parse_certificate(bytes: &[u8]) -> Option<Certificate> {
    Certificate::from_der(bytes)
        .ok()
        .or_else(|| Certificate::from_pem(bytes).ok())
}

/// Vom Nutzer als vertrauenswürdig markierte Zertifikate (meist Stammzertifikate).
#[derive(Debug, Clone, Default)]
pub struct TrustStore {
    anchors: Vec<Certificate>,
}

impl TrustStore {
    /// Aus gespeicherten DER-Bytes (unlesbare Einträge werden übersprungen).
    pub fn from_der_list<'a>(items: impl IntoIterator<Item = &'a [u8]>) -> Self {
        Self {
            anchors: items
                .into_iter()
                .filter_map(|d| Certificate::from_der(d).ok())
                .collect(),
        }
    }

    /// Aus gespeicherten PEM-Texten (so legt die App sie in den Einstellungen ab).
    pub fn from_pem_list<'a>(items: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            anchors: items
                .into_iter()
                .filter_map(|p| Certificate::from_pem(p.as_bytes()).ok())
                .collect(),
        }
    }

    pub fn contains(&self, cert: &Certificate) -> bool {
        self.anchors.iter().any(|a| a == cert)
    }

    pub fn anchors(&self) -> &[Certificate] {
        &self.anchors
    }
}

/// Wie weit reicht das Vertrauen in den Unterzeichner?
#[derive(Debug, Clone, PartialEq)]
pub enum Trust {
    /// Kette bis zu einem vertrauenswürdigen Zertifikat, alle Zertifikate zum Zeitpunkt gültig.
    Trusted,
    /// Kette ist in sich stimmig, endet aber nicht bei einem vertrauenswürdigen Zertifikat.
    Unknown(String),
    /// Kette ist fehlerhaft (falsch signiert, abgelaufen …).
    Invalid(String),
}

/// Ergebnis der Kettenbildung.
#[derive(Debug, Clone)]
pub struct Chain {
    /// Unterzeichner zuerst, Stammzertifikat (falls gefunden) zuletzt.
    pub certs: Vec<Certificate>,
    pub trust: Trust,
}

/// Baut die Kette vom Unterzeichner `leaf` aufwärts. `pool` = Zertifikate aus der Signatur.
/// `at` = Prüfzeitpunkt (Signaturzeit, sonst jetzt).
pub fn build_chain(
    leaf: &Certificate,
    pool: &[Certificate],
    trust: &TrustStore,
    at: SystemTime,
) -> Chain {
    let mut certs = vec![leaf.clone()];
    let verdict = loop {
        let current = certs.last().expect("mindestens ein Zertifikat");
        if let Err(e) = check_validity(current, at) {
            break Trust::Invalid(e);
        }
        if trust.contains(current) {
            break Trust::Trusted;
        }
        let tbs = &current.tbs_certificate;
        let is_self_issued = tbs.issuer == tbs.subject;
        if is_self_issued {
            break match verify_issued_by(current, current) {
                Ok(()) => Trust::Unknown(format!(
                    "Das Stammzertifikat „{}“ ist nicht als vertrauenswürdig hinterlegt",
                    display_name(&tbs.subject)
                )),
                Err(e) => {
                    Trust::Invalid(format!("Zertifikat „{}“: {e}", display_name(&tbs.subject)))
                }
            };
        }
        if certs.len() >= MAX_CHAIN {
            break Trust::Invalid("Zertifikatskette ist zu lang".into());
        }
        // Aussteller suchen: passender Name UND gültige Signatur über das aktuelle Zertifikat.
        let candidates = pool.iter().chain(trust.anchors());
        let issuer = candidates
            .filter(|c| c.tbs_certificate.subject == tbs.issuer && !certs.contains(c))
            .find(|c| verify_issued_by(current, c).is_ok())
            .cloned();
        match issuer {
            Some(i) => certs.push(i),
            None => {
                break Trust::Unknown(format!(
                    "Ausstellerzertifikat „{}“ fehlt bzw. ist nicht vertrauenswürdig",
                    display_name(&tbs.issuer)
                ));
            }
        }
    };
    Chain {
        certs,
        trust: verdict,
    }
}

fn check_validity(cert: &Certificate, at: SystemTime) -> Result<(), String> {
    let v = &cert.tbs_certificate.validity;
    let name = display_name(&cert.tbs_certificate.subject);
    if at < v.not_before.to_system_time() {
        return Err(format!(
            "Zertifikat „{name}“ war zum Prüfzeitpunkt noch nicht gültig (ab {})",
            format_time(v.not_before.to_system_time())
        ));
    }
    if at > v.not_after.to_system_time() {
        return Err(format!(
            "Zertifikat „{name}“ war zum Prüfzeitpunkt abgelaufen (bis {})",
            format_time(v.not_after.to_system_time())
        ));
    }
    Ok(())
}

/// Hat `issuer` das Zertifikat `cert` signiert?
pub fn verify_issued_by(cert: &Certificate, issuer: &Certificate) -> Result<(), String> {
    let tbs = cert.tbs_certificate.to_der().map_err(|e| e.to_string())?;
    let sig = cert
        .signature
        .as_bytes()
        .ok_or("Zertifikatssignatur unlesbar")?;
    crypto::verify(
        &issuer.tbs_certificate.subject_public_key_info,
        &cert.signature_algorithm,
        None,
        &tbs,
        sig,
    )
}
