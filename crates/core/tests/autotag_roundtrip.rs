//! Barrierefreiheit: Prüfbericht und automatisches Tagging (taggen → neu laden → prüfen).

use pdftool_core::Document;
use pdftool_core::accessibility::{CheckKind, CheckStatus};
use pdftool_core::autotag::{AutoTagOptions, auto_tag_pdf};
use pdftool_core::tags::SetAltText;

fn path(name: &str) -> String {
    format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn status(doc: &Document, kind: CheckKind) -> Option<CheckStatus> {
    doc.accessibility_check()
        .unwrap()
        .into_iter()
        .find(|c| c.kind == kind)
        .map(|c| c.status)
}

fn auto_tag(name: &str, title: &str) -> Document {
    let opts = AutoTagOptions {
        lang: "de-DE".into(),
        title: title.into(),
        embed_fonts: true,
    };
    let bytes = std::fs::read(path(name)).unwrap();
    let (out, report) = auto_tag_pdf(&bytes, &opts, &|_, _| true).unwrap();
    assert!(report.pages > 0);
    Document::from_bytes(out).unwrap()
}

#[test]
fn check_reports_untagged_document() {
    let doc = Document::open(path("text_two_pages.pdf")).unwrap();
    assert_eq!(status(&doc, CheckKind::Tagged), Some(CheckStatus::Fail));
    assert_eq!(status(&doc, CheckKind::Language), Some(CheckStatus::Fail));
    assert_eq!(status(&doc, CheckKind::Title), Some(CheckStatus::Fail));
    assert_eq!(status(&doc, CheckKind::Fonts), Some(CheckStatus::Fail));
    assert_eq!(
        status(&doc, CheckKind::AltText),
        None,
        "ohne Baum keine Prüfung"
    );

    let scan = Document::open(path("scan.pdf")).unwrap();
    assert_eq!(status(&scan, CheckKind::Scanned), Some(CheckStatus::Fail));
}

#[test]
fn check_alt_texts_follow_model() {
    let mut doc = Document::open(path("tagged.pdf")).unwrap();
    assert_eq!(status(&doc, CheckKind::Tagged), Some(CheckStatus::Pass));
    assert_eq!(status(&doc, CheckKind::Language), Some(CheckStatus::Pass));
    assert_eq!(status(&doc, CheckKind::Title), Some(CheckStatus::Pass));
    assert_eq!(
        status(&doc, CheckKind::UntaggedContent),
        Some(CheckStatus::Pass),
        "Rahmen ist Artefakt, alles andere hat eine MCID"
    );
    assert_eq!(status(&doc, CheckKind::AltText), Some(CheckStatus::Fail));
    let tree = doc.structure_tree().unwrap();
    let missing = tree
        .nodes()
        .into_iter()
        .find(|(_, n)| n.is_figure() && n.alt.is_none())
        .and_then(|(_, n)| n.id)
        .unwrap();
    doc.execute(Box::new(SetAltText::new(missing, Some("Verlauf".into()))))
        .unwrap();
    assert_eq!(status(&doc, CheckKind::AltText), Some(CheckStatus::Pass));
}

#[test]
fn auto_tag_builds_structure() {
    let doc = auto_tag("text_two_pages.pdf", "");
    assert!(doc.is_tagged());
    assert_eq!(doc.properties().lang, "de-DE");
    assert!(
        !doc.properties().title.is_empty(),
        "Titel aus erster Überschrift"
    );
    let tree = doc.structure_tree().unwrap();
    let nodes = tree.nodes();
    assert_eq!(nodes[0].1.role, "Document");
    assert!(nodes.iter().any(|(_, n)| n.role == "H1"));
    assert!(nodes.iter().any(|(_, n)| n.role == "P"));
    // Jedes Element verweist auf markierten Inhalt mit Text.
    for (_, n) in &nodes[1..] {
        let (page, mcid) = n.content[0];
        let marked = doc.marked_content(page).unwrap();
        assert!(!marked[&mcid].text.trim().is_empty(), "{}", n.role);
    }
    for c in doc.accessibility_check().unwrap() {
        if c.kind != CheckKind::Fonts {
            assert_eq!(c.status, CheckStatus::Pass, "{}: {}", c.title, c.detail);
        }
    }
    assert_eq!(status(&doc, CheckKind::Fonts), Some(CheckStatus::Pass));
    // Schon getaggt: zweites Mal wird abgelehnt.
    let again = auto_tag_pdf(
        &doc.to_bytes().unwrap(),
        &AutoTagOptions::default(),
        &|_, _| true,
    );
    assert!(again.is_err());
}

#[test]
fn auto_tag_figures_and_annotations() {
    let doc = auto_tag("photo.pdf", "Foto");
    let tree = doc.structure_tree().unwrap();
    assert_eq!(
        tree.nodes().iter().filter(|(_, n)| n.is_figure()).count(),
        1
    );
    assert_eq!(doc.properties().title, "Foto");
    assert_eq!(status(&doc, CheckKind::AltText), Some(CheckStatus::Fail));

    let doc = auto_tag("with_annotations.pdf", "Annotationen");
    let roles: Vec<String> = doc
        .structure_tree()
        .unwrap()
        .nodes()
        .iter()
        .map(|(_, n)| n.role.clone())
        .collect();
    assert!(
        roles.iter().any(|r| r == "Annot" || r == "Link"),
        "{roles:?}"
    );
    assert_eq!(status(&doc, CheckKind::TabOrder), None, "/Tabs /S gesetzt");
    assert_eq!(
        status(&doc, CheckKind::UntaggedContent),
        Some(CheckStatus::Pass)
    );
}
