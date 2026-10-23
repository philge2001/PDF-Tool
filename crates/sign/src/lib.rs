//! PDF-Tool-Sign: digitale PDF-Signaturen prüfen und erstellen (PAdES).
//!
//! Eigene Crate, damit `core` frei von Kryptografie-Abhängigkeiten bleibt. Alles hier ist
//! reines Rust (RustCrypto) und ohne PDFium testbar.

pub mod cert;
pub mod create;
pub mod crypto;
pub mod signer;
pub mod timestamp;
pub mod verify;

pub use cert::{CertInfo, Trust, TrustStore};
pub use create::sign_pdf;
pub use signer::{KeyKind, Signer, SignerError};
pub use verify::{Integrity, SignatureReport, Status, unsigned_fields, verify_document};
