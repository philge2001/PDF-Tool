//! Bilddaten aus PDF-Bildern lesen, Bereiche schwärzen, wieder schreiben.
//!
//! Unterstützt: unkomprimiert, Flate/LZW (über lopdf) und JPEG (DCTDecode, über zune-jpeg);
//! Farbräume Grau, RGB, CMYK, ICCBased, Indexed; Bildmasken. Geschrieben wird immer
//! verlustfrei mit Flate. Alles andere (JPEG 2000, JBIG2, CCITT …) meldet einen Fehler –
//! der Aufrufer entfernt das Bild dann komplett.

use lopdf::{Dictionary, Object, Stream};

/// Dekodierte Bilddaten.
pub struct Raster {
    pub width: usize,
    pub height: usize,
    /// Farbkomponenten pro Pixel (1 bei Indexed und Masken).
    pub comps: usize,
    pub bpc: u8,
    /// Zeilen sind auf volle Bytes aufgefüllt (wie in PDF).
    pub data: Vec<u8>,
    /// Rohwerte, die "schwarz" bzw. bei Masken "nicht gemalt" bedeuten – je Komponente.
    black: Vec<u16>,
    /// Neuer Farbraum, falls sich beim Dekodieren etwas ändert (JPEG -> RGB/Grau/CMYK).
    color_space: Option<Object>,
    /// /Decode muss beim Schreiben entfallen (JPEG dekodiert schon "fertig").
    drop_decode: bool,
}

fn get<'a>(d: &'a Dictionary, long: &[u8], short: &[u8]) -> Option<&'a Object> {
    d.get(long).or_else(|_| d.get(short)).ok()
}

fn name_of(o: &Object) -> Option<&[u8]> {
    o.as_name().ok()
}

/// Inline-Bilder benutzen Abkürzungen (/W, /CS /G …). Auf die langen Namen bringen.
pub fn normalize_inline(dict: &Dictionary) -> Dictionary {
    let mut out = Dictionary::new();
    for (k, v) in dict.iter() {
        let key: &[u8] = match k.as_slice() {
            b"W" => b"Width",
            b"H" => b"Height",
            b"CS" => b"ColorSpace",
            b"BPC" => b"BitsPerComponent",
            b"F" => b"Filter",
            b"DP" => b"DecodeParms",
            b"IM" => b"ImageMask",
            b"D" => b"Decode",
            other => other,
        };
        let expand = |n: &[u8]| -> Vec<u8> {
            match n {
                b"G" => b"DeviceGray".to_vec(),
                b"RGB" => b"DeviceRGB".to_vec(),
                b"CMYK" => b"DeviceCMYK".to_vec(),
                b"I" => b"Indexed".to_vec(),
                b"Fl" => b"FlateDecode".to_vec(),
                b"DCT" => b"DCTDecode".to_vec(),
                b"AHx" => b"ASCIIHexDecode".to_vec(),
                b"A85" => b"ASCII85Decode".to_vec(),
                b"LZW" => b"LZWDecode".to_vec(),
                b"RL" => b"RunLengthDecode".to_vec(),
                b"CCF" => b"CCITTFaxDecode".to_vec(),
                other => other.to_vec(),
            }
        };
        let value = match v {
            Object::Name(n) => Object::Name(expand(n)),
            Object::Array(a) => Object::Array(
                a.iter()
                    .map(|o| match o {
                        Object::Name(n) => Object::Name(expand(n)),
                        o => o.clone(),
                    })
                    .collect(),
            ),
            o => o.clone(),
        };
        out.set(key, value);
    }
    out
}

/// Anzahl Komponenten und Rohwert für "schwarz" je Komponente (ohne /Decode-Umkehr).
fn color_info(
    pdf: &lopdf::Document,
    cs: Option<&Object>,
) -> Result<(usize, Vec<u16>, u16), String> {
    let cs = match cs {
        Some(o) => pdf.dereference(o).map(|(_, o)| o).unwrap_or(o),
        None => return Err("Bild ohne Farbraum".into()),
    };
    let kind = match cs {
        Object::Name(n) => n.clone(),
        Object::Array(a) => a
            .first()
            .and_then(name_of)
            .map(<[u8]>::to_vec)
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    match kind.as_slice() {
        b"DeviceGray" | b"CalGray" | b"G" => Ok((1, vec![0], 0)),
        b"DeviceRGB" | b"CalRGB" | b"RGB" => Ok((3, vec![0, 0, 0], 0)),
        b"DeviceCMYK" | b"CMYK" => Ok((4, vec![0, 0, 0, u16::MAX], 0)),
        b"ICCBased" => {
            let n = match cs {
                Object::Array(a) => a
                    .get(1)
                    .and_then(|o| pdf.dereference(o).ok())
                    .and_then(|(_, o)| o.as_stream().ok())
                    .and_then(|s| s.dict.get(b"N").ok())
                    .and_then(|o| o.as_i64().ok())
                    .unwrap_or(3),
                _ => 3,
            };
            match n {
                1 => Ok((1, vec![0], 0)),
                4 => Ok((4, vec![0, 0, 0, u16::MAX], 0)),
                _ => Ok((3, vec![0, 0, 0], 0)),
            }
        }
        b"Indexed" | b"I" => {
            // Palette durchsuchen: Index der dunkelsten Farbe.
            let Object::Array(a) = cs else {
                return Err("Indexed ohne Palette".into());
            };
            let base = a.get(1);
            let (base_comps, ..) = color_info(pdf, base)?;
            let lookup: Vec<u8> = match a.get(3).map(|o| pdf.dereference(o).map(|(_, o)| o)) {
                Some(Ok(Object::String(s, _))) => s.clone(),
                Some(Ok(Object::Stream(s))) => s.decompressed_content().unwrap_or_default(),
                _ => return Err("Palette unlesbar".into()),
            };
            let darkest = lookup
                .chunks(base_comps)
                .enumerate()
                .min_by_key(|(_, c)| match base_comps {
                    // Bei CMYK ist "viel K" dunkel.
                    4 => 255u32.saturating_sub(u32::from(c[3])) * 4,
                    _ => c.iter().map(|&v| u32::from(v)).sum(),
                })
                .map_or(0, |(i, _)| i as u16);
            Ok((1, vec![darkest], 1))
        }
        other => Err(format!(
            "Farbraum {} wird beim Schwärzen nicht unterstützt",
            String::from_utf8_lossy(other)
        )),
    }
}

impl Raster {
    /// Liest ein Bild. `dict` mit langen Schlüsselnamen (siehe [`normalize_inline`]).
    pub fn decode(pdf: &lopdf::Document, stream: &Stream) -> Result<Self, String> {
        let d = &stream.dict;
        let int = |k: &[u8], s: &[u8]| get(d, k, s).and_then(|o| o.as_i64().ok());
        let width = int(b"Width", b"W").ok_or("Bild ohne Breite")? as usize;
        let height = int(b"Height", b"H").ok_or("Bild ohne Höhe")? as usize;
        if width == 0 || height == 0 || width.saturating_mul(height) > 200_000_000 {
            return Err("Ungültige Bildgröße".into());
        }
        let filters: Vec<Vec<u8>> = match get(d, b"Filter", b"F") {
            Some(Object::Name(n)) => vec![n.clone()],
            Some(Object::Array(a)) => a.iter().filter_map(name_of).map(<[u8]>::to_vec).collect(),
            _ => Vec::new(),
        };
        let is_mask = get(d, b"ImageMask", b"IM").and_then(|o| o.as_bool().ok()) == Some(true);
        let decode_arr: Vec<f32> = get(d, b"Decode", b"D")
            .and_then(|o| o.as_array().ok())
            .map(|a| a.iter().filter_map(|o| o.as_float().ok()).collect())
            .unwrap_or_default();
        // Komponente i ist umgekehrt, wenn /Decode [1 0] statt [0 1] sagt.
        let inverted = |i: usize| {
            decode_arr
                .get(2 * i)
                .zip(decode_arr.get(2 * i + 1))
                .is_some_and(|(a, b)| a > b)
        };

        if filters.last().map(Vec::as_slice) == Some(b"DCTDecode") {
            if filters.len() > 1 {
                return Err("Verkettete Filter mit JPEG werden nicht unterstützt".into());
            }
            return Self::decode_jpeg(&stream.content);
        }
        if filters.iter().any(|f| {
            !matches!(
                f.as_slice(),
                b"FlateDecode" | b"LZWDecode" | b"ASCIIHexDecode" | b"ASCII85Decode"
            )
        }) {
            let names: Vec<String> = filters
                .iter()
                .map(|f| String::from_utf8_lossy(f).into_owned())
                .collect();
            return Err(format!(
                "Bildformat {} wird nicht unterstützt",
                names.join("+")
            ));
        }
        let data = if filters.is_empty() {
            stream.content.clone()
        } else {
            stream
                .decompressed_content()
                .map_err(|e| format!("Bild nicht entpackbar: {e}"))?
        };

        let (comps, mut black, bpc_default) = if is_mask {
            // Maske: 1 = "nicht malen" (bei /Decode [1 0] umgekehrt).
            (1, vec![1], 1)
        } else {
            color_info(pdf, get(d, b"ColorSpace", b"CS"))?
        };
        let bpc = int(b"BitsPerComponent", b"BPC").unwrap_or(i64::from(bpc_default)) as u8;
        if !matches!(bpc, 1 | 2 | 4 | 8 | 16) || (comps > 1 && bpc < 8) {
            return Err(format!("{bpc} Bit pro Komponente werden nicht unterstützt"));
        }
        let max = (1u32 << bpc) - 1;
        for (i, b) in black.iter_mut().enumerate() {
            // u16::MAX steht für "Maximalwert" (z. B. K bei CMYK).
            let v = if *b == u16::MAX { max as u16 } else { *b };
            *b = if inverted(i) { max as u16 - v } else { v };
        }
        let row = (width * comps * bpc as usize).div_ceil(8);
        if data.len() < row * height {
            return Err("Bilddaten zu kurz".into());
        }
        Ok(Self {
            width,
            height,
            comps,
            bpc,
            data,
            black,
            color_space: None,
            drop_decode: false,
        })
    }

    fn decode_jpeg(bytes: &[u8]) -> Result<Self, String> {
        use zune_jpeg::JpegDecoder;
        use zune_jpeg::zune_core::bytestream::ZCursor;
        use zune_jpeg::zune_core::colorspace::ColorSpace;
        let mut dec = JpegDecoder::new(ZCursor::new(bytes));
        let data = dec.decode().map_err(|e| format!("JPEG unlesbar: {e:?}"))?;
        let info = dec.info().ok_or("JPEG ohne Kopfdaten")?;
        let (width, height) = (usize::from(info.width), usize::from(info.height));
        let cs = dec.output_colorspace().unwrap_or(ColorSpace::RGB);
        let comps = cs.num_components();
        let (space, black): (&str, Vec<u16>) = match comps {
            1 => ("DeviceGray", vec![0]),
            4 => ("DeviceCMYK", vec![0, 0, 0, 255]),
            _ => ("DeviceRGB", vec![0, 0, 0]),
        };
        if data.len() < width * height * comps {
            return Err("JPEG-Daten zu kurz".into());
        }
        Ok(Self {
            width,
            height,
            comps,
            bpc: 8,
            data,
            black,
            color_space: Some(Object::Name(space.as_bytes().to_vec())),
            drop_decode: true,
        })
    }

    /// Färbt die Pixel x0..x1, y0..y1 (Zeile 0 = oben) schwarz bzw. maskiert sie aus.
    pub fn blacken(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        let (x1, y1) = (x1.min(self.width), y1.min(self.height));
        let bits = self.bpc as usize;
        let row = (self.width * self.comps * bits).div_ceil(8);
        for y in y0..y1 {
            for x in x0..x1 {
                for c in 0..self.comps {
                    let value = self.black[c];
                    let sample = x * self.comps + c;
                    match bits {
                        8 => self.data[y * row + sample] = value as u8,
                        16 => {
                            let i = y * row + sample * 2;
                            self.data[i..i + 2].copy_from_slice(&value.to_be_bytes());
                        }
                        _ => {
                            // 1/2/4 Bit: Wert in das richtige Bitfeld des Bytes schreiben.
                            let bit = sample * bits;
                            let byte = y * row + bit / 8;
                            let shift = 8 - bits - bit % 8;
                            let mask = (((1u16 << bits) - 1) as u8) << shift;
                            self.data[byte] =
                                (self.data[byte] & !mask) | ((value as u8) << shift & mask);
                        }
                    }
                }
            }
        }
    }

    /// Neues Bild-Stream (Flate) mit dem Wörterbuch des alten als Basis.
    pub fn encode(&self, original: &Dictionary) -> Stream {
        let mut dict = original.clone();
        for k in [&b"Filter"[..], b"DecodeParms", b"F", b"DP", b"Length"] {
            dict.remove(k);
        }
        if let Some(cs) = &self.color_space {
            dict.remove(b"CS");
            dict.set("ColorSpace", cs.clone());
            dict.set("BitsPerComponent", 8);
            dict.remove(b"BPC");
        }
        if self.drop_decode {
            dict.remove(b"Decode");
            dict.remove(b"D");
        }
        let mut s = Stream::new(dict, self.data.clone());
        // `compress` setzt /Filter /FlateDecode, wenn es sich lohnt.
        let _ = s.compress();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    fn blacken_gray_and_bits() {
        let pdf = lopdf::Document::new();
        let s = Stream::new(
            dictionary! { "Width" => 4, "Height" => 2, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
            vec![255; 8],
        );
        let mut r = Raster::decode(&pdf, &s).unwrap();
        r.blacken(1, 0, 3, 1);
        assert_eq!(r.data, vec![255, 0, 0, 255, 255, 255, 255, 255]);

        // 1-Bit-Maske 8x1: 0xFF = alles "nicht malen"; Pixel 2..4 sollen "nicht malen" bleiben.
        let m = Stream::new(
            dictionary! { "Width" => 8, "Height" => 1, "ImageMask" => true },
            vec![0x00],
        );
        let mut r = Raster::decode(&pdf, &m).unwrap();
        r.blacken(2, 0, 4, 1);
        assert_eq!(r.data, vec![0b0011_0000]);
    }

    #[test]
    fn inverted_decode_and_inline_names() {
        let pdf = lopdf::Document::new();
        let inline = dictionary! { "W" => 2, "H" => 1, "CS" => "G", "BPC" => 8, "D" => vec![1.into(), 0.into()] };
        let s = Stream::new(normalize_inline(&inline), vec![0, 0]);
        let mut r = Raster::decode(&pdf, &s).unwrap();
        r.blacken(0, 0, 1, 1);
        assert_eq!(r.data, vec![255, 0], "bei /Decode [1 0] ist 255 schwarz");
    }

    #[test]
    fn unsupported_formats_are_errors() {
        let pdf = lopdf::Document::new();
        let s = Stream::new(
            dictionary! { "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 1, "Filter" => "CCITTFaxDecode" },
            vec![0],
        );
        assert!(Raster::decode(&pdf, &s).is_err());
    }
}
