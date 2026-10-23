//! Formulare: lesen -> ausfüllen -> speichern -> neu laden -> vergleichen.

use lopdf::Object;
use pdftool_core::form::appearance::{self, WidgetAppearance};
use pdftool_core::{Document, FieldKind, FieldValue, UpdateField};
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
        "pdftool-form-{}.pdf",
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

// ---------- Formular-Editor (3.3) ----------

use pdftool_core::form::ChoiceOption;
use pdftool_core::{AddField, FormField, Rect, RemoveField};

fn add(doc: &mut Document, mut f: FormField) -> pdftool_core::FieldId {
    f.id = doc.allocate_field_id();
    let id = f.id;
    doc.execute(Box::new(AddField::new(f))).unwrap();
    id
}

fn update(doc: &mut Document, name: &str, change: impl FnOnce(&mut FormField)) {
    let before = doc.forms().by_name(name).unwrap().clone();
    let mut after = before.clone();
    change(&mut after);
    doc.execute(Box::new(UpdateField::new(before, after)))
        .unwrap();
}

#[test]
fn create_fields_in_document_without_form() {
    let mut doc = Document::open(fixture("text_two_pages.pdf")).unwrap();
    let page = doc.pages()[1].object_id;
    let text_kind = FieldKind::Text {
        multiline: false,
        password: false,
        max_len: Some(20),
    };
    let mut name = FormField::new(
        text_kind,
        "vorname",
        page,
        Rect::new(100.0, 600.0, 300.0, 620.0),
    );
    name.value = text("Anna");
    name.required = true;
    add(&mut doc, name);
    let mut combo = FormField::new(
        FieldKind::Combo { editable: true },
        "farbe",
        page,
        Rect::new(100.0, 560.0, 250.0, 580.0),
    );
    combo.options = vec![
        ChoiceOption {
            export: "r".into(),
            display: "Rot".into(),
        },
        ChoiceOption {
            export: "Grün".into(),
            display: "Grün".into(),
        },
    ];
    combo.value = FieldValue::Choice(vec!["r".into()]);
    add(&mut doc, combo);
    let mut radio = FormField::new(
        FieldKind::Radio,
        "groesse",
        page,
        Rect::new(100.0, 520.0, 112.0, 532.0),
    );
    let mut second = radio.widgets[0].clone();
    second.rect = second.rect.translate(40.0, 0.0);
    second.on_state = Some("Option2".into());
    radio.widgets.push(second);
    radio.value = FieldValue::Toggle(Some("Option2".into()));
    add(&mut doc, radio);

    // Vor dem Speichern: Momentaufnahme blendet die neuen Widgets schon aus (kein Doppelbild).
    let snap = doc.render_snapshot().unwrap();
    assert_eq!(snap.hidden.get(&1).map(Vec::len), Some(4));

    let back = reload(&doc);
    assert_eq!(back.forms().len(), 3);
    let f = back.forms().by_name("vorname").unwrap();
    assert_eq!(f.value, text("Anna"));
    assert!(f.required);
    assert_eq!(
        f.kind,
        FieldKind::Text {
            multiline: false,
            password: false,
            max_len: Some(20)
        }
    );
    assert_eq!(f.widgets[0].page, back.pages()[1].object_id);
    assert_eq!(f.widgets[0].rect, Rect::new(100.0, 600.0, 300.0, 620.0));
    let c = back.forms().by_name("farbe").unwrap();
    assert_eq!(c.options.len(), 2);
    assert_eq!(c.display_of("r"), "Rot");
    assert_eq!(c.kind, FieldKind::Combo { editable: true });
    let r = back.forms().by_name("groesse").unwrap();
    assert_eq!(r.kind, FieldKind::Radio);
    assert_eq!(r.widgets.len(), 2);
    assert!(r.widget_is_on(&r.widgets[1]));
    assert_eq!(back.managed_annot_indices(1).len(), 4);
}

#[test]
fn change_properties_of_existing_fields() {
    let mut doc = open_form();
    update(&mut doc, "pflicht", |f| {
        f.name = "pflicht2".into();
        f.required = false;
        f.read_only = true;
        f.tooltip = "Hinweis".into();
        f.widgets[0].rect = f.widgets[0].rect.translate(10.0, -5.0);
    });
    update(&mut doc, "person.email", |f| f.name = "person.mail".into());
    update(&mut doc, "land", |f| {
        f.options.push(ChoiceOption {
            export: "Italien".into(),
            display: "Italien".into(),
        })
    });
    let back = reload(&doc);
    let f = back.forms().by_name("pflicht2").unwrap();
    assert!(!f.required && f.read_only);
    assert_eq!(f.tooltip, "Hinweis");
    assert_eq!(f.widgets[0].rect, Rect::new(190.0, 380.0, 340.0, 398.0));
    // Verschachteltes Feld: nur der Teilname ändert sich, der Elternteil bleibt.
    assert!(back.forms().by_name("person.mail").is_some());
    assert_eq!(back.forms().by_name("land").unwrap().options.len(), 4);
}

#[test]
fn delete_nested_and_top_level_fields() {
    let mut doc = open_form();
    let annots_before = doc.managed_annot_indices(0).len();
    for name in ["person.email", "bemerkung"] {
        let id = doc.forms().by_name(name).unwrap().id;
        doc.execute(Box::new(RemoveField::new(id))).unwrap();
    }
    let back = reload(&doc);
    assert!(back.forms().by_name("person.email").is_none());
    assert!(back.forms().by_name("bemerkung").is_none());
    assert_eq!(value(&back, "person.name"), text("Max Mustermann"));
    assert_eq!(back.forms().len(), 9);
    assert_eq!(back.managed_annot_indices(0).len(), annots_before - 2);
}

#[test]
fn add_option_to_existing_radio_group() {
    let mut doc = open_form();
    update(&mut doc, "versand", |f| {
        let mut w = f.widgets[2].clone();
        w.object_id = None;
        w.rect = w.rect.translate(60.0, 0.0);
        w.on_state = Some("kurier".into());
        f.widgets.push(w);
        f.value = FieldValue::Toggle(Some("kurier".into()));
    });
    let back = reload(&doc);
    let v = back.forms().by_name("versand").unwrap();
    assert_eq!(v.widgets.len(), 4);
    assert_eq!(v.value, FieldValue::Toggle(Some("kurier".into())));
    assert!(v.widget_is_on(&v.widgets[3]));
    assert_eq!(back.managed_annot_indices(0).len(), 12);
}

#[test]
fn created_field_survives_save_undo_redo() {
    let path = std::env::temp_dir().join(format!(
        "pdftool-formedit-{}.pdf",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::copy(fixture("text_two_pages.pdf"), &path).unwrap();
    let mut doc = Document::open(&path).unwrap();
    let page = doc.pages()[0].object_id;
    add(
        &mut doc,
        FormField::new(
            FieldKind::CheckBox,
            "ok",
            page,
            Rect::new(50.0, 50.0, 64.0, 64.0),
        ),
    );
    doc.save().unwrap();
    assert!(doc.forms().by_name("ok").unwrap().source.is_some());
    // Nach dem Speichern ausfüllen: schreibt dasselbe Objekt neu.
    set(&mut doc, "ok", FieldValue::Toggle(Some("Ja".into())));
    doc.save().unwrap();
    let back = Document::open(&path).unwrap();
    assert_eq!(value(&back, "ok"), FieldValue::Toggle(Some("Ja".into())));
    assert_eq!(back.forms().len(), 1);

    doc.undo().unwrap(); // Wert
    doc.undo().unwrap(); // Feld
    doc.save().unwrap();
    assert_eq!(Document::open(&path).unwrap().forms().len(), 0);
    doc.redo().unwrap();
    doc.save().unwrap();
    assert_eq!(Document::open(&path).unwrap().forms().len(), 1);
    let _ = std::fs::remove_file(path);
}

// ---------- Formulare beim Einfügen/Extrahieren (3.4) ----------

#[test]
fn inserting_form_page_brings_fillable_fields() {
    let mut doc = Document::open(fixture("text_two_pages.pdf")).unwrap();
    let mut source = open_form();
    set(&mut source, "person.email", text("quelle@example.org"));
    let cmd = doc.prepare_insert(&source, &[0], 1).unwrap();
    doc.execute(Box::new(cmd)).unwrap();
    // Schaltfläche und Signaturfeld bleiben fremd (PDFium zeichnet sie).
    assert_eq!(doc.forms().len(), 9);
    assert_eq!(value(&doc, "person_email"), text("quelle@example.org"));

    // Ein zweites Mal: gleiche Namen wären dasselbe Feld -> umbenannt.
    let cmd = doc.prepare_insert(&source, &[0], 2).unwrap();
    doc.execute(Box::new(cmd)).unwrap();
    assert_eq!(doc.forms().len(), 18);
    assert!(doc.forms().by_name("person_email_2").is_some());

    let back = reload(&doc);
    assert_eq!(back.page_count(), 4);
    assert_eq!(back.forms().len(), 18);
    assert_eq!(value(&back, "person_email"), text("quelle@example.org"));
    assert_eq!(
        value(&back, "versand_2"),
        FieldValue::Toggle(Some("post".into()))
    );
    assert_eq!(back.managed_annot_indices(1).len(), 11);
    assert_eq!(back.managed_annot_indices(2).len(), 11);

    doc.undo().unwrap();
    doc.undo().unwrap();
    assert!(doc.forms().is_empty());
}

#[test]
fn extracted_pages_keep_their_form() {
    let mut doc = open_form();
    set(&mut doc, "pflicht", text("extrahiert"));
    let bytes = pdftool_core::extract_pages(&doc.snapshot_pdf().unwrap(), &[0]).unwrap();
    let out = Document::from_bytes(bytes).unwrap();
    assert_eq!(out.forms().len(), 11);
    assert_eq!(value(&out, "pflicht"), text("extrahiert"));
    assert_eq!(value(&out, "person.name"), text("Max Mustermann"));
    assert_eq!(out.forms().by_name("versand").unwrap().widgets.len(), 3);
}

#[test]
fn extracting_page_without_fields_drops_form() {
    // Seite 1 der zusammengesetzten Datei hat keine Felder -> keine /AcroForm im Ergebnis.
    let mut doc = Document::open(fixture("text_two_pages.pdf")).unwrap();
    let cmd = doc.prepare_insert(&open_form(), &[0], 1).unwrap();
    doc.execute(Box::new(cmd)).unwrap();
    let snap = doc.snapshot_pdf().unwrap();
    let only_text =
        Document::from_bytes(pdftool_core::extract_pages(&snap, &[0]).unwrap()).unwrap();
    assert!(only_text.forms().is_empty());
    let with_form =
        Document::from_bytes(pdftool_core::extract_pages(&snap, &[1]).unwrap()).unwrap();
    assert_eq!(with_form.forms().len(), 9);
}
