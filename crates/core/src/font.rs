//! Helvetica (Standard-14-Schrift): Zeichenbreiten, WinAnsi-Kodierung, Zeilenumbruch.
//!
//! Die Breiten sind die Standardwerte der Schrift in 1/1000 der Schriftgröße.
//! Sie werden nur gebraucht, um Freitext umzubrechen – gezeichnet wird mit der
//! Helvetica des jeweiligen PDF-Readers.

/// Breiten für ASCII 32..=126.
const ASCII_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278,
    278, // ' '..'/'
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // '0'..'9'
    278, 278, 584, 584, 584, 556, 1015, // ':'..'@'
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, // 'A'..'M'
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, // 'N'..'Z'
    278, 278, 278, 469, 556, 333, // '['..'`'
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, // 'a'..'m'
    556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, // 'n'..'z'
    334, 260, 334, 584, // '{'..'~'
];

/// Breiten für Latin-1 0xA0..=0xFF.
const LATIN1_WIDTHS: [u16; 96] = [
    278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333, // A0..AF
    400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611, // B0..BF
    667, 667, 667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278, // C0..CF
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611, // D0..DF
    556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278, // E0..EF
    556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500, // F0..FF
];

/// Sonderzeichen im WinAnsi-Bereich 0x80..=0x9F: (Zeichen, Code, Breite).
const WINANSI_SPECIAL: [(char, u8, u16); 27] = [
    ('€', 0x80, 556),
    ('‚', 0x82, 222),
    ('ƒ', 0x83, 556),
    ('„', 0x84, 333),
    ('…', 0x85, 1000),
    ('†', 0x86, 556),
    ('‡', 0x87, 556),
    ('ˆ', 0x88, 333),
    ('‰', 0x89, 1000),
    ('Š', 0x8A, 667),
    ('‹', 0x8B, 333),
    ('Œ', 0x8C, 1000),
    ('Ž', 0x8E, 611),
    ('‘', 0x91, 222),
    ('’', 0x92, 222),
    ('“', 0x93, 333),
    ('”', 0x94, 333),
    ('•', 0x95, 350),
    ('–', 0x96, 556),
    ('—', 0x97, 1000),
    ('˜', 0x98, 333),
    ('™', 0x99, 1000),
    ('š', 0x9A, 500),
    ('›', 0x9B, 333),
    ('œ', 0x9C, 944),
    ('ž', 0x9E, 500),
    ('Ÿ', 0x9F, 667),
];

/// WinAnsi-Code eines Zeichens; `None`, wenn Helvetica/WinAnsi es nicht darstellen kann.
pub fn winansi_code(c: char) -> Option<u8> {
    match c as u32 {
        32..=126 | 0xA0..=0xFF => Some(c as u32 as u8),
        _ => WINANSI_SPECIAL.iter().find(|s| s.0 == c).map(|s| s.1),
    }
}

/// Text -> WinAnsi-Bytes; nicht darstellbare Zeichen werden zu '?'.
pub fn winansi_encode(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| winansi_code(c).unwrap_or(b'?'))
        .collect()
}

/// Breite eines Zeichens in Punkt bei gegebener Schriftgröße.
pub fn char_width(c: char, font_size: f32) -> f32 {
    let w = match winansi_code(c) {
        Some(code @ 32..=126) => ASCII_WIDTHS[(code - 32) as usize],
        Some(code @ 0xA0..=0xFF) => LATIN1_WIDTHS[(code - 0xA0) as usize],
        Some(code) => WINANSI_SPECIAL
            .iter()
            .find(|s| s.1 == code)
            .map_or(556, |s| s.2),
        None => 556, // '?'
    };
    w as f32 * font_size / 1000.0
}

pub fn text_width(text: &str, font_size: f32) -> f32 {
    text.chars().map(|c| char_width(c, font_size)).sum()
}

/// Bricht Text in Zeilen um, die höchstens `max_width` breit sind. Harte Umbrüche (`\n`)
/// bleiben erhalten; zu lange Wörter werden notfalls mitten im Wort getrennt.
pub fn wrap(text: &str, font_size: f32, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if text_width(&candidate, font_size) <= max_width || line.is_empty() && word.is_empty()
            {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // Wort passt allein nicht in die Zeile -> zeichenweise aufteilen.
            for c in word.chars() {
                if !line.is_empty()
                    && text_width(&line, font_size) + char_width(c, font_size) > max_width
                {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_and_encoding() {
        assert_eq!(char_width('A', 1000.0), 667.0);
        assert_eq!(char_width('ä', 1000.0), 556.0);
        assert_eq!(
            winansi_encode("Grüße €"),
            vec![b'G', b'r', 0xFC, 0xDF, b'e', b' ', 0x80]
        );
        assert_eq!(winansi_encode("日"), b"?".to_vec());
    }

    #[test]
    fn wraps_words() {
        // "aaaa" = 4 * 556 * 10/1000 = 22.24 pt
        let lines = wrap("aaaa aaaa aaaa", 10.0, 50.0);
        assert_eq!(lines, vec!["aaaa aaaa", "aaaa"]);
        assert_eq!(wrap("a\n\nb", 10.0, 50.0), vec!["a", "", "b"]);
        // Zu langes Wort wird getrennt
        assert_eq!(wrap("aaaaaaaaaa", 10.0, 23.0).len(), 3);
    }
}
