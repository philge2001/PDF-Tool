//! Texterkennung auf einem Graustufenbild (PGM, binär "P5") – zur Gegenprobe.
//!
//! `cargo run -p pdftool-ocr --example ocr_demo -- bild.pgm [deu+eng] [dpi]`
//! (PGM z. B. mit `convert bild.png -colorspace gray bild.pgm` erzeugen)
#![allow(clippy::disallowed_methods)] // Zeitmessung im Desktop-Werkzeug (nicht im Browser)

use pdftool_ocr::{Engine, Image};

/// Liest ein binäres PGM: Kopf "P5 breite höhe maxwert", dann ein Byte pro Pixel.
fn read_pgm(bytes: &[u8]) -> (u32, u32, &[u8]) {
    let mut fields = Vec::new();
    let mut pos = 0;
    while fields.len() < 4 {
        while bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if bytes[pos] == b'#' {
            while bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        fields.push(String::from_utf8_lossy(&bytes[start..pos]).into_owned());
    }
    assert_eq!(fields[0], "P5", "nur binäres PGM");
    assert_eq!(fields[3], "255", "nur 8 Bit (ImageMagick: -depth 8)");
    let w: u32 = fields[1].parse().expect("Breite");
    let h: u32 = fields[2].parse().expect("Höhe");
    (w, h, &bytes[pos + 1..])
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("Aufruf: ocr_demo bild.pgm [deu+eng] [dpi]");
        std::process::exit(2);
    };
    let bytes = std::fs::read(path).expect("Bild lesen");
    let (width, height, data) = read_pgm(&bytes);
    let langs = args.get(1).map_or("deu+eng", String::as_str);
    let dpi = args.get(2).and_then(|d| d.parse().ok()).unwrap_or(300);
    let start = std::time::Instant::now();
    let engine = Engine::new(langs).expect("Tesseract starten");
    let words = engine
        .recognize(&Image {
            data,
            width,
            height,
            bytes_per_pixel: 1,
            dpi,
        })
        .expect("erkennen");
    for w in &words {
        println!(
            "{:>5} {:>5} {:>4}×{:<4} {:>5.1}%  {}",
            w.left, w.top, w.width, w.height, w.confidence, w.text
        );
    }
    eprintln!("{} Wörter in {:.2?}", words.len(), start.elapsed());
}
