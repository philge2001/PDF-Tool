//! Rendert Seite 1 offscreen als PGM (Fehlersuche).
use foliocraft_render::RenderService;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scale: f32 = args[2].parse().unwrap();
    let svc = RenderService::start(|| {}).expect("PDFium");
    let off = svc.offscreen().unwrap();
    let bytes = Arc::new(std::fs::read(&args[0]).unwrap());
    let img = off.render_page(&bytes, 0, (595.0, 842.0), scale).unwrap();
    let mut out = format!("P5 {} {} 255\n", img.width, img.height).into_bytes();
    out.extend(img.pixels.chunks_exact(4).map(|p| p[0]));
    std::fs::write(&args[1], out).unwrap();
}
