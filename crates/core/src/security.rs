//! Passwortschutz (Phase 9): verschlüsselte PDFs öffnen, Schutz setzen und entfernen.
//!
//! **Arbeitsweise:** Ein verschlüsseltes Dokument wird beim Öffnen entschlüsselt; intern
//! arbeitet FolioCraft auf einer Klartext-Fassung (Anzeige, Bearbeiten, inkrementelles
//! Speichern im Speicher). Erst beim Schreiben der Datei wird wieder verschlüsselt:
//! - unverändert mit dem **ursprünglichen Schlüssel** (gleiche Passwörter, gleiche
//!   Berechtigungen – auch wenn nur das Benutzerpasswort bekannt ist),
//! - oder mit einem neu gesetzten Schutz (AES-256, PDF 2.0, R6).
//!
//! Weil der Schlüssel neu angewendet wird, ist das Speichern geschützter Dateien immer ein
//! komplettes Neuschreiben (vorhandene Signaturen werden dadurch ungültig).
//!
//! **Berechtigungen** sind in PDF eine Bitte an Programme, keine Kryptografie. FolioCraft hält
//! sich daran: Wer das Besitzerpasswort nicht kennt, kann ein Dokument, das keine Änderungen
//! erlaubt, nicht speichern.

use crate::error::{CoreError, Result};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{EncryptionState, EncryptionVersion, Object, StringFormat};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Was ein Dokument erlaubt (vereinfacht; Barrierefreiheit wird immer erlaubt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    pub print: bool,
    /// Inhalt ändern, Seiten einfügen/löschen/drehen.
    pub modify: bool,
    /// Text und Bilder kopieren.
    pub copy: bool,
    /// Kommentieren und Formulare ausfüllen.
    pub annotate: bool,
}

impl Permissions {
    pub const ALL: Permissions = Permissions {
        print: true,
        modify: true,
        copy: true,
        annotate: true,
    };

    pub fn is_restricted(&self) -> bool {
        *self != Self::ALL
    }

    fn from_lopdf(p: lopdf::Permissions) -> Self {
        use lopdf::Permissions as P;
        Self {
            print: p.contains(P::PRINTABLE),
            modify: p.contains(P::MODIFIABLE),
            copy: p.contains(P::COPYABLE),
            annotate: p.contains(P::ANNOTABLE) || p.contains(P::FILLABLE),
        }
    }

    fn to_lopdf(self) -> lopdf::Permissions {
        use lopdf::Permissions as P;
        let mut p = P::COPYABLE_FOR_ACCESSIBILITY;
        if self.print {
            p |= P::PRINTABLE | P::PRINTABLE_IN_HIGH_QUALITY;
        }
        if self.modify {
            p |= P::MODIFIABLE | P::ASSEMBLABLE;
        }
        if self.copy {
            p |= P::COPYABLE;
        }
        if self.annotate {
            p |= P::ANNOTABLE | P::FILLABLE;
        }
        p
    }

    /// Kurzbeschreibung für die Anzeige ("Drucken, Kopieren").
    pub fn describe(&self) -> String {
        let mut v = Vec::new();
        if self.print {
            v.push("Drucken");
        }
        if self.modify {
            v.push("Ändern");
        }
        if self.copy {
            v.push("Kopieren");
        }
        if self.annotate {
            v.push("Kommentieren/Formulare");
        }
        if v.is_empty() {
            "nichts".into()
        } else {
            v.join(", ")
        }
    }
}

/// Gewünschter neuer Schutz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protection {
    /// Passwort zum Öffnen; leer = öffnen ohne Passwort (nur Berechtigungen).
    pub user_password: String,
    /// Passwort für volle Rechte (Pflicht, nicht leer).
    pub owner_password: String,
    pub permissions: Permissions,
}

/// Schutz eines geöffneten Dokuments.
#[derive(Debug, Clone)]
pub struct Security {
    /// Wie beim Speichern verschlüsselt wird.
    state: EncryptionState,
    /// /ID des Trailers (für ältere Verfahren Teil der Schlüsselberechnung).
    id: Option<Object>,
    pub permissions: Permissions,
    /// Ist das Besitzerpasswort bekannt (= volle Rechte)?
    pub owner_known: bool,
    /// Braucht das Öffnen ein Passwort?
    pub needs_password: bool,
    /// Kurzname des Verfahrens, z. B. "AES-256".
    pub method: String,
}

impl Security {
    /// Darf dieses Dokument gespeichert werden?
    pub fn can_save(&self) -> bool {
        self.owner_known || self.permissions.modify
    }
}

/// Mini-Dokument, das nur das Verschlüsselungs-Dictionary enthält – reicht, um Passwörter
/// zu prüfen, nachdem lopdf das Original-Dictionary beim Entschlüsseln entfernt hat.
fn auth_doc(state: &EncryptionState, id: Option<&Object>) -> Option<lopdf::Document> {
    let dict = state.encode().ok()?;
    let mut d = lopdf::Document::with_version("1.7");
    let oid = d.add_object(dict);
    d.trailer.set("Encrypt", oid);
    if let Some(id) = id {
        d.trailer.set("ID", id.clone());
    }
    Some(d)
}

fn method_name(state: &EncryptionState) -> String {
    match (state.version(), state.revision()) {
        (5, _) => "AES-256".into(),
        (4, _) => "AES-128 / RC4-128".into(),
        (2, _) => "RC4-128".into(),
        _ => "RC4-40".into(),
    }
}

/// Lädt eine (evtl. verschlüsselte) PDF. Liefert das entschlüsselte lopdf-Dokument und – bei
/// verschlüsselten Dateien – den Schutz.
pub(crate) fn load(
    bytes: &[u8],
    password: Option<&str>,
) -> Result<(lopdf::Document, Option<Security>)> {
    let pdf = match password {
        Some(pw) => {
            lopdf::Document::load_mem_with_options(bytes, lopdf::LoadOptions::with_password(pw))
        }
        None => lopdf::Document::load_mem(bytes),
    }
    .map_err(|e| match e {
        lopdf::Error::InvalidPassword => CoreError::WrongPassword,
        e => CoreError::Pdf(e),
    })?;
    if pdf.is_encrypted() {
        // Noch verschlüsselt: Das leere Passwort reicht nicht.
        return Err(if password.is_some() {
            CoreError::WrongPassword
        } else {
            CoreError::PasswordRequired
        });
    }
    let Some(state) = pdf.encryption_state.clone() else {
        return Ok((pdf, None));
    };
    let id = pdf.trailer.get(b"ID").ok().cloned();
    let used = password.unwrap_or("");
    let auth = auth_doc(&state, id.as_ref());
    let owner_known = auth
        .as_ref()
        .is_some_and(|d| d.authenticate_owner_password(used).is_ok());
    let needs_password = auth
        .as_ref()
        .is_none_or(|d| d.authenticate_user_password("").is_err());
    let security = Security {
        permissions: Permissions::from_lopdf(state.permissions()),
        method: method_name(&state),
        state,
        id,
        owner_known,
        needs_password,
    };
    Ok((pdf, Some(security)))
}

impl Security {
    /// Prüft das Besitzerpasswort (z. B. um ein Dokument mit Einschränkungen zu ändern).
    pub fn unlock(&mut self, owner_password: &str) -> bool {
        let ok = auth_doc(&self.state, self.id.as_ref())
            .is_some_and(|d| d.authenticate_owner_password(owner_password).is_ok());
        if ok {
            self.owner_known = true;
        }
        ok
    }

    /// Neuer Schutz: AES-256 (R6) mit zufälligem Dateischlüssel.
    pub fn new(protection: &Protection) -> Result<Self> {
        if protection.owner_password.is_empty() {
            return Err(CoreError::Other(
                "Das Besitzerpasswort darf nicht leer sein.".into(),
            ));
        }
        let mut key = [0u8; 32];
        getrandom::fill(&mut key)
            .map_err(|e| CoreError::Other(format!("Zufallszahlen nicht verfügbar: {e}")))?;
        let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
        let mut filters = BTreeMap::new();
        filters.insert(b"StdCF".to_vec(), filter);
        let version = EncryptionVersion::V5 {
            encrypt_metadata: true,
            crypt_filters: filters,
            file_encryption_key: &key,
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: &protection.owner_password,
            user_password: &protection.user_password,
            permissions: protection.permissions.to_lopdf(),
        };
        let state = EncryptionState::try_from(version)
            .map_err(|e| CoreError::Other(format!("Verschlüsselung: {e}")))?;
        Ok(Self {
            method: method_name(&state),
            state,
            id: None,
            permissions: protection.permissions,
            owner_known: true,
            needs_password: !protection.user_password.is_empty(),
        })
    }

    /// Verschlüsselt eine Klartext-PDF (komplett neu geschrieben).
    pub(crate) fn encrypt(&self, plain: &[u8]) -> Result<Vec<u8>> {
        let mut pdf = lopdf::Document::load_mem(plain)?;
        // Alte Fassungen gehören nicht in die verschlüsselte Datei (sie lägen im Klartext darin).
        pdf.trailer.remove(b"Prev");
        pdf.trailer.remove(b"XRefStm");
        pdf.trailer.remove(b"Encrypt");
        pdf.prune_objects();
        pdf.compress();
        // /ID: die ursprüngliche (ältere Verfahren rechnen damit), sonst eine neue.
        let id = self.id.clone().unwrap_or_else(|| {
            let mut b = [0u8; 16];
            let _ = getrandom::fill(&mut b);
            let s = Object::String(b.to_vec(), StringFormat::Hexadecimal);
            Object::Array(vec![s.clone(), s])
        });
        pdf.trailer.set("ID", id);
        pdf.encrypt(&self.state)
            .map_err(|e| CoreError::Other(format!("Verschlüsseln: {e}")))?;
        // Einige Programme (z. B. Poppler) brauchen die Schlüssellänge im Krypto-Filter.
        if let Ok(eid) = pdf.trailer.get(b"Encrypt").and_then(Object::as_reference)
            && let Ok(Object::Dictionary(e)) = pdf.get_object_mut(eid)
            && let Ok(Object::Dictionary(cf)) = e.get_mut(b"CF")
        {
            for (_, f) in cf.iter_mut() {
                if let Object::Dictionary(f) = f {
                    let len = match f.get(b"CFM").and_then(Object::as_name) {
                        Ok(b"AESV3") => Some(32),
                        Ok(b"AESV2") => Some(16),
                        _ => None,
                    };
                    if let Some(len) = len {
                        if !f.has(b"Length") {
                            f.set("Length", len);
                        }
                        if !f.has(b"AuthEvent") {
                            f.set("AuthEvent", "DocOpen");
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        pdf.save_to(&mut out)
            .map_err(|e| CoreError::Other(format!("PDF schreiben: {e}")))?;
        Ok(out)
    }
}

/// Schreibt eine entschlüsselte PDF als Klartext neu (Basis für das Arbeiten im Speicher).
pub(crate) fn plain_bytes(pdf: &lopdf::Document) -> Result<Vec<u8>> {
    let mut pdf = pdf.clone();
    pdf.encryption_state = None;
    pdf.trailer.remove(b"Encrypt");
    pdf.trailer.remove(b"Prev");
    pdf.trailer.remove(b"XRefStm");
    let mut out = Vec::new();
    pdf.save_to(&mut out)
        .map_err(|e| CoreError::Other(format!("PDF schreiben: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_roundtrip() {
        let p = Permissions {
            print: true,
            modify: false,
            copy: false,
            annotate: true,
        };
        assert_eq!(Permissions::from_lopdf(p.to_lopdf()), p);
        assert!(p.is_restricted());
        assert!(!Permissions::ALL.is_restricted());
        assert_eq!(p.describe(), "Drucken, Kommentieren/Formulare");
    }
}
