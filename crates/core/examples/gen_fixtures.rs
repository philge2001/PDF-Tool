//! Erzeugt die Test-PDFs unter `tests/fixtures/`.
//!
//! Aufruf (im Workspace-Root):
//!   cargo run -p foliocraft-core --example gen_fixtures
//!   cargo run -p foliocraft-core --example gen_fixtures -- big 500 /tmp/big.pdf
//!
//! Die Dateien sind komplett selbst erzeugt (keine fremden Inhalte).

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};
use std::path::{Path, PathBuf};

/// Seitengrößen in Punkt.
const A4: (f32, f32) = (595.0, 842.0);
const A5: (f32, f32) = (420.0, 595.0);
const LETTER_LANDSCAPE: (f32, f32) = (792.0, 612.0);

/// Kleiner Baukasten um lopdf, damit die Fixture-Definitionen lesbar bleiben.
struct Builder {
    doc: Document,
    pages_id: ObjectId,
    font_id: ObjectId,
    kids: Vec<Object>,
}

impl Builder {
    fn new() -> Self {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        // Standard-14-Schrift: muss nicht eingebettet werden.
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        Self {
            doc,
            pages_id,
            font_id,
            kids: Vec::new(),
        }
    }

    /// Fügt eine Seite mit Textzeilen hinzu. `lines`: (x, y, Schriftgröße, Text).
    /// Gibt die Objekt-ID der Seite zurück, damit z. B. Annotationen angehängt werden können.
    fn page(&mut self, size: (f32, f32), rotate: i64, lines: &[(f32, f32, f32, &str)]) -> ObjectId {
        let mut ops = Vec::new();
        // Dünner grauer Rahmen, damit man Seitengrenzen beim Rendern sieht.
        ops.push(Operation::new(
            "RG",
            vec![0.7.into(), 0.7.into(), 0.7.into()],
        ));
        ops.push(Operation::new(
            "re",
            vec![
                20.into(),
                20.into(),
                (size.0 - 40.0).into(),
                (size.1 - 40.0).into(),
            ],
        ));
        ops.push(Operation::new("S", vec![]));
        for &(x, y, fs, text) in lines {
            ops.push(Operation::new("BT", vec![]));
            ops.push(Operation::new("Tf", vec!["F1".into(), fs.into()]));
            ops.push(Operation::new("Td", vec![x.into(), y.into()]));
            ops.push(Operation::new("Tj", vec![winansi(text)]));
            ops.push(Operation::new("ET", vec![]));
        }
        let content = Content { operations: ops };
        let content_id = self.doc.add_object(Stream::new(
            Dictionary::new(),
            content.encode().expect("Content encodieren"),
        ));
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => vec![0.into(), 0.into(), size.0.into(), size.1.into()],
            "Contents" => content_id,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => self.font_id } },
        };
        if rotate != 0 {
            page.set("Rotate", rotate);
        }
        let id = self.doc.add_object(page);
        self.kids.push(id.into());
        id
    }

    /// Hängt eine Annotation an eine Seite an.
    fn annotate(&mut self, page_id: ObjectId, annot: Dictionary) -> ObjectId {
        let annot_id = self.doc.add_object(annot);
        let page = self
            .doc
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
            .expect("Seite existiert");
        match page.get_mut(b"Annots") {
            Ok(Object::Array(arr)) => arr.push(annot_id.into()),
            _ => page.set("Annots", vec![Object::from(annot_id)]),
        }
        annot_id
    }

    fn finish(mut self, path: &Path) {
        let count = self.kids.len() as i64;
        self.doc.objects.insert(
            self.pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => self.kids,
                "Count" => count,
            }),
        );
        let catalog_id = self.doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => self.pages_id,
        });
        self.doc.trailer.set("Root", catalog_id);
        self.doc.compress();
        self.doc.save(path).expect("PDF speichern");
        println!("geschrieben: {}", path.display());
    }
}

/// Text als PDF-String in WinAnsi (für ä/ö/ü/ß identisch mit Latin-1).
fn winansi(text: &str) -> Object {
    let bytes = text
        .chars()
        .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
        .collect();
    Object::String(bytes, StringFormat::Literal)
}

fn text_two_pages(path: &Path) {
    let mut b = Builder::new();
    b.page(
        A4,
        0,
        &[
            (72.0, 760.0, 24.0, "FolioCraft Testdokument"),
            (
                72.0,
                720.0,
                12.0,
                "Lorem ipsum dolor sit amet, consectetur adipiscing elit.",
            ),
            (
                72.0,
                700.0,
                12.0,
                "Sed do eiusmod tempor incididunt ut labore et dolore magna.",
            ),
            (72.0, 680.0, 12.0, "Grüße aus Köln: Äpfel, Öl und Übermaß."),
            (
                72.0,
                660.0,
                12.0,
                "Suchwort: Zebra. Noch einmal ZEBRA und zebra.",
            ),
        ],
    );
    b.page(
        A4,
        0,
        &[
            (72.0, 760.0, 18.0, "Seite zwei"),
            (
                72.0,
                720.0,
                12.0,
                "Hier steht das Suchwort Zebra ein viertes Mal.",
            ),
            (72.0, 700.0, 12.0, "Lorem ipsum erscheint hier ebenfalls."),
        ],
    );
    b.finish(path);
}

fn mixed_sizes(path: &Path) {
    let mut b = Builder::new();
    b.page(A4, 0, &[(72.0, 760.0, 18.0, "A4 Hochformat")]);
    b.page(
        LETTER_LANDSCAPE,
        0,
        &[(72.0, 530.0, 18.0, "Letter Querformat")],
    );
    b.page(A5, 0, &[(50.0, 520.0, 18.0, "A5 Hochformat")]);
    b.finish(path);
}

fn rotated(path: &Path) {
    let mut b = Builder::new();
    b.page(A4, 0, &[(72.0, 760.0, 18.0, "Ohne Rotation")]);
    b.page(
        A4,
        90,
        &[(
            72.0,
            760.0,
            18.0,
            "Rotate 90: Text oben links im unrotierten Raum",
        )],
    );
    b.page(A4, 180, &[(72.0, 760.0, 18.0, "Rotate 180")]);
    b.finish(path);
}

/// Simuliert eine PDF aus einem anderen Programm, die bereits Annotationen enthält.
fn with_annotations(path: &Path) {
    let mut b = Builder::new();
    let page = b.page(
        A4,
        0,
        &[
            (72.0, 760.0, 18.0, "Dokument mit vorhandenen Annotationen"),
            (72.0, 720.0, 12.0, "Dieser Satz ist gelb hervorgehoben."),
            (
                72.0,
                680.0,
                12.0,
                "Hier liegt ein Link (nicht verwaltet, muss erhalten bleiben).",
            ),
        ],
    );
    // Highlight über "Dieser Satz ist gelb hervorgehoben." (ca. 200pt breit), ohne /AP:
    // Viele Programme schreiben Markup ohne Appearance – unser Import muss das abkönnen.
    b.annotate(
        page,
        dictionary! {
            "Type" => "Annot",
            "Subtype" => "Highlight",
            "Rect" => vec![70.into(), 716.into(), 272.into(), 732.into()],
            "QuadPoints" => vec![
                70.into(), 732.into(), 272.into(), 732.into(),
                70.into(), 716.into(), 272.into(), 716.into(),
            ],
            "C" => vec![1.into(), 1.into(), 0.into()],
            "CA" => 1.0,
            "T" => Object::string_literal("Fremdautor"),
            "Contents" => Object::string_literal("Vorhandener Kommentar"),
            "NM" => Object::string_literal("fixture-highlight-1"),
        },
    );
    b.annotate(
        page,
        dictionary! {
            "Type" => "Annot",
            "Subtype" => "Text",
            "Rect" => vec![400.into(), 700.into(), 420.into(), 720.into()],
            "C" => vec![0.2.into(), 0.6.into(), 1.into()],
            "Name" => "Comment",
            "T" => Object::string_literal("Fremdautor"),
            "Contents" => Object::string_literal("Eine vorhandene Notiz"),
            "NM" => Object::string_literal("fixture-note-1"),
        },
    );
    b.annotate(
        page,
        dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![70.into(), 676.into(), 140.into(), 692.into()],
            "Border" => vec![0.into(), 0.into(), 0.into()],
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.org") },
        },
    );
    b.finish(path);
}

/// Verschachtelter Seitenbaum: MediaBox, Ressourcen und Rotation werden von den
/// Zwischenknoten geerbt (wie in vielen echten PDFs). Testet das Umsortieren von Seiten.
///
///   Wurzel ─┬─ Knoten A (A4, Schrift F1)            ─┬─ Seite 1
///           │                                         └─ Seite 2
///           └─ Knoten B (Letter quer, Rotate 90, F1) ─┬─ Seite 3
///                                                     └─ Seite 4
fn nested_tree(path: &Path) {
    let mut doc = Document::with_version("1.7");
    let font = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let root = doc.new_object_id();
    let node_a = doc.new_object_id();
    let node_b = doc.new_object_id();
    let page = |doc: &mut Document, parent: ObjectId, n: usize| -> ObjectId {
        let text = format!("Seite {n} im verschachtelten Baum");
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 24.into()]),
                Operation::new("Td", vec![72.into(), 500.into()]),
                Operation::new("Tj", vec![winansi(&text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let c = doc.add_object(Stream::new(
            Dictionary::new(),
            content.encode().expect("Content"),
        ));
        doc.add_object(dictionary! { "Type" => "Page", "Parent" => parent, "Contents" => c })
    };
    let p1 = page(&mut doc, node_a, 1);
    let p2 = page(&mut doc, node_a, 2);
    let p3 = page(&mut doc, node_b, 3);
    let p4 = page(&mut doc, node_b, 4);
    let resources = || dictionary! { "Font" => dictionary! { "F1" => font } };
    doc.objects.insert(
        node_a,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Parent" => root, "Kids" => vec![p1.into(), p2.into()], "Count" => 2,
            "MediaBox" => vec![0.into(), 0.into(), A4.0.into(), A4.1.into()],
            "Resources" => resources(),
        }),
    );
    doc.objects.insert(node_b, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Parent" => root, "Kids" => vec![p3.into(), p4.into()], "Count" => 2,
        "MediaBox" => vec![0.into(), 0.into(), LETTER_LANDSCAPE.0.into(), LETTER_LANDSCAPE.1.into()],
        "Resources" => resources(),
        "Rotate" => 90,
    }));
    doc.objects.insert(
        root,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![node_a.into(), node_b.into()], "Count" => 4,
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
    doc.trailer.set("Root", catalog);
    doc.compress();
    doc.save(path).expect("PDF speichern");
    println!("geschrieben: {}", path.display());
}

/// Viele Seiten für Performance-Tests (wird nicht eingecheckt).
fn big(path: &Path, pages: usize) {
    let mut b = Builder::new();
    let line = "Performance-Test: Lorem ipsum dolor sit amet, consectetur adipiscing elit.";
    for i in 1..=pages {
        let title = format!("Seite {i} von {pages}");
        let mut lines: Vec<(f32, f32, f32, &str)> = vec![(72.0, 760.0, 20.0, title.as_str())];
        for row in 0..40 {
            lines.push((72.0, 720.0 - row as f32 * 16.0, 10.0, line));
        }
        b.page(A4, 0, &lines);
    }
    b.finish(path);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("big") {
        let pages = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(500);
        let out = args
            .get(2)
            .map(PathBuf::from)
            .unwrap_or_else(|| "big.pdf".into());
        big(&out, pages);
        return;
    }
    // CARGO_MANIFEST_DIR zeigt zur Compile-Zeit auf crates/core – von dort zwei Ebenen hoch.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    std::fs::create_dir_all(&dir).expect("Fixture-Ordner anlegen");
    let dir = dir.canonicalize().expect("Pfad auflösen");
    text_two_pages(&dir.join("text_two_pages.pdf"));
    mixed_sizes(&dir.join("mixed_sizes.pdf"));
    rotated(&dir.join("rotated.pdf"));
    with_annotations(&dir.join("with_annotations.pdf"));
    nested_tree(&dir.join("nested_tree.pdf"));
}
