//! Formulare: lesen -> ausfüllen -> speichern -> neu laden -> vergleichen.

use foliocraft_core::form::appearance::{self, WidgetAppearance};
use foliocraft_core::{Document, FieldKind, FieldValue, UpdateField};
use lopdf::Object;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn open_form() -> Document {
    Document::open(fixture("form.pdf")).unwrap()
}

fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

fn set(doc: &mut Document, name: &str, value: FieldValue) {
    let field = doc.forms().by_name(name).unwrap().clone();
    doc.execute(Box::new(UpdateField::value(&field, value)))
        .unwrap();
}

fn value(doc: &Document, name: &str) -> FieldValue {
    doc.forms().by_name(name).unwrap().value.clone()
}

fn text(s: &str) -> FieldValue {
    FieldValue::Text(s.into())
}

#[test]
fn reads_all_fields_with_hierarchy_values_and_flags() {
    let doc = open_form();
    let names: Vec<&str> = doc.forms().iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "person.name",
            "person.email",
            "bemerkung",
            "agb",
            "versand",
            "land",
            "interessen",
            "kundennr",
            "pflicht",
            "absenden",
            "unterschrift"
        ]
    );
    let f = |n: &str| doc.forms().by_name(n).unwrap();
    assert_eq!(f("person.name").value, text("Max Mustermann"));
    assert_eq!(f("person.name").font_size, 11.0);
    assert_eq!(f("person.email").font_size, 0.0); // automatisch
    assert!(matches!(
        f("bemerkung").kind,
        FieldKind::Text {
            multiline: true,
            ..
        }
    ));
    assert_eq!(f("agb").kind, FieldKind::CheckBox);
    assert_eq!(f("agb").value, FieldValue::Toggle(None));
    assert_eq!(f("agb").widgets[0].on_state.as_deref(), Some("Ja"));
    let versand = f("versand");
    assert_eq!(versand.kind, FieldKind::Radio);
    assert_eq!(versand.widgets.len(), 3);
    assert_eq!(versand.value, FieldValue::Toggle(Some("post".into())));
    assert!(versand.widget_is_on(&versand.widgets[0]));
    assert!(!versand.widget_is_on(&versand.widgets[1]));
    assert_eq!(f("land").kind, FieldKind::Combo { editable: false });
    assert_eq!(f("land").options[1].display, "Österreich");
    assert_eq!(f("interessen").kind, FieldKind::List { multi: true });
    assert_eq!(
        f("interessen").value,
        FieldValue::Choice(vec!["rust".into()])
    );
    assert_eq!(f("interessen").display_of("pdf"), "PDF");
    assert!(f("kundennr").read_only);
    assert!(f("pflicht").required);
    assert_eq!(f("absenden").kind, FieldKind::PushButton);
    assert_eq!(f("unterschrift").kind, FieldKind::Signature);
    // Alle Widgets liegen auf Seite 1.
    let page = doc.pages()[0].object_id;
    assert!(
        doc.forms()
            .iter()
            .all(|f| f.widgets.iter().all(|w| w.page == page))
    );
}

#[test]
fn renderer_hides_widgets_of_fillable_fields_only() {
    let doc = open_form();
    // 13 Widgets, davon 11 ausfüllbar (Schaltfläche und Signatur zeichnet PDFium).
    assert_eq!(doc.managed_annot_indices(0).len(), 11);
    let snap = doc.render_snapshot().unwrap();
    assert_eq!(snap.hidden.get(&0).map(Vec::len), Some(11));
}

#[test]
fn fill_save_reload_roundtrip() {
    let mut doc = open_form();
    set(&mut doc, "person.name", text("Erika Musterfrau"));
    set(&mut doc, "person.email", text("erika@example.org"));
    set(
        &mut doc,
        "bemerkung",
        text("Zeile eins\nZeile zwei mit Ümlaut"),
    );
    set(&mut doc, "agb", FieldValue::Toggle(Some("Ja".into())));
    set(
        &mut doc,
        "versand",
        FieldValue::Toggle(Some("email".into())),
    );
    set(&mut doc, "land", FieldValue::Choice(vec!["Schweiz".into()]));
    set(
        &mut doc,
        "interessen",
        FieldValue::Choice(vec!["pdf".into(), "egui".into()]),
    );
    assert!(doc.is_dirty());

    let back = reload(&doc);
    for name in [
        "person.name",
        "person.email",
        "bemerkung",
        "agb",
        "versand",
        "land",
        "interessen",
    ] {
        assert_eq!(value(&back, name), value(&doc, name), "{name}");
    }
    // Unverändert gebliebene Felder auch.
    assert_eq!(value(&back, "kundennr"), text("K-12345"));
    // Hierarchie bleibt erhalten (Wert steht im Kind, nicht im Elternfeld).
    assert_eq!(back.forms().len(), 11);

    // /AS der Widgets ist umgeschaltet, vorhandene Erscheinungsbilder bleiben.
    let pdf = back.pdf();
    let versand = back.forms().by_name("versand").unwrap();
    let states: Vec<Vec<u8>> = versand
        .widgets
        .iter()
        .map(|w| {
            let d = pdf.get_dictionary(w.object_id.unwrap()).unwrap();
            d.get(b"AS").unwrap().as_name().unwrap().to_vec()
        })
        .collect();
    assert_eq!(
        states,
        vec![b"Off".to_vec(), b"Off".to_vec(), b"email".to_vec()]
    );

    // Listenfeld: /I mit den Indizes.
    let list = back.forms().by_name("interessen").unwrap();
    let d = pdf.get_dictionary(list.source.unwrap()).unwrap();
    let idx: Vec<i64> = d
        .get(b"I")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_i64().unwrap())
        .collect();
    assert_eq!(idx, vec![0, 2]);
}

#[test]
fn text_appearance_is_written() {
    let mut doc = open_form();
    set(&mut doc, "person.email", text("erika@example.org"));
    let back = reload(&doc);
    let pdf = back.pdf();
    let w = back.forms().by_name("person.email").unwrap().widgets[0].clone();
    let d = pdf.get_dictionary(w.object_id.unwrap()).unwrap();
    let ap = d.get(b"AP").unwrap().as_dict().unwrap();
    let id = ap.get(b"N").unwrap().as_reference().unwrap();
    let Object::Stream(s) = pdf.get_object(id).unwrap() else {
        panic!("kein Stream")
    };
    let content = String::from_utf8_lossy(&s.decompressed_content().unwrap_or(s.content.clone()))
        .into_owned();
    assert!(content.contains("(erika@example.org) Tj"), "{content}");
    // Breite/Höhe der BBox = Größe von /Rect.
    let bbox: Vec<f32> = s
        .dict
        .get(b"BBox")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_float().unwrap())
        .collect();
    assert_eq!(bbox, vec![0.0, 0.0, w.rect.width(), w.rect.height()]);
}

#[test]
fn typing_merges_into_one_undo_step() {
    let mut doc = open_form();
    for s in ["E", "Er", "Eri"] {
        let field = doc.forms().by_name("person.email").unwrap().clone();
        doc.execute_merging(Box::new(UpdateField::value(&field, text(s)).mergeable()))
            .unwrap();
    }
    assert_eq!(
        doc.history().undo_label().as_deref(),
        Some("„email“ ausfüllen")
    );
    doc.undo().unwrap();
    assert_eq!(value(&doc, "person.email"), text(""));
    assert!(!doc.is_dirty());
    doc.redo().unwrap();
    assert_eq!(value(&doc, "person.email"), text("Eri"));
}

#[test]
fn save_twice_to_file() {
    let path = std::env::temp_dir().join(format!(
        "foliocraft-form-{}.pdf",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::copy(fixture("form.pdf"), &path).unwrap();
    let mut doc = Document::open(&path).unwrap();
    set(&mut doc, "pflicht", text("erste Fassung"));
    doc.save().unwrap();
    assert!(!doc.is_dirty());
    set(&mut doc, "pflicht", text("zweite Fassung"));
    doc.save().unwrap();
    let back = Document::open(&path).unwrap();
    assert_eq!(value(&back, "pflicht"), text("zweite Fassung"));
    // Rückgängig nach dem Speichern und erneut speichern.
    doc.undo().unwrap();
    doc.save().unwrap();
    assert_eq!(
        value(&Document::open(&path).unwrap(), "pflicht"),
        text("erste Fassung")
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn widgets_follow_their_page() {
    let mut doc = open_form();
    let page = doc.pages()[0].object_id;
    let blank = doc.prepare_blank_page(0, (200.0, 200.0));
    doc.execute(Box::new(blank)).unwrap();
    assert_eq!(doc.page_index_of(page), Some(1));
    assert_eq!(doc.forms().widgets_on(page).count(), 13);
    set(&mut doc, "person.name", text("nach Einfügen"));
    let back = reload(&doc);
    assert_eq!(back.page_count(), 2);
    assert_eq!(value(&back, "person.name"), text("nach Einfügen"));
    assert_eq!(back.managed_annot_indices(1).len(), 11);
}

#[test]
fn generated_states_for_checkbox_without_appearance() {
    let doc = open_form();
    let mut agb = doc.forms().by_name("agb").unwrap().clone();
    agb.widgets[0].on_state = Some("Yes".into());
    agb.value = FieldValue::Toggle(Some("Yes".into()));
    match appearance::build(&agb, &agb.widgets[0]) {
        Some(WidgetAppearance::States { on_state, on, off }) => {
            assert_eq!(on_state, "Yes");
            assert!(on.content.len() > off.content.len());
        }
        _ => panic!("erwartet zwei Zustände"),
    }
}
