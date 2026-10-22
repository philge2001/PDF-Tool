//! Hash- und Signaturverfahren: was wir prüfen (und später erzeugen) können.
//!
//! Algorithmen werden in CMS/X.509 über OIDs (Objektkennungen wie "2.16.840.1.101.3.4.2.1")
//! benannt. Hier steht die Zuordnung OID -> Verfahren an einer Stelle.

use der::Encode;
use der::oid::ObjectIdentifier;
use sha2::Digest;
use spki::{AlgorithmIdentifierOwned, SubjectPublicKeyInfoOwned};

pub const OID_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
pub const OID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
pub const OID_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
pub const OID_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");

pub const OID_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
pub const OID_RSA_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.5");
pub const OID_RSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
pub const OID_RSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
pub const OID_RSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
pub const OID_RSA_PSS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.10");
pub const OID_EC_PUBLIC_KEY: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
pub const OID_ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
pub const OID_ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
pub const OID_ECDSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.4");
pub const OID_P256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");
pub const OID_P384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.132.0.34");

/// Unterstützte Hashverfahren.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    pub fn from_oid(oid: ObjectIdentifier) -> Option<Self> {
        Some(match oid {
            OID_SHA1 => Hash::Sha1,
            OID_SHA256 => Hash::Sha256,
            OID_SHA384 => Hash::Sha384,
            OID_SHA512 => Hash::Sha512,
            _ => return None,
        })
    }

    pub fn oid(self) -> ObjectIdentifier {
        match self {
            Hash::Sha1 => OID_SHA1,
            Hash::Sha256 => OID_SHA256,
            Hash::Sha384 => OID_SHA384,
            Hash::Sha512 => OID_SHA512,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Hash::Sha1 => "SHA-1",
            Hash::Sha256 => "SHA-256",
            Hash::Sha384 => "SHA-384",
            Hash::Sha512 => "SHA-512",
        }
    }

    /// Hash über mehrere Teilstücke (z. B. die beiden Bereiche der /ByteRange).
    pub fn digest<'a>(self, parts: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
        // Generische Hilfsfunktion: `D` ist irgendein Hashverfahren mit dem `Digest`-Trait.
        fn run<'a, D: Digest>(parts: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
            let mut h = D::new();
            for p in parts {
                h.update(p);
            }
            h.finalize().to_vec()
        }
        match self {
            Hash::Sha1 => run::<sha1::Sha1>(parts),
            Hash::Sha256 => run::<sha2::Sha256>(parts),
            Hash::Sha384 => run::<sha2::Sha384>(parts),
            Hash::Sha512 => run::<sha2::Sha512>(parts),
        }
    }
}

/// Hashverfahren, das ein Signaturalgorithmus fest vorgibt (z. B. sha256WithRSAEncryption).
fn hash_of_signature_alg(oid: ObjectIdentifier) -> Option<Hash> {
    Some(match oid {
        OID_RSA_SHA1 => Hash::Sha1,
        OID_RSA_SHA256 | OID_ECDSA_SHA256 => Hash::Sha256,
        OID_RSA_SHA384 | OID_ECDSA_SHA384 => Hash::Sha384,
        OID_RSA_SHA512 | OID_ECDSA_SHA512 => Hash::Sha512,
        _ => return None,
    })
}

/// Prüft eine Signatur über `message`.
///
/// `fallback_hash` gilt, wenn der Signaturalgorithmus selbst keinen Hash festlegt
/// (in CMS steht dann z. B. nur "rsaEncryption" und der Hash im SignerInfo).
/// Fehler sind Texte für den Nutzer.
pub fn verify(
    key: &SubjectPublicKeyInfoOwned,
    alg: &AlgorithmIdentifierOwned,
    fallback_hash: Option<Hash>,
    message: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    if alg.oid == OID_RSA_PSS {
        return Err("RSA-PSS-Signaturen werden noch nicht unterstützt".into());
    }
    let hash = hash_of_signature_alg(alg.oid)
        .or(fallback_hash)
        .ok_or_else(|| format!("Unbekanntes Signaturverfahren {}", alg.oid))?;
    let digest = hash.digest([message]);
    verify_prehashed(key, hash, &digest, signature)
}

/// Wie [`verify`], aber mit bereits berechnetem Hash.
pub fn verify_prehashed(
    key: &SubjectPublicKeyInfoOwned,
    hash: Hash,
    digest: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    let key_der = key.to_der().map_err(|e| e.to_string())?;
    let bad = || "Signatur passt nicht zum Schlüssel".to_string();
    match key.algorithm.oid {
        OID_RSA => {
            use rsa::pkcs8::DecodePublicKey;
            let pk = rsa::RsaPublicKey::from_public_key_der(&key_der)
                .map_err(|e| format!("RSA-Schlüssel unlesbar: {e}"))?;
            // PKCS#1 v1.5: Der Hash wird mit seiner Algorithmuskennung verpackt signiert –
            // deshalb muss das Schema den Hash-Typ kennen.
            let scheme = match hash {
                Hash::Sha1 => rsa::Pkcs1v15Sign::new::<sha1::Sha1>(),
                Hash::Sha256 => rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
                Hash::Sha384 => rsa::Pkcs1v15Sign::new::<sha2::Sha384>(),
                Hash::Sha512 => rsa::Pkcs1v15Sign::new::<sha2::Sha512>(),
            };
            pk.verify(scheme, digest, signature).map_err(|_| bad())
        }
        OID_EC_PUBLIC_KEY => {
            use p256::ecdsa::signature::hazmat::PrehashVerifier;
            use p256::pkcs8::DecodePublicKey;
            let curve = key
                .algorithm
                .parameters
                .as_ref()
                .and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
            match curve {
                Some(OID_P256) => {
                    let vk = p256::ecdsa::VerifyingKey::from_public_key_der(&key_der)
                        .map_err(|e| format!("EC-Schlüssel unlesbar: {e}"))?;
                    let sig = p256::ecdsa::Signature::from_der(signature)
                        .map_err(|e| format!("ECDSA-Signatur unlesbar: {e}"))?;
                    vk.verify_prehash(digest, &sig).map_err(|_| bad())
                }
                Some(OID_P384) => {
                    let vk = p384::ecdsa::VerifyingKey::from_public_key_der(&key_der)
                        .map_err(|e| format!("EC-Schlüssel unlesbar: {e}"))?;
                    let sig = p384::ecdsa::Signature::from_der(signature)
                        .map_err(|e| format!("ECDSA-Signatur unlesbar: {e}"))?;
                    vk.verify_prehash(digest, &sig).map_err(|_| bad())
                }
                other => Err(format!(
                    "Elliptische Kurve {} wird nicht unterstützt",
                    other.map_or("?".into(), |o| o.to_string())
                )),
            }
        }
        other => Err(format!("Schlüsseltyp {other} wird nicht unterstützt")),
    }
}

/// Hex-Darstellung (z. B. für Fingerabdrücke und Seriennummern).
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02X}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        // Bekannte Testvektoren für "abc".
        assert_eq!(
            hex(&Hash::Sha256.digest([&b"a"[..], b"bc"])),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
        assert_eq!(
            hex(&Hash::Sha1.digest([&b"abc"[..]])),
            "A9993E364706816ABA3E25717850C26C9CD0D89D"
        );
        assert_eq!(Hash::from_oid(OID_SHA384), Some(Hash::Sha384));
        assert_eq!(Hash::Sha512.oid(), OID_SHA512);
    }
}
