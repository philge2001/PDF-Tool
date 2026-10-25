//! Seitenoperationen: ausführen -> speichern -> neu laden -> vergleichen.

use pdftool_core::{
    AddAnnotation, AnnotKind, Annotation, Color, DeletePages, Document, Point, ReorderPages,
    RotatePages,
};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn open(name: &str) -> (Vec<u8>, Document) {
    let bytes = std::fs::read(fixture(name)).unwrap();
    let doc = Document::from_bytes(bytes.clone()).unwrap();
    (bytes, doc)
}

/// Speichert in den Speicher und lädt neu.
fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

/// Seitengrößen (wie angezeigt) und Rotationen.
fn layout(doc: &Document) -> Vec<((f32, f32), u16)> {
    doc.pages()
        .iter()
        .map(|p| (p.display_size(), p.rotation))
        .collect()
}

fn note(doc: &mut Document, page: usize, text: &str) {
    let mut a = Annotation::new(
        page,
        AnnotKind::Note {
            pos: Point::new(100.0, 100.0),
        },
        Color::YELLOW,
    );
    a.id = doc.allocate_annotation_id();
    a.contents = text.into();
    doc.execute(Box::new(AddAnnotation::new(a))).unwrap();
}

#[test]
fn rotate_is_saved_incrementally() {
    let (original, mut doc) = open("text_two_pages.pdf");
    doc.execute(Box::new(RotatePages::new(&[0], 1))).unwrap();
    assert!(doc.is_dirty());
    let bytes = doc.to_bytes().unwrap();
    assert_eq!(
        &bytes[..original.len()],
        &original[..],
        "Originalbytes unverändert"
    );
    let back = Document::from_bytes(bytes).unwrap();
    assert_eq!(back.pages()[0].rotation, 90);
    assert_eq!(back.pages()[1].rotation, 0);
}

#[test]
fn reorder_mixed_sizes() {
    let (_, mut doc) = open("mixed_sizes.pdf");
    let before = layout(&doc);
    doc.execute(Box::new(ReorderPages::new(vec![2, 0, 1])))
        .unwrap();
    let back = reload(&doc);
    assert_eq!(layout(&back), vec![before[2], before[0], before[1]]);
}

#[test]
fn reorder_nested_tree_keeps_inherited_attributes() {
    let (_, mut doc) = open("nested_tree.pdf");
    let before = layout(&doc);
    assert_eq!(
        before[2].1, 90,
        "Fixture: Rotation wird vom Zwischenknoten geerbt"
    );
    // Seite 4 nach vorn, Seite 1 nach hinten
    doc.execute(Box::new(ReorderPages::new(vec![3, 1, 2, 0])))
        .unwrap();
    let back = reload(&doc);
    assert_eq!(
        layout(&back),
        vec![before[3], before[1], before[2], before[0]]
    );
    // Ressourcen (Schrift) müssen weiterhin erreichbar sein: jede Seite hat /Resources.
    let pdf = back.pdf();
    for p in back.pages() {
        let res = has_font_resources(pdf, p.object_id);
        assert!(res, "Seite {:?} ohne Ressourcen", p.object_id);
    }
}

/// Hat die Seite (direkt oder geerbt) ein /Resources mit /Font?
fn has_font_resources(pdf: &lopdf::Document, id: lopdf::ObjectId) -> bool {
    let mut current = pdf.get_dictionary(id).ok();
    while let Some(d) = current {
        if let Ok(r) = d.get(b"Resources") {
            let r = pdf.dereference(r).map(|(_, o)| o).unwrap_or(r);
            return r.as_dict().is_ok_and(|r| r.has(b"Font"));
        }
        current = d
            .get(b"Parent")
            .ok()
            .and_then(|p| p.as_reference().ok())
            .and_then(|p| pdf.get_dictionary(p).ok());
    }
    false
}

#[test]
fn delete_page_removes_it_and_its_annotations() {
    let (_, mut doc) = open("text_two_pages.pdf");
    note(&mut doc, 0, "bleibt");
    note(&mut doc, 1, "fliegt raus");
    doc.execute(Box::new(DeletePages::new(&[1]))).unwrap();
    let back = reload(&doc);
    assert_eq!(back.page_count(), 1);
    let contents: Vec<_> = back
        .annotations()
        .iter()
        .map(|a| a.contents.clone())
        .collect();
    assert_eq!(contents, vec!["bleibt"]);
}

#[test]
fn annotations_follow_moved_pages() {
    let (_, mut doc) = open("text_two_pages.pdf");
    note(&mut doc, 1, "auf Seite 2");
    doc.execute(Box::new(ReorderPages::moving(&[1], 0, 2).unwrap()))
        .unwrap();
    assert_eq!(doc.annotations().iter().next().unwrap().page, 0);
    let back = reload(&doc);
    let a = back.annotations().iter().next().unwrap();
    assert_eq!((a.page, a.contents.as_str()), (0, "auf Seite 2"));
}

#[test]
fn undo_after_save_restores_page_order() {
    let path = std::env::temp_dir().join(format!(
        "pdftool-pages-{}.pdf",
        pdftool_core::date::system_now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::copy(fixture("mixed_sizes.pdf"), &path).unwrap();
    let mut doc = Document::open(&path).unwrap();
    let before = layout(&doc);
    doc.execute(Box::new(DeletePages::new(&[0]))).unwrap();
    doc.save().unwrap();
    assert_eq!(Document::open(&path).unwrap().page_count(), 2);

    // Rückgängig nach dem Speichern: Seite kommt zurück (ihr Objekt steht noch in der Datei).
    assert!(doc.undo().unwrap());
    doc.save().unwrap();
    assert_eq!(layout(&Document::open(&path).unwrap()), before);
    let _ = std::fs::remove_file(path);
}

#[test]
fn render_snapshot_hides_managed_annotations() {
    let (_, mut doc) = open("with_annotations.pdf");
    doc.execute(Box::new(RotatePages::new(&[0], 1))).unwrap();
    let snap = doc.render_snapshot().unwrap();
    // Seite unverändert bei den Annotationen: Highlight (0) und Notiz (1) verwaltet, Link (2) nicht.
    assert_eq!(snap.hidden.get(&0), Some(&vec![0, 1]));
    let reloaded = Document::from_bytes(snap.bytes).unwrap();
    assert_eq!(reloaded.pages()[0].rotation, 90);
}
