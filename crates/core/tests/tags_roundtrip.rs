//! Tagged PDF: Strukturbaum lesen, Titel/Sprache/Alternativtext ändern → speichern → neu laden.

use foliocraft_core::Document;
use foliocraft_core::tags::{DocProperties, SetAltText, SetProperties};

fn fixture(name: &str) -> Document {
    Document::open(format!(
        "{}/../../tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

#[test]
fn reads_structure_with_text_and_alt() {
    let doc = fixture("tagged.pdf");
    assert!(doc.is_tagged());
    assert!(!fixture("text_two_pages.pdf").is_tagged());
    assert!(fixture("text_two_pages.pdf").structure_tree().is_none());

    let tree = doc.structure_tree().unwrap();
    let nodes = tree.nodes();
    let roles: Vec<&str> = nodes.iter().map(|(_, n)| n.role.as_str()).collect();
    assert_eq!(roles, vec!["Document", "H1", "P", "Figure", "P", "Figure"]);
    assert_eq!(nodes[1].0, 1, "Tiefe unter Document");
    let figs: Vec<_> = nodes.iter().filter(|(_, n)| n.is_figure()).collect();
    assert_eq!(figs[0].1.alt.as_deref(), Some("Graustufen-Verlauf"));
    assert_eq!(figs[1].1.alt, None);

    // Inhalt über MCID: Text der Überschrift, Rechteck der Abbildung.
    let marked = doc.marked_content(0).unwrap();
    let (page, mcid) = nodes[1].1.content[0];
    assert_eq!(page, 0);
    assert_eq!(marked[&mcid].text, "Barrierefreies Dokument");
    let fig = &marked[&figs[0].1.content[0].1];
    assert!(fig.has_image);
    let b = fig.bounds.unwrap();
    assert!(
        (b.x0 - 72.0).abs() < 0.01 && (b.y1 - 700.0).abs() < 0.01,
        "{b:?}"
    );
    // Der Rahmen ist ein Artefakt (ohne MCID) und zählt nicht.
    assert_eq!(marked.len(), 5);

    assert_eq!(doc.properties().title, "Barrierefreies Testdokument");
    assert_eq!(doc.properties().lang, "de-DE");
}

#[test]
fn edits_are_undoable_and_saved() {
    let mut doc = fixture("tagged.pdf");
    let tree = doc.structure_tree().unwrap();
    let fig2 = tree
        .nodes()
        .iter()
        .filter(|(_, n)| n.is_figure())
        .nth(1)
        .unwrap()
        .1
        .id
        .unwrap();
    let before = doc.properties().clone();
    let after = DocProperties {
        title: "Neuer Titel – Ä".into(),
        lang: "en-GB".into(),
    };
    doc.execute(Box::new(SetProperties::new(before.clone(), after.clone())))
        .unwrap();
    doc.execute(Box::new(SetAltText::new(
        fig2,
        Some("Kleines Quadrat".into()),
    )))
    .unwrap();
    // Sofort sichtbar (ungespeichert).
    let alt_now = doc
        .structure_tree()
        .unwrap()
        .nodes()
        .iter()
        .filter(|(_, n)| n.is_figure())
        .nth(1)
        .unwrap()
        .1
        .alt
        .clone();
    assert_eq!(alt_now.as_deref(), Some("Kleines Quadrat"));

    let saved = reload(&doc);
    assert_eq!(*saved.properties(), after);
    let figs: Vec<Option<String>> = saved
        .structure_tree()
        .unwrap()
        .nodes()
        .iter()
        .filter(|(_, n)| n.is_figure())
        .map(|(_, n)| n.alt.clone())
        .collect();
    assert_eq!(
        figs,
        vec![
            Some("Graustufen-Verlauf".into()),
            Some("Kleines Quadrat".into())
        ]
    );
    // DisplayDocTitle gesetzt.
    let prefs = saved
        .pdf()
        .catalog()
        .unwrap()
        .get(b"ViewerPreferences")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    assert!(prefs.get(b"DisplayDocTitle").unwrap().as_bool().unwrap());

    // Undo: beides zurück.
    doc.undo().unwrap();
    doc.undo().unwrap();
    let back = reload(&doc);
    assert_eq!(*back.properties(), before);
    assert_eq!(
        back.structure_tree()
            .unwrap()
            .nodes()
            .iter()
            .filter(|(_, n)| n.is_figure())
            .nth(1)
            .unwrap()
            .1
            .alt,
        None
    );

    // Titel für ein Dokument ohne /Info setzen.
    let mut plain = fixture("text_two_pages.pdf");
    let b = plain.properties().clone();
    plain
        .execute(Box::new(SetProperties::new(
            b,
            DocProperties {
                title: "Titel".into(),
                lang: "de".into(),
            },
        )))
        .unwrap();
    assert_eq!(reload(&plain).properties().title, "Titel");
}
