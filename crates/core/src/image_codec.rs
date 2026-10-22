//! Bilder kodieren: PNG (verlustfrei) und JPEG (verlustbehaftet, Qualität 1–100).
//! Für den Export von Seiten als Bild und zum Komprimieren von Bildern in PDFs.

use crate::error::{CoreError, Result};

/// Pixelformat der Eingabe (8 Bit je Kanal, Zeile für Zeile, ohne Auffüllung).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Gray,
    Rgb,
    Rgba,
}

impl PixelFormat {
    pub fn channels(self) -> usize {
        match self {
            PixelFormat::Gray => 1,
            PixelFormat::Rgb => 3,
            PixelFormat::Rgba => 4,
        }
    }
}

fn check(data: &[u8], width: u32, height: u32, format: PixelFormat) -> Result<()> {
    let need = width as usize * height as usize * format.channels();
    if width == 0 || height == 0 || data.len() < need {
        return Err(CoreError::Other(format!(
            "Bilddaten passen nicht zu {width}×{height}"
        )));
    }
    Ok(())
}

/// PNG kodieren.
pub fn encode_png(data: &[u8], width: u32, height: u32, format: PixelFormat) -> Result<Vec<u8>> {
    check(data, width, height, format)?;
    let mut out = Vec::new();
    {
        // Eigener Block: Der Encoder leiht `out` aus und muss vor dem Zurückgeben enden.
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(match format {
            PixelFormat::Gray => png::ColorType::Grayscale,
            PixelFormat::Rgb => png::ColorType::Rgb,
            PixelFormat::Rgba => png::ColorType::Rgba,
        });
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc
            .write_header()
            .map_err(|e| CoreError::Other(format!("PNG schreiben: {e}")))?;
        let len = width as usize * height as usize * format.channels();
        w.write_image_data(&data[..len])
            .map_err(|e| CoreError::Other(format!("PNG schreiben: {e}")))?;
    }
    Ok(out)
}

/// JPEG kodieren (Alphakanal wird auf Weiß verrechnet). `quality` 1–100.
pub fn encode_jpeg(
    data: &[u8],
    width: u32,
    height: u32,
    format: PixelFormat,
    quality: u8,
) -> Result<Vec<u8>> {
    check(data, width, height, format)?;
    let (w, h) = (
        u16::try_from(width)
            .map_err(|_| CoreError::Other("JPEG: Bild zu breit (> 65535)".into()))?,
        u16::try_from(height)
            .map_err(|_| CoreError::Other("JPEG: Bild zu hoch (> 65535)".into()))?,
    );
    let rgb_storage;
    let (pixels, color) = match format {
        PixelFormat::Gray => (data, jpeg_encoder::ColorType::Luma),
        PixelFormat::Rgb => (data, jpeg_encoder::ColorType::Rgb),
        PixelFormat::Rgba => {
            // Auf weißen Hintergrund verrechnen: JPEG kennt keine Transparenz.
            rgb_storage = data
                .chunks_exact(4)
                .flat_map(|p| {
                    let a = u32::from(p[3]);
                    let mix = move |c: u8| ((u32::from(c) * a + 255 * (255 - a)) / 255) as u8;
                    [mix(p[0]), mix(p[1]), mix(p[2])]
                })
                .collect::<Vec<u8>>();
            (rgb_storage.as_slice(), jpeg_encoder::ColorType::Rgb)
        }
    };
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100))
        .encode(pixels, w, h, color)
        .map_err(|e| CoreError::Other(format!("JPEG schreiben: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_and_jpeg_roundtrip_sizes() {
        // 4×2 RGBA: links rot, rechts halb durchsichtig blau.
        let mut px = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                px.extend_from_slice(if x < 2 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 255, 128]
                });
            }
        }
        let png = encode_png(&px, 4, 2, PixelFormat::Rgba).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        let img = crate::image_edit::NewImage::from_bytes(&png).unwrap();
        assert_eq!((img.width, img.height), (4, 2));
        assert!(img.smask.is_some());

        let jpg = encode_jpeg(&px, 4, 2, PixelFormat::Rgba, 85).unwrap();
        assert!(jpg.starts_with(&[0xFF, 0xD8]));
        let img = crate::image_edit::NewImage::from_bytes(&jpg).unwrap();
        assert_eq!((img.width, img.height), (4, 2));
        assert!(encode_png(&px[..3], 4, 2, PixelFormat::Rgba).is_err());
    }
}
