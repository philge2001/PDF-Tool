//! PDF-Textstrings (z. B. /Contents, /T): Kodieren und Dekodieren.
//!
//! Laut PDF-Standard ist ein Textstring entweder PDFDocEncoding (1 Byte pro Zeichen)
//! oder UTF-16BE mit Byte-Order-Mark FE FF (PDF 2.0 erlaubt zusätzlich UTF-8 mit BOM).

use lopdf::{Object, StringFormat};

/// Zeichen 0x80..=0xA0 in PDFDocEncoding (weicht dort von Latin-1 ab).
const PDFDOC_HIGH: [char; 33] = [
    '•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘', '’', '‚', '™',
    'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{FFFD}', '€',
];

/// Kodiert Text als PDF-Textstring: reines ASCII direkt, sonst UTF-16BE mit BOM.
pub fn encode(text: &str) -> Object {
    if text.is_ascii() {
        Object::String(text.as_bytes().to_vec(), StringFormat::Literal)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

/// Dekodiert einen PDF-Textstring.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        // `chunks_exact(2)` liefert Byte-Paare; ein evtl. einzelnes Restbyte wird ignoriert.
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0xA0 => PDFDOC_HIGH[(b - 0x80) as usize],
            _ => b as char, // Rest entspricht Latin-1
        })
        .collect()
}

/// Liest einen Textstring aus einem Dictionary (fehlend/ungültig -> leer).
pub fn get(dict: &lopdf::Dictionary, key: &[u8]) -> String {
    dict.get(key)
        .ok()
        .and_then(|o| o.as_str().ok())
        .map(decode)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ascii_and_unicode() {
        for text in ["Hallo", "Grüße aus Köln – €", "日本語", ""] {
            let Object::String(bytes, _) = encode(text) else {
                panic!()
            };
            assert_eq!(decode(&bytes), text);
        }
    }

    #[test]
    fn decodes_pdfdocencoding() {
        assert_eq!(decode(&[b'a', 0xE4, 0x84, 0xA0]), "aä—€");
    }
}
