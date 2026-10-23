//! Passwortschutz: schützen → speichern → mit/ohne Passwort öffnen → ändern → Schutz entfernen.

use foliocraft_core::security::{Permissions, Protection};
use foliocraft_core::{CoreError, Document};
use std::path::PathBuf;

fn fixture(name: &str) -> Document {
    Document::open(format!(
        "{}/../../tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fc_sec_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("doc.pdf")
}

fn first_text(doc: &Document) -> String {
    doc.text_blocks(0).unwrap()[0].text.clone()
}

/// Schützt text_two_pages.pdf und speichert es.
fn protected(name: &str, user: &str, permissions: Permissions) -> PathBuf {
    let mut doc = fixture("text_two_pages.pdf");
    doc.set_protection(Some(&Protection {
        user_password: user.into(),
        owner_password: "besitzer".into(),
        permissions,
    }))
    .unwrap();
    assert!(doc.is_dirty(), "Schutz ändern = ungespeicherte Änderung");
    let path = tmp(name);
    doc.save_as(&path).unwrap();
    path
}

const READ_ONLY: Permissions = Permissions {
    print: true,
    modify: false,
    copy: false,
    annotate: false,
};

#[test]
fn open_with_and_without_password() {
    let path = protected("open", "geheim", READ_ONLY);
    let raw = std::fs::read(&path).unwrap();
    assert!(raw.windows(8).any(|w| w == b"/Encrypt"));
    assert!(
        !raw.windows(12).any(|w| w == b"Testdokument"),
        "Inhalt verschlüsselt"
    );

    assert!(matches!(
        Document::open(&path),
        Err(CoreError::PasswordRequired)
    ));
    assert!(matches!(
        Document::open_with_password(&path, Some("falsch")),
        Err(CoreError::WrongPassword)
    ));

    // Benutzerpasswort: lesen ja, speichern nein (keine Änderungen erlaubt).
    let mut doc = Document::open_with_password(&path, Some("geheim")).unwrap();
    assert_eq!(first_text(&doc), "FolioCraft Testdokument");
    let sec = doc.security().unwrap();
    assert!(sec.needs_password && !sec.owner_known && !sec.can_save());
    assert_eq!(sec.permissions, READ_ONLY);
    assert_eq!(sec.method, "AES-256");
    assert!(doc.save_as(tmp("open2")).is_err());
    assert!(
        doc.set_protection(None).is_err(),
        "Schutz entfernen braucht Besitzerpasswort"
    );

    // Besitzerpasswort nachreichen -> ändern und speichern; Schutz bleibt unverändert.
    assert!(!doc.unlock("falsch"));
    assert!(doc.unlock("besitzer"));
    let b = doc.text_blocks(0).unwrap()[0].clone();
    doc.edit_text_block(0, &b, "Geändert").unwrap();
    let path2 = tmp("open3");
    doc.save_as(&path2).unwrap();
    let again = Document::open_with_password(&path2, Some("geheim")).unwrap();
    assert_eq!(first_text(&again), "Geändert");
    assert_eq!(again.security().unwrap().permissions, READ_ONLY);

    // Mit dem Besitzerpasswort öffnen = volle Rechte.
    let owner = Document::open_with_password(&path2, Some("besitzer")).unwrap();
    assert!(owner.security().unwrap().owner_known);
}

#[test]
fn permissions_only_and_removing_protection() {
    // Ohne Benutzerpasswort: öffnet ohne Nachfrage, Berechtigungen gelten trotzdem.
    let path = protected("perm", "", READ_ONLY);
    let mut doc = Document::open(&path).unwrap();
    let sec = doc.security().unwrap();
    assert!(!sec.needs_password && !sec.owner_known);
    assert!(doc.unlock("besitzer"));
    doc.set_protection(None).unwrap();
    let plain = tmp("perm2");
    doc.save_as(&plain).unwrap();
    let raw = std::fs::read(&plain).unwrap();
    assert!(!raw.windows(8).any(|w| w == b"/Encrypt"), "Schutz entfernt");
    assert!(Document::open(&plain).unwrap().security().is_none());
}

#[test]
fn unrestricted_document_can_be_saved_with_user_password() {
    let path = protected("free", "geheim", Permissions::ALL);
    let mut doc = Document::open_with_password(&path, Some("geheim")).unwrap();
    assert!(doc.security().unwrap().can_save());
    let b = doc.text_blocks(0).unwrap()[0].clone();
    doc.edit_text_block(0, &b, "Neu").unwrap();
    let path2 = tmp("free2");
    doc.save_as(&path2).unwrap();
    // Gleiche Passwörter wie vorher (ursprünglicher Schlüssel wiederverwendet).
    assert!(
        Document::open_with_password(&path2, Some("besitzer"))
            .unwrap()
            .security()
            .unwrap()
            .owner_known
    );
    assert_eq!(
        first_text(&Document::open_with_password(&path2, Some("geheim")).unwrap()),
        "Neu"
    );
}
