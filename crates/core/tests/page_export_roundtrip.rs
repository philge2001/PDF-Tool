//! Seiten extrahieren: neue, eigenständige PDF mit genau den gewünschten Seiten.

use pdftool_core::{
    AddAnnotation, AnnotKind, Annotation, Color, Document, Point, RotatePages, extract_pages,
};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn text(doc: &Document, page: usize) -> String {
    doc.pdf()
        .extract_text(&[page as u32 + 1])
        .unwrap_or_default()
}

#[test]
fn extract_single_page() {
    let doc = Document::open(fixture("text_two_pages.pdf")).unwrap();
    let bytes = extract_pages(&doc.snapshot_pdf().unwrap(), &[1]).unwrap();
    let out = Document::from_bytes(bytes).unwrap();
    assert_eq!(out.page_count(), 1);
    assert!(text(&out, 0).contains("Seite zwei"));
    // Nur die Objekte der einen Seite: Katalog, Seitenbaum, Seite, Inhalt, Schrift, Info.
    assert!(
        out.pdf().objects.len() <= 7,
        "{} Objekte",
        out.pdf().objects.len()
    );
}

#[test]
fn extract_takes_unsaved_changes_along() {
    let mut doc = Document::open(fixture("text_two_pages.pdf")).unwrap();
    doc.execute(Box::new(RotatePages::new(&[0], 1))).unwrap();
    let mut a = Annotation::new(
        0,
        AnnotKind::Note {
            pos: Point::new(50.0, 50.0),
        },
        Color::YELLOW,
    );
    a.id = doc.allocate_annotation_id();
    a.contents = "kommt mit".into();
    doc.execute(Box::new(AddAnnotation::new(a))).unwrap();

    let bytes = extract_pages(&doc.snapshot_pdf().unwrap(), &[0]).unwrap();
    let out = Document::from_bytes(bytes).unwrap();
    assert_eq!(out.pages()[0].rotation, 90);
    let notes: Vec<_> = out
        .annotations()
        .iter()
        .map(|a| a.contents.clone())
        .collect();
    assert_eq!(notes, vec!["kommt mit"]);
}

#[test]
fn extract_from_nested_tree_in_new_order() {
    let doc = Document::open(fixture("nested_tree.pdf")).unwrap();
    let bytes = extract_pages(&doc.snapshot_pdf().unwrap(), &[3, 0]).unwrap();
    let out = Document::from_bytes(bytes).unwrap();
    assert_eq!(out.page_count(), 2);
    assert!(text(&out, 0).contains("Seite 4"));
    assert!(text(&out, 1).contains("Seite 1"));
    assert_eq!(out.pages()[0].rotation, 90, "geerbte Rotation übernommen");
}

#[test]
fn extract_keeps_annotations_and_links() {
    // with_annotations: Highlight + Notiz (verwaltet) und ein URI-Link (fremd).
    let doc = Document::open(fixture("with_annotations.pdf")).unwrap();
    let bytes = extract_pages(&doc.snapshot_pdf().unwrap(), &[0]).unwrap();
    let out = Document::from_bytes(bytes).unwrap();
    assert_eq!(out.annotations().len(), 2);
}
