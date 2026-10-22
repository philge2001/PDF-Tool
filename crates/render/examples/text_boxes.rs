//! Zeigt die Zeichen-Rechtecke, die PDFium für Seite 1 liefert (Fehlersuche).
//! `FOLIOCRAFT_PDFIUM_PATH=… cargo run -p foliocraft-render --example text_boxes -- datei.pdf [Text]`
use foliocraft_render::{Event, RenderService};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let svc = RenderService::start(|| {}).expect("PDFium");
    svc.open(
        1,
        Arc::new(std::fs::read(&args[0]).expect("lesen")),
        Default::default(),
    );
    svc.extract_text(1, 0);
    loop {
        for e in svc.poll() {
            if let Event::Text { text, .. } = e {
                let needle = args.get(1).map_or("", String::as_str);
                for c in text
                    .chars
                    .iter()
                    .filter(|c| needle.is_empty() || needle.contains(c.ch))
                {
                    println!("{:?} {:?}", c.ch, c.rect);
                }
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
