//! Signatur erstellen (PAdES Baseline B-B).
//!
//! Die CMS-Struktur (SignedData) enthält:
//! - die signierten Attribute: Inhaltstyp, Hash der Datei (messageDigest) und
//!   `signing-certificate-v2` (Hash des Unterzeichnerzertifikats – verhindert, dass jemand
//!   das Zertifikat austauscht). Eine Signaturzeit steht laut PAdES NICHT darin, sondern in /M.
//! - die Signatur über diese Attribute,
//! - das Zertifikat und die Kette, damit Prüfer sie nicht suchen müssen.

use crate::crypto::Hash;
use crate::signer::{Signer, SignerError};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedAttributes, SignedData, SignerIdentifier,
    SignerInfo, SignerInfos,
};
use der::asn1::{Any, ObjectIdentifier, OctetString, SetOfVec};
use der::{Decode, Encode, Sequence};
use foliocraft_core::signature_prep::{self, SignatureSpec};
use spki::AlgorithmIdentifierOwned;
use x509_cert::attr::Attribute;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::serial_number::SerialNumber;

pub const OID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
pub const OID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const OID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const OID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const OID_SIGNING_CERT_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");

/// Platz für die CMS-Signatur in der Datei. Reicht für RSA-4096 mit einer Kette aus ein paar
/// Zertifikaten und einem Zeitstempel.
pub const DEFAULT_CONTENTS_LEN: usize = 24 * 1024;

// ---------- ESS signing-certificate-v2 (RFC 5035) ----------
// `#[derive(Sequence)]` erzeugt die DER-Kodierung aus der Struktur – Feld für Feld in
// der angegebenen Reihenfolge, so wie die ASN.1-Definition es vorgibt.

/// IssuerSerial ::= SEQUENCE { issuer GeneralNames, serialNumber CertificateSerialNumber }
#[derive(Clone, Debug, Sequence)]
struct IssuerSerial {
    issuer: Vec<GeneralName>,
    serial_number: SerialNumber,
}

/// ESSCertIDv2 ::= SEQUENCE { hashAlgorithm DEFAULT sha256, certHash, issuerSerial OPTIONAL }
/// (hashAlgorithm lassen wir weg: SHA-256 ist der Standardwert und darf in DER nicht stehen.)
#[derive(Clone, Debug, Sequence)]
struct EssCertIdV2 {
    cert_hash: OctetString,
    issuer_serial: IssuerSerial,
}

/// SigningCertificateV2 ::= SEQUENCE { certs SEQUENCE OF ESSCertIDv2 }
#[derive(Clone, Debug, Sequence)]
struct SigningCertificateV2 {
    certs: Vec<EssCertIdV2>,
}

fn attribute(oid: ObjectIdentifier, value: impl Encode) -> Result<Attribute, SignerError> {
    // Erst kodieren, dann als "beliebiger ASN.1-Wert" (Any) wieder einlesen.
    let any = Any::from_der(&value.to_der().map_err(other)?).map_err(other)?;
    Ok(Attribute {
        oid,
        values: SetOfVec::try_from(vec![any]).map_err(other)?,
    })
}

fn other(e: impl std::fmt::Display) -> SignerError {
    SignerError::Other(e.to_string())
}

/// Baut die CMS-Signatur (DER) für den Hash `digest` der signierten Bytes.
pub fn build_cms(signer: &Signer, hash: Hash, digest: &[u8]) -> Result<Vec<u8>, SignerError> {
    let cert = &signer.cert;
    let cert_der = cert.to_der().map_err(other)?;
    let ess = SigningCertificateV2 {
        certs: vec![EssCertIdV2 {
            cert_hash: OctetString::new(Hash::Sha256.digest([cert_der.as_slice()]))
                .map_err(other)?,
            issuer_serial: IssuerSerial {
                issuer: vec![GeneralName::DirectoryName(
                    cert.tbs_certificate.issuer.clone(),
                )],
                serial_number: cert.tbs_certificate.serial_number.clone(),
            },
        }],
    };
    // `SetOfVec::try_from` sortiert die Attribute so, wie DER es für ein SET verlangt.
    let attrs: SignedAttributes = SetOfVec::try_from(vec![
        attribute(OID_CONTENT_TYPE, OID_DATA)?,
        attribute(OID_MESSAGE_DIGEST, OctetString::new(digest).map_err(other)?)?,
        attribute(OID_SIGNING_CERT_V2, ess)?,
    ])
    .map_err(other)?;

    // Signiert wird die DER-Kodierung der Attribute als SET.
    let attrs_der = attrs.to_der().map_err(other)?;
    let (sig_alg, signature) = signer
        .key
        .sign_prehashed(&hash.digest([attrs_der.as_slice()]))?;
    let digest_alg = AlgorithmIdentifierOwned {
        oid: hash.oid(),
        parameters: None,
    };
    let signer_info = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: cert.tbs_certificate.issuer.clone(),
            serial_number: cert.tbs_certificate.serial_number.clone(),
        }),
        digest_alg: digest_alg.clone(),
        signed_attrs: Some(attrs),
        signature_algorithm: sig_alg,
        signature: OctetString::new(signature).map_err(other)?,
        unsigned_attrs: None,
    };
    let certs: Vec<CertificateChoices> = std::iter::once(cert)
        .chain(&signer.chain)
        .map(|c| CertificateChoices::Certificate(c.clone()))
        .collect();
    let signed_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![digest_alg]).map_err(other)?,
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: OID_DATA,
            econtent: None, // "detached": die PDF-Bytes stehen nicht in der Signatur
        },
        certificates: Some(CertificateSet(SetOfVec::try_from(certs).map_err(other)?)),
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(vec![signer_info]).map_err(other)?),
    };
    let info = ContentInfo {
        content_type: OID_SIGNED_DATA,
        content: Any::encode_from(&signed_data).map_err(other)?,
    };
    info.to_der().map_err(other)
}

/// Signiert: hängt an `base` (fertige PDF-Bytes) eine Revision mit Signatur an.
/// `spec.sub_filter` und `spec.contents_len` werden hier gesetzt.
pub fn sign_pdf(
    base: &[u8],
    mut spec: SignatureSpec,
    signer: &Signer,
) -> Result<Vec<u8>, SignerError> {
    spec.sub_filter = "ETSI.CAdES.detached".into();
    if spec.contents_len == 0 {
        spec.contents_len = DEFAULT_CONTENTS_LEN;
    }
    if spec.name.is_empty() {
        spec.name = signer.info().name;
    }
    let prepared = signature_prep::prepare_signature(base, &spec).map_err(other)?;
    let hash = signer.key.hash();
    let digest = hash.digest(prepared.signed_parts());
    let cms = build_cms(signer, hash, &digest)?;
    prepared.finish(&cms).map_err(other)
}
