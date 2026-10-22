//! Text bearbeiten/hinzufügen: bearbeiten → speichern → neu laden → vergleichen.

use foliocraft_core::content::{self, Item};
use foliocraft_core::embedded_font::SubsetFont;
use foliocraft_core::{Color, Document, Point, TextBlock};
use lopdf::{Object, Stream, dictionary};

fn fixture(name: &str) -> Document {
    let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    Document::open(path).unwrap()
}

fn reload(doc: &Document) -> Document {
    Document::from_bytes(doc.to_bytes().unwrap()).unwrap()
}

fn block<'a>(blocks: &'a [TextBlock], starts: &str) -> &'a TextBlock {
    blocks
        .iter()
        .find(|b| b.text.starts_with(starts))
        .unwrap_or_else(|| panic!("kein Block mit {starts:?}: {blocks:#?}"))
}

/// Ressourcennamen der Schriften, mit denen auf Seite `page` Text gezeigt wird.
fn fonts_used(doc: &Document, page: usize) -> Vec<String> {
    let id = doc.pages()[page].object_id;
    let it = content::interpret_page(doc.pdf(), id).unwrap();
    let mut v: Vec<String> = it
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text(t) if !t.glyphs.is_empty() => {
                Some(String::from_utf8_lossy(&t.font).into_owned())
            }
            _ => None,
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn blocks_of_fixture() {
    let doc = fixture("text_two_pages.pdf");
    let blocks = doc.text_blocks(0).unwrap();
    let b = block(&blocks, "Grüße");
    assert_eq!(b.text, "Grüße aus Köln: Äpfel, Öl und Übermaß.");
    assert!((b.size_on_page - 12.0).abs() < 0.5, "{}", b.size_on_page);
    assert_eq!(b.stamp, 0);
}

#[test]
fn edit_keeps_positions_and_saves() {
    let mut doc = fixture("text_two_pages.pdf");
    let blocks = doc.text_blocks(0).unwrap();
    let target = block(&blocks, "Lorem").clone();
    let others: Vec<(String, Point)> = blocks
        .iter()
        .filter(|b| b.ops != target.ops)
        .map(|b| (b.text.clone(), b.origin))
        .collect();

    doc.edit_text_block(0, &target, "Hallo Welt – Привет Ω")
        .unwrap();
    assert!(doc.is_dirty());
    let edited = doc.text_blocks(0).unwrap();
    assert_eq!(edited[0].stamp, doc.content_stamp(doc.pages()[0].object_id));

    // Nach Speichern und neu Laden: neuer Text da, alles andere unverändert an seinem Platz.
    let saved = reload(&doc);
    let after = saved.text_blocks(0).unwrap();
    let all: String = after
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("|");
    assert!(all.contains("Hallo Welt – "), "{all}");
    assert!(all.contains("Привет Ω"), "{all}");
    assert!(!all.contains("Lorem"), "{all}");
    for (text, origin) in &others {
        let b = block(&after, text);
        assert!(
            b.origin.distance(*origin) < 0.01,
            "{text} verschoben: {:?} -> {:?}",
            origin,
            b.origin
        );
    }
    // Latein bleibt in Helvetica, Kyrillisch kommt aus der eingebetteten Ersatzschrift.
    let fonts = fonts_used(&saved, 0);
    assert!(fonts.iter().any(|f| f.starts_with("FcE")), "{fonts:?}");
    // Ersatzschrift-Läufe gehören zum Block: eine Zeile, ein Block, Schrift des Blocks = F1.
    let new = block(&after, "Hallo");
    assert_eq!(new.text, "Hallo Welt – Привет Ω");
    assert_eq!(String::from_utf8_lossy(&new.font), "F1");

    // Erneut bearbeiten (beginnt jetzt mit Ersatzschrift-Zeichen) bleibt korrekt.
    let mut saved = saved;
    let b = block(&saved.text_blocks(0).unwrap(), "Hallo").clone();
    saved.edit_text_block(0, &b, "Ωmega und Alpha").unwrap();
    let again = reload(&saved);
    assert_eq!(
        block(&again.text_blocks(0).unwrap(), "Ωmega").text,
        "Ωmega und Alpha"
    );
    let b = block(&again.text_blocks(0).unwrap(), "Ωmega").clone();
    let mut again = again;
    again.edit_text_block(0, &b, "Zurück").unwrap();
    assert_eq!(
        block(&reload(&again).text_blocks(0).unwrap(), "Zurück").text,
        "Zurück"
    );

    // Die Originalbytes stehen unverändert am Anfang (inkrementelles Update).
    let original = std::fs::read(format!(
        "{}/../../tests/fixtures/text_two_pages.pdf",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert!(saved.bytes().starts_with(&original));
}

#[test]
fn undo_redo_restores_content_also_after_save() {
    let mut doc = fixture("text_two_pages.pdf");
    let blocks = doc.text_blocks(0).unwrap();
    let target = block(&blocks, "Suchwort").clone();
    doc.edit_text_block(0, &target, "Neu").unwrap();

    let dir = std::env::temp_dir().join(format!("fc_textedit_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.pdf");
    doc.save_as(&path).unwrap();
    assert!(!doc.is_dirty());
    assert!(block(&doc.text_blocks(0).unwrap(), "Neu").text == "Neu");

    // Undo nach dem Speichern: alter Text ist wieder da und wird auch so gespeichert.
    assert!(doc.undo().unwrap());
    let undone = reload(&doc);
    let b = block(&undone.text_blocks(0).unwrap(), "Suchwort").clone();
    assert_eq!(b.text, "Suchwort: Zebra. Noch einmal ZEBRA und zebra.");
    assert!(doc.redo().unwrap());
    assert!(block(&reload(&doc).text_blocks(0).unwrap(), "Neu").text == "Neu");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn stale_block_is_rejected() {
    let mut doc = fixture("text_two_pages.pdf");
    let blocks = doc.text_blocks(0).unwrap();
    doc.edit_text_block(0, block(&blocks, "Lorem"), "A")
        .unwrap();
    assert!(doc.edit_text_block(0, block(&blocks, "Sed"), "B").is_err());
}

#[test]
fn add_text_on_rotated_page() {
    let mut doc = fixture("rotated.pdf");
    let rotation = doc.pages()[0].rotation;
    doc.add_text(
        0,
        Point::new(100.0, 200.0),
        "Neu hinzugefügt ✓\nZweite Zeile",
        14.0,
        Color::BLUE,
    )
    .ok();
    // ✓ fehlt in Noto Sans -> klare Fehlermeldung, nichts geändert.
    assert!(!doc.is_dirty());
    doc.add_text(
        0,
        Point::new(100.0, 200.0),
        "Neu hinzugefügt\nZweite Zeile",
        14.0,
        Color::BLUE,
    )
    .unwrap();

    let saved = reload(&doc);
    let blocks = saved.text_blocks(0).unwrap();
    let b = block(&blocks, "Neu hinzugefügt");
    assert!(b.origin.distance(Point::new(100.0, 200.0)) < 0.01);
    assert!((b.size_on_page - 14.0).abs() < 0.01);
    // Grundlinie um die Seitendrehung gedreht -> aufrecht in der Anzeige.
    let angle = b
        .direction
        .y
        .atan2(b.direction.x)
        .to_degrees()
        .rem_euclid(360.0);
    assert!(
        (angle - f32::from(rotation)).abs() < 0.1,
        "{angle} vs {rotation}"
    );
    block(&blocks, "Zweite Zeile");

    // Zweimal hinzufügen: Die angehängten Inhalte müssen sauber getrennt sein.
    let mut twice = fixture("rotated.pdf");
    twice
        .add_text(0, Point::new(50.0, 50.0), "Eins", 10.0, Color::BLACK)
        .unwrap();
    twice
        .add_text(0, Point::new(50.0, 80.0), "Zwei", 10.0, Color::BLACK)
        .unwrap();
    let content = twice.page_content(0).unwrap().content;
    assert!(
        !content.windows(2).any(|w| w == b"Qq"),
        "Operatoren zusammengeklebt"
    );
    let texts: Vec<String> = reload(&twice)
        .text_blocks(0)
        .unwrap()
        .into_iter()
        .map(|b| b.text)
        .collect();
    assert!(
        texts.contains(&"Eins".to_string()) && texts.contains(&"Zwei".to_string()),
        "{texts:?}"
    );

    // Neuer Text lässt sich anschließend bearbeiten (dieselbe Ersatzschrift).
    let mut saved = saved;
    let b = block(&saved.text_blocks(0).unwrap(), "Zweite").clone();
    saved.edit_text_block(0, &b, "Ωmega").unwrap();
    let again = reload(&saved);
    block(&again.text_blocks(0).unwrap(), "Ωmega");
}

/// Eine Seite, deren Text in einer eingebetteten Teilmenge (/F1) steht.
fn subset_document() -> Document {
    let mut pdf = lopdf::Document::with_version("1.7");
    let mut sub = SubsetFont::new();
    let text = sub.encode_object("Hallo Welt");
    let font = sub.write(&mut |o| pdf.add_object(o)).unwrap();
    // Wie eine fremde Teilmengen-Schrift (nicht unsere eigene Noto-Teilmenge).
    pdf.get_object_mut(font)
        .and_then(Object::as_dict_mut)
        .unwrap()
        .set("BaseFont", "ABCDEF+TestSans");
    let content = lopdf::content::Content {
        operations: vec![
            lopdf::content::Operation::new("BT", vec![]),
            lopdf::content::Operation::new("Tf", vec!["F1".into(), 20.into()]),
            lopdf::content::Operation::new("Td", vec![72.into(), 700.into()]),
            lopdf::content::Operation::new("Tj", vec![text]),
            lopdf::content::Operation::new("ET", vec![]),
        ],
    };
    let content_id = pdf.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let pages_id = pdf.new_object_id();
    let page = pdf.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        "Contents" => content_id,
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
    });
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
        ),
    );
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    Document::from_bytes(bytes).unwrap()
}

#[test]
fn subset_font_uses_only_present_glyphs() {
    let mut doc = subset_document();
    let b = block(&doc.text_blocks(0).unwrap(), "Hallo").clone();
    assert_eq!(b.text, "Hallo Welt");

    // Nur vorhandene Zeichen: bleibt komplett in der Originalschrift.
    doc.edit_text_block(0, &b, "Wolle tel").unwrap();
    let saved = reload(&doc);
    assert_eq!(fonts_used(&saved, 0), vec!["F1".to_string()]);
    assert_eq!(
        block(&saved.text_blocks(0).unwrap(), "Wolle").text,
        "Wolle tel"
    );

    // "x" und "!" fehlen der Teilmenge -> Ersatzschrift für genau diese Zeichen.
    let mut doc = subset_document();
    let b = block(&doc.text_blocks(0).unwrap(), "Hallo").clone();
    doc.edit_text_block(0, &b, "Hallo Box!").unwrap();
    let saved = reload(&doc);
    assert_eq!(
        fonts_used(&saved, 0),
        vec!["F1".to_string(), "FcE".to_string()]
    );
    assert_eq!(
        block(&saved.text_blocks(0).unwrap(), "Hallo").text,
        "Hallo Box!"
    );
}
