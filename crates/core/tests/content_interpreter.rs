//! Content-Interpreter gegen die Fixture `content_mix.pdf`.

use pdftool_core::content::{Item, TextShow, interpret_form, interpret_page, page_resources};
use pdftool_core::font;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn load() -> lopdf::Document {
    lopdf::Document::load(fixture("content_mix.pdf")).unwrap()
}

fn page_id(pdf: &lopdf::Document, n: u32) -> lopdf::ObjectId {
    pdf.get_pages()[&n]
}

fn texts(items: &[Item]) -> Vec<&TextShow> {
    items
        .iter()
        .filter_map(|i| match i {
            Item::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn finds_all_text_in_order() {
    let pdf = load();
    let page = interpret_page(&pdf, page_id(&pdf, 1)).unwrap();
    let t: Vec<String> = texts(&page.items).iter().map(|t| t.text()).collect();
    assert_eq!(
        t,
        vec![
            "Geheim: 4711-0815",
            "KerningTJ-Test",
            "Abstand Test",
            "Gedreht 45",
            "Unsichtbar OCR",
            "Zeile eins",
            "Zeile zwei",
            "Zeile drei"
        ]
    );
    let all = texts(&page.items);
    assert!(
        all.iter().all(|t| t.exact),
        "Helvetica-Breiten sind bekannt"
    );
    assert_eq!(all[4].render_mode, 3, "unsichtbarer Text wird erkannt");
    assert_eq!(
        all[5].render_mode, 0,
        "q/Q stellt den Darstellungsmodus wieder her"
    );
}

#[test]
fn glyph_positions_simple_and_kerning() {
    let pdf = load();
    let page = interpret_page(&pdf, page_id(&pdf, 1)).unwrap();
    let t = texts(&page.items);

    // "Geheim: 4711-0815" bei x=72, 12 pt Helvetica.
    let g = &t[0].glyphs;
    assert!(close(g[0].rect.x0, 72.0));
    let width = font::text_width("Geheim: 4711-0815", 12.0);
    assert!(close(g.last().unwrap().rect.x1, 72.0 + width));
    assert!(close(g[0].rect.y0, 760.0 - 0.2 * 12.0) && close(g[0].rect.y1, 760.0 + 0.8 * 12.0));

    // TJ: nach "Ke" kommt eine Verschiebung von 120/1000 em nach links.
    let k = &t[1].glyphs;
    let r_x = 72.0 + font::text_width("Ke", 12.0) - 0.120 * 12.0;
    assert!(close(k[2].rect.x0, r_x), "{} vs {r_x}", k[2].rect.x0);
    assert_eq!(k[2].at.part, 2, "drittes Element im TJ-Array");
    assert_eq!(k[2].at.start, 0);
}

#[test]
fn spacing_and_scaling() {
    let pdf = load();
    let page = interpret_page(&pdf, page_id(&pdf, 1)).unwrap();
    let t = &texts(&page.items)[2];
    assert_eq!((t.char_spacing, t.word_spacing, t.h_scale), (2.0, 5.0, 1.2));
    // Erwartete Position von "T" (nach "Abstand "): Summe der Vorschübe × 1,2.
    let mut x = 72.0;
    for c in "Abstand ".chars() {
        let w = font::char_width(c, 11.0) + 2.0 + if c == ' ' { 5.0 } else { 0.0 };
        x += w * 1.2;
    }
    let tg = &t.glyphs[8];
    assert_eq!(tg.text, "T");
    assert!(close(tg.rect.x0, x), "{} vs {x}", tg.rect.x0);
}

#[test]
fn rotated_text_and_line_operators() {
    let pdf = load();
    let page = interpret_page(&pdf, page_id(&pdf, 1)).unwrap();
    let t = texts(&page.items);
    // 45°: das erste Zeichen beginnt bei (300, 600); spätere liegen rechts OBEN davon.
    let rot = &t[3].glyphs;
    assert!(rot[0].rect.contains(pdftool_core::Point::new(300.5, 600.5)));
    // Ursprung des letzten Zeichens: Breite von "Gedreht 4" entlang der 45°-Richtung.
    // 0.7071 steht so in der Fixture (Tm) – bewusst nicht die exakte Konstante 1/√2.
    #[allow(clippy::approx_constant)]
    let d = font::text_width("Gedreht 4", 14.0) * 0.7071;
    let origin = pdftool_core::Point::new(300.0 + d + 0.5, 600.0 + d + 0.5);
    let last = rot.last().unwrap().rect;
    assert!(last.contains(origin), "{last:?} {origin:?}");

    // ' und ": je eine Zeile (TL 14) tiefer.
    let baseline = |s: &TextShow| s.glyphs[0].rect.y0 + 0.2 * 12.0;
    assert!(close(baseline(t[5]), 610.0));
    assert!(close(baseline(t[6]), 596.0));
    assert!(close(baseline(t[7]), 582.0));
    assert_eq!((t[7].word_spacing, t[7].char_spacing), (1.0, 0.0));
}

#[test]
fn paths_images_and_forms() {
    let pdf = load();
    let page = interpret_page(&pdf, page_id(&pdf, 1)).unwrap();
    let mut paths = Vec::new();
    let mut images = Vec::new();
    let mut forms = Vec::new();
    for item in &page.items {
        match item {
            Item::Path {
                bounds, painted, ..
            } => paths.push((*bounds, *painted)),
            Item::Image { name, bounds, .. } => images.push((name.clone(), *bounds)),
            Item::Form { name, matrix, .. } => forms.push((name.clone(), *matrix)),
            _ => {}
        }
    }
    let rect = |r: pdftool_core::Rect| (r.x0, r.y0, r.x1, r.y1);
    assert_eq!(paths.len(), 2);
    assert_eq!(rect(paths[0].0), (72.0, 500.0, 172.0, 550.0));
    assert_eq!(rect(paths[1].0), (200.0, 500.0, 300.0, 550.0));
    assert!(paths.iter().all(|p| p.1));
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].0.as_deref(), Some(&b"Im1"[..]));
    assert_eq!(rect(images[0].1), (350.0, 480.0, 450.0, 540.0));
    assert_eq!(images[1].0, None, "Inline-Bild");
    assert_eq!(rect(images[1].1), (480.0, 480.0, 520.0, 520.0));

    // Formular-XObject: eigener Inhalt, versetzt um (72, 420).
    assert_eq!(forms.len(), 1);
    let resources = page_resources(&pdf, page_id(&pdf, 1));
    let xobjects = resources
        .unwrap()
        .get(b"XObject")
        .unwrap()
        .as_dict()
        .unwrap();
    let id = xobjects.get(b"Fm1").unwrap().as_reference().unwrap();
    let stream = pdf.get_object(id).unwrap().as_stream().unwrap();
    let inner = interpret_form(&pdf, stream, resources, forms[0].1).unwrap();
    let t = texts(&inner.items);
    assert_eq!(t[0].text(), "Im Formular-XObject");
    assert!(close(t[0].glyphs[0].rect.x0, 72.0));
    assert!(close(t[0].glyphs[0].rect.y0, 425.0 - 0.2 * 10.0));
}

#[test]
fn cid_font_widths() {
    let pdf = load();
    let page = interpret_page(&pdf, page_id(&pdf, 2)).unwrap();
    let t = texts(&page.items)[0];
    assert!(t.two_byte && t.exact);
    let codes: Vec<u32> = t.glyphs.iter().map(|g| g.code).collect();
    assert_eq!(codes, vec![0x24, 0x25, 0x26, 0x28, 0x2A, 0x01]);
    // /W: 36..38 = 600/700/800, 40..42 = 333, sonst /DW 1000.
    let total: f32 = t.glyphs.iter().map(|g| g.advance).sum();
    assert!(close(
        total,
        (600.0 + 700.0 + 800.0 + 333.0 + 333.0 + 1000.0) / 1000.0 * 20.0
    ));
    assert!(close(t.glyphs[1].rect.x0, 100.0 + 0.6 * 20.0));
}
