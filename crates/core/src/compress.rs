//! PDF verkleinern (Phase 8): Bilder herunterrechnen, Streams komprimieren, unbenutzte
//! Objekte entfernen, Objekt-Streams nutzen – die Datei wird komplett neu geschrieben.
//!
//! **Bilder:** Für jedes Bild-XObject wird die größte Darstellung im Dokument bestimmt (auf
//! allen Seiten und in Formular-XObjects). Liegt die effektive Auflösung deutlich über dem
//! Ziel, wird das Bild verkleinert (Flächenmittel) und als JPEG neu kodiert; eine /SMask
//! (Transparenz) wird mitverkleinert. Unangetastet bleiben Masken, Bilder mit /Mask oder
//! /Decode und andere Farbräume als Grau/RGB – lieber etwas größer als kaputt.

use crate::content::{self, Item, Matrix};
use crate::error::{CoreError, Result};
use crate::image_codec::{self, PixelFormat};
use crate::raster::Raster;
use lopdf::{Dictionary, Object, ObjectId, Stream, dictionary};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressOptions {
    /// Ziel-Auflösung für Bilder; `None` = Bilder nicht verkleinern.
    pub image_dpi: Option<u32>,
    /// JPEG-Qualität für neu kodierte Bilder (1–100).
    pub jpeg_quality: u8,
}

impl Default for CompressOptions {
    fn default() -> Self {
        Self {
            image_dpi: Some(150),
            jpeg_quality: 75,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompressReport {
    pub size_before: usize,
    pub size_after: usize,
    pub images_resampled: usize,
    pub objects_removed: usize,
}

impl CompressReport {
    pub fn summary(&self) -> String {
        let pct = if self.size_before > 0 {
            100.0 - self.size_after as f64 * 100.0 / self.size_before as f64
        } else {
            0.0
        };
        format!(
            "Vorher {}, nachher {} ({pct:.0} % kleiner).\n{} Bild(er) verkleinert, {} unbenutzte Objekt(e) entfernt.",
            human(self.size_before),
            human(self.size_after),
            self.images_resampled,
            self.objects_removed
        )
    }
}

/// Größe lesbar: "1,2 MB", "340 KB".
pub fn human(bytes: usize) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / f64::from(1u32 << 20)).replace('.', ",")
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// Größte Darstellung (Breite, Höhe in Punkt) je Bild-Objekt.
type Usage = HashMap<ObjectId, (f32, f32)>;

fn record(usage: &mut Usage, id: ObjectId, m: &Matrix) {
    // Seitenlängen des Einheitsquadrats nach der Abbildung = dargestellte Größe.
    let w = m.0[0].hypot(m.0[1]);
    let h = m.0[2].hypot(m.0[3]);
    let e = usage.entry(id).or_insert((0.0, 0.0));
    e.0 = e.0.max(w);
    e.1 = e.1.max(h);
}

fn xobject_id(
    pdf: &lopdf::Document,
    resources: Option<&Dictionary>,
    name: &[u8],
) -> Option<ObjectId> {
    let xobjects = resources?.get(b"XObject").ok()?;
    let xobjects = pdf.dereference(xobjects).ok()?.1.as_dict().ok()?;
    xobjects.get(name).ok()?.as_reference().ok()
}

/// Sammelt die Bildverwendungen eines Inhalts (rekursiv in Formular-XObjects, begrenzt tief).
fn collect(
    pdf: &lopdf::Document,
    it: &content::Interpreted,
    resources: Option<&Dictionary>,
    usage: &mut Usage,
    depth: usize,
) {
    for item in &it.items {
        match item {
            Item::Image {
                name: Some(name),
                matrix,
                ..
            } => {
                if let Some(id) = xobject_id(pdf, resources, name) {
                    record(usage, id, matrix);
                }
            }
            Item::Form {
                id: Some(id),
                matrix,
                ..
            } if depth < 8 => {
                let Ok(form) = pdf.get_object(*id).and_then(Object::as_stream) else {
                    continue;
                };
                let form_res = form
                    .dict
                    .get(b"Resources")
                    .ok()
                    .and_then(|o| pdf.dereference(o).ok())
                    .and_then(|(_, o)| o.as_dict().ok())
                    .or(resources);
                if let Ok(inner) = content::interpret_form(pdf, form, resources, *matrix) {
                    collect(pdf, &inner, form_res, usage, depth + 1);
                }
            }
            _ => {}
        }
    }
}

/// Flächenmittel-Verkleinerung (jedes Zielpixel = Mittel der überdeckten Quellpixel).
fn resample(data: &[u8], w: usize, h: usize, comps: usize, nw: usize, nh: usize) -> Vec<u8> {
    let mut out = vec![0u8; nw * nh * comps];
    for ny in 0..nh {
        let y0 = ny * h / nh;
        let y1 = ((ny + 1) * h / nh).max(y0 + 1).min(h);
        for nx in 0..nw {
            let x0 = nx * w / nw;
            let x1 = ((nx + 1) * w / nw).max(x0 + 1).min(w);
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            for c in 0..comps {
                let mut sum = 0u32;
                for y in y0..y1 {
                    let row = y * w * comps;
                    for x in x0..x1 {
                        sum += u32::from(data[row + x * comps + c]);
                    }
                }
                out[(ny * nw + nx) * comps + c] = ((sum + count / 2) / count) as u8;
            }
        }
    }
    out
}

/// Darf dieses Bild neu kodiert werden? Liefert das Pixelformat.
fn eligible(stream: &Stream) -> Option<PixelFormat> {
    let d = &stream.dict;
    if d.get(b"ImageMask")
        .and_then(Object::as_bool)
        .unwrap_or(false)
        || d.has(b"Mask")
        || d.has(b"Decode")
        || d.get(b"BitsPerComponent").and_then(Object::as_i64).ok() != Some(8)
    {
        return None;
    }
    match d.get(b"ColorSpace").and_then(Object::as_name).ok()? {
        b"DeviceGray" => Some(PixelFormat::Gray),
        b"DeviceRGB" => Some(PixelFormat::Rgb),
        _ => None,
    }
}

/// Verkleinert ein Bild (und seine /SMask) auf `nw` × `nh`. `false`, wenn nicht möglich.
fn shrink_image(
    pdf: &mut lopdf::Document,
    id: ObjectId,
    nw: usize,
    nh: usize,
    quality: u8,
) -> bool {
    let Ok(stream) = pdf.get_object(id).and_then(Object::as_stream) else {
        return false;
    };
    let Some(format) = eligible(stream) else {
        return false;
    };
    let Ok(r) = Raster::decode(pdf, stream) else {
        return false;
    };
    if r.comps != format.channels() || r.bpc != 8 {
        return false;
    }
    let small = resample(&r.data, r.width, r.height, r.comps, nw, nh);
    let Ok(jpeg) = image_codec::encode_jpeg(&small, nw as u32, nh as u32, format, quality) else {
        return false;
    };
    // Transparenz mitverkleinern.
    let smask = stream
        .dict
        .get(b"SMask")
        .and_then(Object::as_reference)
        .ok();
    if let Some(mask_id) = smask {
        let Ok(mask) = pdf.get_object(mask_id).and_then(Object::as_stream) else {
            return false;
        };
        let Ok(m) = Raster::decode(pdf, mask) else {
            return false;
        };
        if m.comps != 1 || m.bpc != 8 {
            return false;
        }
        let small_mask = resample(&m.data, m.width, m.height, 1, nw, nh);
        let mut new_mask = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => nw as i64, "Height" => nh as i64,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            small_mask,
        );
        let _ = new_mask.compress();
        pdf.objects.insert(mask_id, Object::Stream(new_mask));
    }
    let Ok(Object::Stream(s)) = pdf.get_object_mut(id) else {
        return false;
    };
    s.dict.set("Width", nw as i64);
    s.dict.set("Height", nh as i64);
    s.dict.set("Filter", "DCTDecode");
    s.dict.remove(b"DecodeParms");
    s.set_content(jpeg);
    true
}

/// Komprimiert eine PDF. `progress(Anteil, Text)` → `false` bricht ab.
pub fn compress_pdf(
    bytes: &[u8],
    options: &CompressOptions,
    progress: &dyn Fn(f32, String) -> bool,
) -> Result<(Vec<u8>, CompressReport)> {
    let mut pdf = lopdf::Document::load_mem(bytes)?;
    if pdf.is_encrypted() {
        return Err(CoreError::Other(
            "Verschlüsselte PDFs können nicht komprimiert werden.".into(),
        ));
    }
    let mut report = CompressReport {
        size_before: bytes.len(),
        ..Default::default()
    };

    if let Some(dpi) = options.image_dpi {
        // 1) Wo wird welches Bild wie groß dargestellt?
        let pages: Vec<ObjectId> = pdf.get_pages().into_values().collect();
        let mut usage = Usage::new();
        for (i, &page) in pages.iter().enumerate() {
            if !progress(
                0.5 * i as f32 / pages.len().max(1) as f32,
                format!("Seite {} untersuchen …", i + 1),
            ) {
                return Err(CoreError::Other("Abgebrochen.".into()));
            }
            if let Ok(it) = content::interpret_page(&pdf, page) {
                collect(
                    &pdf,
                    &it,
                    content::page_resources(&pdf, page),
                    &mut usage,
                    0,
                );
            }
        }
        // 2) Zu hoch aufgelöste Bilder verkleinern.
        let total = usage.len().max(1);
        for (k, (id, (w_pt, h_pt))) in usage.into_iter().enumerate() {
            if !progress(
                0.5 + 0.4 * k as f32 / total as f32,
                "Bilder verkleinern …".into(),
            ) {
                return Err(CoreError::Other("Abgebrochen.".into()));
            }
            let Ok(stream) = pdf.get_object(id).and_then(Object::as_stream) else {
                continue;
            };
            let px = |key: &[u8]| stream.dict.get(key).and_then(Object::as_i64).unwrap_or(0) as f32;
            let (w_px, h_px) = (px(b"Width"), px(b"Height"));
            // Benötigte Pixel bei Ziel-dpi (Darstellung in Zoll × dpi).
            let need_w = (w_pt / 72.0 * dpi as f32).ceil().max(1.0);
            let need_h = (h_pt / 72.0 * dpi as f32).ceil().max(1.0);
            // Nur, wenn es sich lohnt (mehr als 25 % zu viel) – und nie vergrößern.
            if w_px <= need_w * 1.25 && h_px <= need_h * 1.25 {
                continue;
            }
            let f = (need_w / w_px).max(need_h / h_px).min(1.0);
            let (nw, nh) = (
                ((w_px * f).round() as usize).max(1),
                ((h_px * f).round() as usize).max(1),
            );
            if shrink_image(&mut pdf, id, nw, nh, options.jpeg_quality) {
                report.images_resampled += 1;
            }
        }
    }

    // 3) Aufräumen und kompakt neu schreiben.
    progress(0.9, "Schreiben …".into());
    pdf.trailer.remove(b"Prev");
    pdf.trailer.remove(b"XRefStm");
    report.objects_removed = pdf.prune_objects().len();
    pdf.compress();
    let mut out = Vec::new();
    let opts = lopdf::SaveOptions::builder()
        .use_object_streams(true)
        .compression_level(9)
        .build();
    pdf.save_with_options(&mut out, opts)
        .map_err(|e| CoreError::Other(format!("PDF schreiben: {e}")))?;
    report.size_after = out.len();
    Ok((out, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_averages() {
        // 4×2 Grau -> 2×1: Mittel von je 2×2.
        let data = [0, 100, 200, 255, 0, 100, 200, 255];
        assert_eq!(resample(&data, 4, 2, 1, 2, 1), vec![50, 228]);
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(1500), "2 KB");
        assert_eq!(human(3 << 20), "3,0 MB");
    }
}
