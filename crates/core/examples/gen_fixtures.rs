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
    /// Zusätzliche Einträge für den Katalog (z. B. /AcroForm).
    catalog_extra: Dictionary,
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
            catalog_extra: Dictionary::new(),
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

    /// Seite mit frei geschriebenem Inhalt (rohe Content-Stream-Bytes) und Ressourcen.
    fn raw_page(&mut self, size: (f32, f32), content: Vec<u8>, resources: Dictionary) -> ObjectId {
        let content_id = self.doc.add_object(Stream::new(Dictionary::new(), content));
        let id = self.doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => vec![0.into(), 0.into(), size.0.into(), size.1.into()],
            "Contents" => content_id,
            "Resources" => resources,
        });
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
        let mut catalog = dictionary! {
            "Type" => "Catalog",
            "Pages" => self.pages_id,
        };
        catalog.extend(&self.catalog_extra);
        let catalog_id = self.doc.add_object(catalog);
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

/// Form-XObject (für Appearance Streams von Formularfeldern).
fn form_xobject(doc: &mut Document, w: f32, h: f32, ops: &str, font: Option<ObjectId>) -> ObjectId {
    let mut res = Dictionary::new();
    if let Some(f) = font {
        res.set("Font", dictionary! { "Helv" => f });
    }
    doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), w.into(), h.into()],
            "Resources" => res,
        },
        ops.as_bytes().to_vec(),
    ))
}

/// Formular mit allen Feldtypen (wie von einem anderen Programm erzeugt, inkl. Appearances).
///
/// Felder (voller Name: Typ, Wert):
///   person.name (Text, "Max Mustermann") · person.email (Text, leer)
///   bemerkung (Text, mehrzeilig) · agb (Kontrollkästchen, aus; An-Zustand "Ja")
///   versand (Optionsfelder post/abholung/email, "post") · land (Kombifeld, "Deutschland")
///   interessen (Liste, Mehrfachauswahl, ["rust"]) · kundennr (Text, schreibgeschützt, "K-12345")
///   pflicht (Text, Pflichtfeld) · absenden (Schaltfläche) · unterschrift (Signaturfeld)
fn form(path: &Path) {
    let mut b = Builder::new();
    let labels = [
        (72.0, 762.0, 16.0, "Anmeldeformular"),
        (72.0, 720.0, 11.0, "Name:"),
        (72.0, 690.0, 11.0, "E-Mail:"),
        (72.0, 660.0, 11.0, "Bemerkung:"),
        (72.0, 590.0, 11.0, "AGB akzeptiert:"),
        (72.0, 560.0, 11.0, "Versand:"),
        (198.0, 560.0, 11.0, "Post"),
        (258.0, 560.0, 11.0, "Abholung"),
        (338.0, 560.0, 11.0, "E-Mail"),
        (72.0, 530.0, 11.0, "Land:"),
        (72.0, 500.0, 11.0, "Interessen:"),
        (72.0, 420.0, 11.0, "Kundennummer:"),
        (72.0, 390.0, 11.0, "Pflichtfeld:"),
        (72.0, 300.0, 11.0, "Unterschrift:"),
    ];
    let page = b.page(A4, 0, &labels);
    let font = b.font_id;
    let doc = &mut b.doc;
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| -> Object {
        vec![x0.into(), y0.into(), x1.into(), y1.into()].into()
    };
    let text = |s: &str| Object::string_literal(s.as_bytes().to_vec());
    let mut fields: Vec<Object> = Vec::new();
    let mut widgets: Vec<ObjectId> = Vec::new();

    // person (nicht-terminal) mit zwei Text-Kindern; Feld und Widget im selben Objekt.
    let person = doc.new_object_id();
    let name_ap = form_xobject(
        doc,
        250.0,
        18.0,
        "/Tx BMC q BT /Helv 11 Tf 0 g 2 5 Td (Max Mustermann) Tj ET Q EMC",
        Some(font),
    );
    let name = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "Parent" => person, "T" => text("name"),
        "FT" => "Tx", "V" => text("Max Mustermann"), "DA" => text("/Helv 11 Tf 0 g"),
        "Rect" => rect(180.0, 715.0, 430.0, 733.0), "P" => page, "F" => 4,
        "AP" => dictionary! { "N" => name_ap },
    });
    let email = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "Parent" => person, "T" => text("email"),
        "FT" => "Tx", "DA" => text("/Helv 0 Tf 0 g"),
        "Rect" => rect(180.0, 685.0, 430.0, 703.0), "P" => page, "F" => 4,
    });
    doc.objects.insert(
        person,
        Object::Dictionary(dictionary! {
            "T" => text("person"), "Kids" => vec![name.into(), email.into()],
        }),
    );
    fields.push(person.into());
    widgets.extend([name, email]);

    let bemerkung = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "T" => text("bemerkung"), "FT" => "Tx",
        "Ff" => 1 << 12, "DA" => text("/Helv 10 Tf 0 g"),
        "Rect" => rect(180.0, 610.0, 430.0, 670.0), "P" => page, "F" => 4,
    });
    fields.push(bemerkung.into());
    widgets.push(bemerkung);

    // Kontrollkästchen mit Erscheinungsbildern für "Ja" und "Off".
    let check_on = form_xobject(
        doc,
        14.0,
        14.0,
        "0 g 0.5 w 0.5 0.5 13 13 re S 3 7 m 6 3 l 11 11 l 1.5 w S",
        None,
    );
    let check_off = form_xobject(doc, 14.0, 14.0, "0 g 0.5 w 0.5 0.5 13 13 re S", None);
    let agb = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "T" => text("agb"), "FT" => "Btn",
        "V" => "Off", "AS" => "Off",
        "Rect" => rect(180.0, 586.0, 194.0, 600.0), "P" => page, "F" => 4,
        "AP" => dictionary! { "N" => dictionary! { "Ja" => check_on, "Off" => check_off } },
    });
    fields.push(agb.into());
    widgets.push(agb);

    // Optionsfeld-Gruppe: ein Feld, drei Widgets mit je eigenem An-Zustand.
    let versand = doc.new_object_id();
    let mut kids = Vec::new();
    for (state, x) in [("post", 180.0), ("abholung", 240.0), ("email", 320.0)] {
        let on = form_xobject(
            doc,
            12.0,
            12.0,
            "1 w 0 G 0 g 6 11.5 m 11.5 6 l 6 0.5 l 0.5 6 l h S 6 9 m 9 6 l 6 3 l 3 6 l h f",
            None,
        );
        let off = form_xobject(
            doc,
            12.0,
            12.0,
            "1 w 0 G 6 11.5 m 11.5 6 l 6 0.5 l 0.5 6 l h S",
            None,
        );
        let as_state = if state == "post" { state } else { "Off" };
        let w = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Parent" => versand, "AS" => as_state,
            "Rect" => rect(x, 557.0, x + 12.0, 569.0), "P" => page, "F" => 4,
            "AP" => dictionary! { "N" => dictionary! { state => on, "Off" => off } },
        });
        kids.push(Object::from(w));
        widgets.push(w);
    }
    doc.objects.insert(
        versand,
        Object::Dictionary(dictionary! {
            "T" => text("versand"), "FT" => "Btn", "Ff" => (1 << 15) | (1 << 14),
            "V" => "post", "Kids" => kids,
        }),
    );
    fields.push(versand.into());

    let land = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "T" => text("land"), "FT" => "Ch", "Ff" => 1 << 17,
        "Opt" => vec![text("Deutschland"), winansi("Österreich"), text("Schweiz")],
        "V" => text("Deutschland"), "DA" => text("/Helv 11 Tf 0 g"),
        "Rect" => rect(180.0, 525.0, 330.0, 543.0), "P" => page, "F" => 4,
    });
    let interessen = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "T" => text("interessen"), "FT" => "Ch", "Ff" => 1 << 21,
        "Opt" => vec![
            vec![text("pdf"), text("PDF")].into(),
            vec![text("rust"), text("Rust")].into(),
            vec![text("egui"), text("egui")].into(),
        ],
        "V" => vec![text("rust")], "DA" => text("/Helv 10 Tf 0 g"),
        "Rect" => rect(180.0, 445.0, 330.0, 510.0), "P" => page, "F" => 4,
    });
    let kundennr = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "T" => text("kundennr"), "FT" => "Tx", "Ff" => 1,
        "V" => text("K-12345"), "DA" => text("/Helv 11 Tf 0 g"),
        "Rect" => rect(180.0, 415.0, 330.0, 433.0), "P" => page, "F" => 4,
    });
    let pflicht = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "T" => text("pflicht"), "FT" => "Tx", "Ff" => 2,
        "DA" => text("/Helv 11 Tf 0 g"),
        "Rect" => rect(180.0, 385.0, 330.0, 403.0), "P" => page, "F" => 4,
    });
    let button_ap = form_xobject(
        doc,
        100.0,
        22.0,
        "0.85 g 0 0 100 22 re f BT /Helv 11 Tf 0 g 24 7 Td (Absenden) Tj ET",
        Some(font),
    );
    let absenden = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "T" => text("absenden"), "FT" => "Btn", "Ff" => 1 << 16,
        "Rect" => rect(72.0, 340.0, 172.0, 362.0), "P" => page, "F" => 4,
        "AP" => dictionary! { "N" => button_ap },
    });
    let unterschrift = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "T" => text("unterschrift"), "FT" => "Sig",
        "Rect" => rect(180.0, 260.0, 400.0, 310.0), "P" => page, "F" => 4,
    });
    for id in [land, interessen, kundennr, pflicht, absenden, unterschrift] {
        fields.push(id.into());
        widgets.push(id);
    }

    for w in widgets {
        let p = b
            .doc
            .get_object_mut(page)
            .and_then(Object::as_dict_mut)
            .expect("Seite");
        match p.get_mut(b"Annots") {
            Ok(Object::Array(arr)) => arr.push(w.into()),
            _ => p.set("Annots", vec![Object::from(w)]),
        }
    }
    b.catalog_extra.set(
        "AcroForm",
        dictionary! {
            "Fields" => fields,
            "DA" => text("/Helv 0 Tf 0 g"),
            "DR" => dictionary! { "Font" => dictionary! { "Helv" => font } },
        },
    );
    b.finish(path);
}

/// Alles, was der Content-Interpreter können muss: Kerning (TJ), Abstände, gedrehter und
/// unsichtbarer Text, Zeilen-Operatoren (' und "), Vektorgrafik, Bild, Inline-Bild,
/// Formular-XObject und (Seite 2) eine CID-Schrift mit /W-Breiten.
fn content_mix(path: &Path) {
    let mut b = Builder::new();
    let font = b.font_id;
    // 4x4-Graustufenbild (unkomprimiert, 8 Bit).
    let pixels: Vec<u8> = (0..16).map(|i| (i * 16) as u8).collect();
    let image = b.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 4, "Height" => 4,
            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
        },
        pixels,
    ));
    let form = b.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 200.into(), 20.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT /F1 10 Tf 0 5 Td (Im Formular-XObject) Tj ET".to_vec(),
    ));
    let mut c: Vec<u8> = Vec::new();
    c.extend_from_slice(
        b"BT /F1 12 Tf 72 760 Td (Geheim: 4711-0815) Tj ET\n\
          BT /F1 12 Tf 72 730 Td [(Ke) 120 (rning) -250 (TJ-Test)] TJ ET\n\
          q BT /F1 11 Tf 2 Tc 5 Tw 120 Tz 72 700 Td (Abstand Test) Tj ET Q\n\
          BT /F1 14 Tf 0.7071 0.7071 -0.7071 0.7071 300 600 Tm (Gedreht 45) Tj ET\n\
          q BT 3 Tr /F1 12 Tf 72 640 Td (Unsichtbar OCR) Tj ET Q\n\
          BT /F1 12 Tf 14 TL 72 610 Td (Zeile eins) Tj (Zeile zwei) ' 1 0 (Zeile drei) \" ET\n\
          0 0 1 rg 72 500 100 50 re f\n\
          1 0 0 RG 2 w 200 500 m 300 550 l S\n\
          q 100 0 0 60 350 480 cm /Im1 Do Q\n\
          q 40 0 0 40 480 480 cm BI /W 2 /H 2 /CS /G /BPC 8 ID ",
    );
    c.extend_from_slice(&[0x00, 0x55, 0xAA, 0xFF]);
    c.extend_from_slice(b" EI Q\nq 1 0 0 1 72 420 cm /Fm1 Do Q\n");
    b.raw_page(
        A4,
        c,
        dictionary! {
            "Font" => dictionary! { "F1" => font },
            "XObject" => dictionary! { "Im1" => image, "Fm1" => form },
        },
    );

    // Seite 2: Type0-Schrift (Identity-H), Breiten aus /W. Nicht eingebettet – es geht nur
    // um die Geometrie; Reader zeigen hier eine Ersatzschrift.
    let cid = b.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "Helvetica",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "DW" => 1000,
        "W" => vec![
            Object::from(36), vec![Object::from(600), 700.into(), 800.into()].into(),
            Object::from(40), Object::from(42), Object::from(333),
        ],
    });
    let type0 = b.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "Helvetica",
        "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::from(cid)],
    });
    b.raw_page(
        A4,
        b"BT /F2 20 Tf 100 700 Td <0024002500260028002A0001> Tj ET".to_vec(),
        dictionary! { "Font" => dictionary! { "F2" => type0 } },
    );
    b.finish(path);
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
    form(&dir.join("form.pdf"));
    content_mix(&dir.join("content_mix.pdf"));
}
