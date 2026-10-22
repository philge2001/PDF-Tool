//! Gibt die vom Content-Interpreter gefundenen Objekte einer Seite aus (zur Gegenprobe).
//!
//! `cargo run -p foliocraft-core --example content_dump -- datei.pdf [Seite]`

use foliocraft_core::content::{Item, interpret_page};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pdf = lopdf::Document::load(&args[0]).expect("PDF laden");
    let n: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let page = pdf.get_pages()[&n];
    let height = foliocraft_core::Document::open(&args[0])
        .map(|d| d.pages()[n as usize - 1].crop_box.y1)
        .unwrap_or(842.0);
    for item in interpret_page(&pdf, page).expect("interpretieren").items {
        match item {
            Item::Text(t) => {
                // Wörter wie pdftotext -bbox: x links/rechts, y von oben gemessen.
                let mut word = String::new();
                let mut rect: Option<foliocraft_core::Rect> = None;
                let flush = |w: &mut String, r: &mut Option<foliocraft_core::Rect>| {
                    if let Some(r) = r.take() {
                        println!(
                            "{:<22} x {:7.2}..{:7.2}  y {:7.2}..{:7.2}",
                            w,
                            r.x0,
                            r.x1,
                            height - r.y1,
                            height - r.y0
                        );
                    }
                    w.clear();
                };
                for g in &t.glyphs {
                    if g.text.trim().is_empty() {
                        flush(&mut word, &mut rect);
                        continue;
                    }
                    word.push_str(&g.text);
                    rect = Some(rect.map_or(g.rect, |r| r.union(&g.rect)));
                }
                flush(&mut word, &mut rect);
            }
            other => println!("{other:?}"),
        }
    }
}
