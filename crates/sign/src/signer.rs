//! Signaturschlüssel: aus einer PKCS#12-Datei laden oder als Testzertifikat erzeugen.
//!
//! Eine PKCS#12-Datei (.p12/.pfx) enthält den privaten Schlüssel und die Zertifikatskette,
//! geschützt durch ein Passwort. So geben z. B. Zertifizierungsstellen und Windows
//! ("Zertifikat exportieren") digitale IDs heraus.

use crate::cert::CertInfo;
use crate::crypto::{self, Hash};
use der::asn1::{Any, ObjectIdentifier};
use der::{Decode, Encode};
use rand_core::{OsRng, RngCore};
use spki::{AlgorithmIdentifierOwned, EncodePublicKey};
use std::str::FromStr;
use std::time::Duration;
use x509_cert::Certificate;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::ext::pkix::ExtendedKeyUsage;
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::Validity;

/// E-Mail-Schutz – Standard-Verwendungszweck für persönliche Signaturzertifikate.
const OID_EKU_EMAIL: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.4");
/// Verwendungszweck "Dokumente signieren" (von PDF-Readern ausgewertet).
const OID_EKU_DOCUMENT_SIGNING: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113583.1.1.5");

#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error(
        "Die Zertifikatsdatei konnte nicht geöffnet werden – falsches Passwort oder keine PKCS#12-Datei."
    )]
    Pkcs12,
    #[error("Die Datei enthält keinen privaten Schlüssel.")]
    NoKey,
    #[error("Schlüsseltyp wird nicht unterstützt (RSA, ECDSA P-256 und P-384 gehen).")]
    UnsupportedKey,
    #[error("Kein Zertifikat in der Datei passt zum privaten Schlüssel.")]
    NoMatchingCertificate,
    #[error("{0}")]
    Other(String),
}

fn other(e: impl std::fmt::Display) -> SignerError {
    SignerError::Other(e.to_string())
}

/// Privater Schlüssel. Wird nie gespeichert oder ausgegeben, nur zum Signieren benutzt.
/// `Clone`, damit ein Hintergrund-Job (Signieren mit Zeitstempel) eine eigene Kopie bekommt.
#[derive(Clone)]
pub enum PrivateKey {
    Rsa(Box<rsa::RsaPrivateKey>),
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
}

// Bewusst ohne Inhalt: Schlüssel gehören nicht in Logs.
impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PrivateKey::Rsa(_) => "PrivateKey::Rsa(…)",
            PrivateKey::P256(_) => "PrivateKey::P256(…)",
            PrivateKey::P384(_) => "PrivateKey::P384(…)",
        })
    }
}

impl PrivateKey {
    /// Aus PKCS#8-DER (so steht der Schlüssel in PKCS#12).
    fn from_pkcs8(der: &[u8]) -> Result<Self, SignerError> {
        use p256::pkcs8::DecodePrivateKey;
        if let Ok(k) = rsa::RsaPrivateKey::from_pkcs8_der(der) {
            return Ok(Self::Rsa(Box::new(k)));
        }
        if let Ok(k) = p256::ecdsa::SigningKey::from_pkcs8_der(der) {
            return Ok(Self::P256(k));
        }
        if let Ok(k) = p384::ecdsa::SigningKey::from_pkcs8_der(der) {
            return Ok(Self::P384(k));
        }
        Err(SignerError::UnsupportedKey)
    }

    fn to_pkcs8(&self) -> Result<Vec<u8>, SignerError> {
        use p256::pkcs8::EncodePrivateKey;
        let doc = match self {
            PrivateKey::Rsa(k) => k.to_pkcs8_der(),
            PrivateKey::P256(k) => k.to_pkcs8_der(),
            PrivateKey::P384(k) => k.to_pkcs8_der(),
        }
        .map_err(other)?;
        Ok(doc.as_bytes().to_vec())
    }

    /// Öffentlicher Schlüssel als SubjectPublicKeyInfo-DER (zum Abgleich mit dem Zertifikat).
    fn public_key_der(&self) -> Result<Vec<u8>, SignerError> {
        let doc = match self {
            PrivateKey::Rsa(k) => rsa::RsaPublicKey::from(k.as_ref()).to_public_key_der(),
            PrivateKey::P256(k) => k.verifying_key().to_public_key_der(),
            PrivateKey::P384(k) => k.verifying_key().to_public_key_der(),
        }
        .map_err(other)?;
        Ok(doc.as_bytes().to_vec())
    }

    /// Passender Hash: SHA-384 zu P-384, sonst SHA-256.
    pub fn hash(&self) -> Hash {
        match self {
            PrivateKey::P384(_) => Hash::Sha384,
            _ => Hash::Sha256,
        }
    }

    /// Signiert einen bereits berechneten Hash. Liefert Algorithmuskennung und Signaturwert.
    pub fn sign_prehashed(
        &self,
        digest: &[u8],
    ) -> Result<(AlgorithmIdentifierOwned, Vec<u8>), SignerError> {
        use p256::ecdsa::signature::hazmat::PrehashSigner;
        match self {
            PrivateKey::Rsa(k) => {
                // `sign_with_rng` nutzt "Blinding" (Zufall), das schützt vor Zeitmessungs-Angriffen.
                let sig = k
                    .sign_with_rng(&mut OsRng, rsa::Pkcs1v15Sign::new::<sha2::Sha256>(), digest)
                    .map_err(other)?;
                // RSA-Kennungen tragen laut Standard ausdrücklich NULL als Parameter.
                let alg = AlgorithmIdentifierOwned {
                    oid: crypto::OID_RSA_SHA256,
                    parameters: Some(Any::from(der::asn1::Null)),
                };
                Ok((alg, sig))
            }
            PrivateKey::P256(k) => {
                let sig: p256::ecdsa::Signature = k.sign_prehash(digest).map_err(other)?;
                let alg = AlgorithmIdentifierOwned {
                    oid: crypto::OID_ECDSA_SHA256,
                    parameters: None,
                };
                Ok((alg, sig.to_der().as_bytes().to_vec()))
            }
            PrivateKey::P384(k) => {
                let sig: p384::ecdsa::Signature = k.sign_prehash(digest).map_err(other)?;
                let alg = AlgorithmIdentifierOwned {
                    oid: crypto::OID_ECDSA_SHA384,
                    parameters: None,
                };
                Ok((alg, sig.to_der().as_bytes().to_vec()))
            }
        }
    }
}

/// Art des Schlüssels für neue Testzertifikate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// RSA 2048 Bit – am weitesten verbreitet.
    Rsa2048,
    /// ECDSA P-256 – kürzer und schneller, von aktuellen Readern unterstützt.
    P256,
}

/// Digitale ID: Schlüssel + Zertifikat (+ weitere Zertifikate der Kette).
#[derive(Debug, Clone)]
pub struct Signer {
    pub key: PrivateKey,
    pub cert: Certificate,
    /// Weitere Zertifikate (Zwischen- und Stammzertifikate), ohne `cert`.
    pub chain: Vec<Certificate>,
}

impl Signer {
    /// Öffnet eine PKCS#12-Datei.
    pub fn from_pkcs12(data: &[u8], password: &str) -> Result<Self, SignerError> {
        let store =
            p12_keystore::KeyStore::from_pkcs12(data, password).map_err(|_| SignerError::Pkcs12)?;
        let (_, entry) = store.private_key_chain().ok_or(SignerError::NoKey)?;
        let key = PrivateKey::from_pkcs8(entry.key())?;
        let certs: Vec<Certificate> = entry
            .chain()
            .iter()
            .filter_map(|c| Certificate::from_der(c.as_der()).ok())
            .collect();
        let public = key.public_key_der()?;
        let pos = certs
            .iter()
            .position(|c| {
                c.tbs_certificate
                    .subject_public_key_info
                    .to_der()
                    .is_ok_and(|d| d == public)
            })
            .ok_or(SignerError::NoMatchingCertificate)?;
        let mut chain = certs;
        let cert = chain.remove(pos);
        Ok(Self { key, cert, chain })
    }

    /// Speichert Schlüssel und Kette als PKCS#12 (Passwortschutz mit AES).
    pub fn to_pkcs12(&self, password: &str) -> Result<Vec<u8>, SignerError> {
        let mut certs = Vec::new();
        for c in std::iter::once(&self.cert).chain(&self.chain) {
            let der = c.to_der().map_err(other)?;
            certs.push(p12_keystore::Certificate::from_der(&der).map_err(other)?);
        }
        let mut local_id = [0u8; 20];
        OsRng.fill_bytes(&mut local_id);
        let entry = p12_keystore::PrivateKeyChain::new(self.key.to_pkcs8()?, local_id, certs);
        let mut store = p12_keystore::KeyStore::new();
        store.add_entry(
            "pdftool",
            p12_keystore::KeyStoreEntry::PrivateKeyChain(entry),
        );
        store.writer(password).write().map_err(other)
    }

    pub fn info(&self) -> CertInfo {
        CertInfo::of(&self.cert)
    }

    /// Erzeugt eine neue digitale ID mit selbst ausgestelltem Zertifikat.
    ///
    /// Andere Reader zeigen solche Signaturen als "Identität unbekannt" an, bis man dem
    /// Zertifikat ausdrücklich vertraut – für Tests und den internen Gebrauch reicht das.
    pub fn self_signed(
        name: &str,
        organization: &str,
        email: &str,
        kind: KeyKind,
        years: u32,
    ) -> Result<Self, SignerError> {
        let mut dn = format!("CN={}", escape_dn(name));
        if !organization.is_empty() {
            dn.push_str(&format!(",O={}", escape_dn(organization)));
        }
        if !email.is_empty() {
            dn.push_str(&format!(",1.2.840.113549.1.9.1={}", escape_dn(email)));
        }
        let subject = Name::from_str(&dn).map_err(other)?;
        let validity = Validity::from_now(Duration::from_secs(u64::from(years) * 365 * 86_400))
            .map_err(other)?;
        let key = match kind {
            KeyKind::Rsa2048 => PrivateKey::Rsa(Box::new(
                rsa::RsaPrivateKey::new(&mut OsRng, 2048).map_err(other)?,
            )),
            KeyKind::P256 => PrivateKey::P256(p256::ecdsa::SigningKey::random(&mut OsRng)),
        };
        let cert = build_certificate(&key, subject, validity)?;
        Ok(Self {
            key,
            cert,
            chain: Vec::new(),
        })
    }
}

/// Zufällige, positive Seriennummer (16 Byte).
fn random_serial() -> Result<SerialNumber, SignerError> {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[0] &= 0x7F; // höchstes Bit 0 = positive Zahl
    bytes[0] |= 0x01; // nie mit Null beginnen (DER-Normalform)
    SerialNumber::new(&bytes).map_err(other)
}

fn build_certificate(
    key: &PrivateKey,
    subject: Name,
    validity: Validity,
) -> Result<Certificate, SignerError> {
    let spki_der = key.public_key_der()?;
    let spki = spki::SubjectPublicKeyInfoOwned::from_der(&spki_der).map_err(other)?;
    let serial = random_serial()?;
    // Endzertifikat mit sich selbst als Aussteller = selbst ausgestellt. Das Profil setzt
    // Schlüsselkennungen, "keine CA" und die Schlüsselverwendung (digitale Signatur).
    let profile = Profile::Leaf {
        issuer: subject.clone(),
        enable_key_agreement: false,
        enable_key_encipherment: false,
    };

    // Der Builder ist generisch über den Signierer. Je Schlüsseltyp ein eigener Aufruf,
    // weil RSA und ECDSA unterschiedliche Typen sind.
    macro_rules! build {
        ($signer:expr, $sig:ty) => {{
            let mut b = CertificateBuilder::new(profile, serial, validity, subject, spki, $signer)
                .map_err(other)?;
            b.add_extension(&ExtendedKeyUsage(vec![
                OID_EKU_EMAIL,
                OID_EKU_DOCUMENT_SIGNING,
            ]))
            .map_err(other)?;
            b.build::<$sig>().map_err(other)?
        }};
    }
    Ok(match key {
        PrivateKey::Rsa(k) => {
            let signer = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new(k.as_ref().clone());
            build!(&signer, rsa::pkcs1v15::Signature)
        }
        PrivateKey::P256(k) => build!(k, p256::ecdsa::DerSignature),
        PrivateKey::P384(k) => build!(k, p384::ecdsa::DerSignature),
    })
}

/// Sonderzeichen in Namensteilen maskieren (RFC 4514), z. B. "Muster, Erika" -> "Muster\, Erika".
fn escape_dn(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.trim().chars().enumerate() {
        if matches!(c, ',' | '+' | '"' | '\\' | '<' | '>' | ';' | '=') || (i == 0 && c == '#') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_names() {
        assert_eq!(escape_dn("Muster, Erika"), "Muster\\, Erika");
        assert_eq!(escape_dn("#1"), "\\#1");
    }

    #[test]
    fn self_signed_p256_roundtrips_through_pkcs12() {
        let s = Signer::self_signed(
            "Erika Muster, Köln",
            "Test GmbH",
            "erika@example.org",
            KeyKind::P256,
            3,
        )
        .unwrap();
        assert_eq!(s.info().name, "Erika Muster, Köln");
        // Zertifikat ist von sich selbst signiert.
        crate::cert::verify_issued_by(&s.cert, &s.cert).unwrap();
        let p12 = s.to_pkcs12("geheim").unwrap();
        assert!(matches!(
            Signer::from_pkcs12(&p12, "falsch"),
            Err(SignerError::Pkcs12)
        ));
        let back = Signer::from_pkcs12(&p12, "geheim").unwrap();
        assert_eq!(back.cert, s.cert);
        assert!(matches!(back.key, PrivateKey::P256(_)));
    }
}
