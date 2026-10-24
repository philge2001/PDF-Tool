//! Seiten einfügen (aus anderen PDFs, leere Seiten): einfügen -> speichern -> neu laden -> prüfen.

use pdftool_core::annotation::import::page_annots;
use pdftool_core::{AnnotKind, Document};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn open(name: &str) -> Document {
    Document::open(fixture(name)).unwrap()
}

fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

/// Text einer Seite (0-basiert) über lopdf.
fn page_text(doc: &Document, page: usize) -> String {
    doc.pdf()
        .extract_text(&[page as u32 + 1])
        .unwrap_or_default()
}

#[test]
fn insert_page_from_other_pdf_with_annotations() {
    let mut doc = open("text_two_pages.pdf");
    let source = open("with_annotations.pdf");
    let cmd = doc.prepare_insert(&source, &[0], 1).unwrap();
    doc.execute(Box::new(cmd)).unwrap();
    assert_eq!(doc.page_count(), 3);
    // Annotationen der eingefügten Seite sind sofort im Modell (auf Seite 1).
    let on_new: Vec<_> = doc.annotations().on_page(1).collect();
    assert_eq!(on_new.len(), 2);

    let back = reload(&doc);
    assert_eq!(back.page_count(), 3);
    assert!(page_text(&back, 0).contains("PDF Tool Testdokument"));
    assert!(page_text(&back, 1).contains("vorhandenen Annotationen"));
    assert!(page_text(&back, 2).contains("Seite zwei"));

    // Highlight + Notiz verwaltet, der Link ist als fremde Annotation mitgekommen.
    let kinds: Vec<_> = back
        .annotations()
        .on_page(1)
        .map(|a| a.kind.label())
        .collect();
    assert_eq!(kinds, vec!["Hervorhebung", "Notiz"]);
    let pdf = back.pdf();
    let has_link = page_annots(pdf, back.pages()[1].object_id).iter().any(|o| {
        let d = pdf.get_dictionary(o.as_reference().unwrap()).unwrap();
        d.get(b"Subtype").unwrap().as_name().unwrap() == b"Link"
    });
    assert!(has_link);
}

#[test]
fn inserted_pages_keep_inherited_resources() {
    let mut doc = open("mixed_sizes.pdf");
    let source = open("nested_tree.pdf");
    // Seite 3 der Quelle erbt MediaBox, Schrift und Rotation von ihrem Zwischenknoten.
    let cmd = doc.prepare_insert(&source, &[2, 0], 3).unwrap();
    doc.execute(Box::new(cmd)).unwrap();
    let back = reload(&doc);
    assert_eq!(back.page_count(), 5);
    assert_eq!(back.pages()[3].rotation, 90);
    assert_eq!(
        back.pages()[3].display_size(),
        source.pages()[2].display_size()
    );
    assert!(page_text(&back, 3).contains("Seite 3 im verschachtelten Baum"));
    assert!(page_text(&back, 4).contains("Seite 1 im verschachtelten Baum"));
}

#[test]
fn same_source_twice_gives_distinct_objects() {
    let mut doc = open("text_two_pages.pdf");
    let source = open("text_two_pages.pdf");
    let a = doc.prepare_insert(&source, &[0], 2).unwrap();
    doc.execute(Box::new(a)).unwrap();
    let b = doc.prepare_insert(&source, &[0], 3).unwrap();
    doc.execute(Box::new(b)).unwrap();
    let ids: Vec<_> = doc.pages().iter().map(|p| p.object_id).collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 4);
    assert_eq!(reload(&doc).page_count(), 4);
}

#[test]
fn blank_page_and_undo() {
    let mut doc = open("text_two_pages.pdf");
    let cmd = doc.prepare_blank_page(2, (300.0, 400.0));
    doc.execute(Box::new(cmd)).unwrap();
    let back = reload(&doc);
    assert_eq!(back.page_count(), 3);
    assert_eq!(back.pages()[2].display_size(), (300.0, 400.0));

    doc.undo().unwrap();
    assert_eq!(doc.page_count(), 2);
    assert!(doc.model().extra_objects.is_empty());
    assert_eq!(reload(&doc).page_count(), 2);
}

#[test]
fn object_ids_never_reused_after_undo() {
    let mut doc = open("text_two_pages.pdf");
    let a = doc.prepare_blank_page(0, (100.0, 100.0));
    doc.execute(Box::new(a)).unwrap();
    let first: Vec<_> = doc.model().extra_objects.keys().copied().collect();
    doc.undo().unwrap();
    let b = doc.prepare_blank_page(0, (100.0, 100.0));
    doc.execute(Box::new(b)).unwrap();
    let second: Vec<_> = doc.model().extra_objects.keys().copied().collect();
    assert!(first.iter().all(|id| !second.contains(id)));
}

#[test]
fn annotations_on_existing_pages_shift_and_save_after_insert() {
    let mut doc = open("with_annotations.pdf");
    let source = open("text_two_pages.pdf");
    let cmd = doc.prepare_insert(&source, &[0, 1], 0).unwrap();
    doc.execute(Box::new(cmd)).unwrap();
    // Die beiden vorhandenen Annotationen liegen jetzt auf Seite 2 (0-basiert).
    assert!(doc.annotations().iter().all(|a| a.page == 2));
    let back = reload(&doc);
    assert_eq!(back.page_count(), 3);
    let kinds: Vec<_> = back.annotations().on_page(2).map(|a| &a.kind).collect();
    assert!(matches!(kinds[0], AnnotKind::Highlight { .. }));
    assert_eq!(kinds.len(), 2);
}

#[test]
fn save_to_file_then_undo_insert() {
    let path = std::env::temp_dir().join(format!(
        "pdftool-insert-{}.pdf",
        pdftool_core::date::system_now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::copy(fixture("text_two_pages.pdf"), &path).unwrap();
    let mut doc = Document::open(&path).unwrap();
    let source = open("mixed_sizes.pdf");
    let cmd = doc.prepare_insert(&source, &[1], 2).unwrap();
    doc.execute(Box::new(cmd)).unwrap();
    doc.save().unwrap();
    assert_eq!(Document::open(&path).unwrap().page_count(), 3);
    assert!(
        doc.model().extra_objects.is_empty(),
        "nach dem Speichern in der Datei"
    );

    doc.undo().unwrap();
    doc.save().unwrap();
    assert_eq!(Document::open(&path).unwrap().page_count(), 2);
    // Wiederherstellen schreibt die Objekte erneut – kein Fehler, keine Duplikate.
    doc.redo().unwrap();
    doc.save().unwrap();
    assert_eq!(Document::open(&path).unwrap().page_count(), 3);
    let _ = std::fs::remove_file(path);
}
