//! Mitgelieferte Unicode-Schrift (Noto Sans, SIL Open Font License – siehe
//! `assets/fonts/OFL.txt`) und die Einbettung von TrueType-Schriften als Teilmenge.
//!
//! Wofür: Freitext mit beliebigen Zeichen (Kyrillisch, Griechisch, Sonderzeichen …) und
//! Ersatzschrift beim Bearbeiten von Text, wenn der Originalschrift Zeichen fehlen. Seit
//! Phase 11 kann statt Noto Sans auch eine Systemschrift eingebettet werden ([`FontData`],
//! gefunden über `system_fonts`).
//!
//! In die PDF kommt nur eine **Teilmenge** (nur die benutzten Glyphen, meist wenige KB) als
//! Type0-Schrift: 2 Byte pro Zeichen (Identity-H), Breiten in /W, und eine /ToUnicode-CMap,
//! damit Kopieren und Suchen den richtigen Text liefern.

use crate::error::{CoreError, Result};
use lopdf::{Object, ObjectId, Stream, StringFormat, dictionary};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};
use subsetter::GlyphRemapper;

/// Die Schriftdatei, fest in das Programm eingebaut.
pub static NOTO_SANS: &[u8] = include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf");
/// PostScript-Name (ohne Teilmengen-Präfix).
const POSTSCRIPT_NAME: &str = "NotoSans-Regular";

/// Die geparste Schrift – einmal pro Programmlauf. `OnceLock` = threadsicher, lazy.
fn face() -> &'static ttf_parser::Face<'static> {
    static FACE: OnceLock<ttf_parser::Face<'static>> = OnceLock::new();
    FACE.get_or_init(|| ttf_parser::Face::parse(NOTO_SANS, 0).expect("eingebaute Schrift gültig"))
}

/// Einheiten pro em (für Umrechnungen in 1/1000 em, wie PDF sie erwartet).
fn per_mille(v: f32) -> f32 {
    v * 1000.0 / f32::from(face().units_per_em())
}

/// Hat die Schrift eine Glyphe für dieses Zeichen?
pub fn has_glyph(c: char) -> bool {
    face().glyph_index(c).is_some()
}

fn glyph(c: char) -> ttf_parser::GlyphId {
    face().glyph_index(c).unwrap_or(ttf_parser::GlyphId(0))
}

/// Breite eines Zeichens in Punkt bei gegebener Schriftgröße.
pub fn char_width(c: char, size: f32) -> f32 {
    let adv = face().glyph_hor_advance(glyph(c)).unwrap_or(0);
    per_mille(f32::from(adv)) * size / 1000.0
}

pub fn text_width(text: &str, size: f32) -> f32 {
    text.chars().map(|c| char_width(c, size)).sum()
}

/// Zeilenumbruch mit den Breiten dieser Schrift (gleiche Regeln wie `font::wrap`).
pub fn wrap(text: &str, size: f32, max_width: f32) -> Vec<String> {
    crate::font::wrap_with(text, size, max_width, &char_width)
}

/// Ober- und Unterlänge in 1/1000 em.
pub fn ascent_descent() -> (f32, f32) {
    let f = face();
    (
        per_mille(f32::from(f.ascender())),
        per_mille(f32::from(f.descender())),
    )
}

/// Eine einbettbare TrueType-Schrift (Noto Sans oder eine Systemschrift).
pub struct FontData {
    bytes: FontBytes,
    /// Index in einer Schriftsammlung (.ttc), sonst 0.
    pub index: u32,
    /// PostScript-Name (z. B. "Arial-BoldMT") – wird zum /BaseFont.
    pub postscript: String,
    /// Familienname (z. B. "Arial").
    pub family: String,
    pub bold: bool,
    pub italic: bool,
}

enum FontBytes {
    Static(&'static [u8]),
    Owned(Vec<u8>),
}

impl std::fmt::Debug for FontData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FontData({})", self.postscript)
    }
}

/// Gleich = gleiche Schrift (PostScript-Name und Index in der Sammlung).
impl PartialEq for FontData {
    fn eq(&self, other: &Self) -> bool {
        self.postscript == other.postscript && self.index == other.index
    }
}

impl FontData {
    /// Die eingebaute Noto Sans (geteilt, nur einmal angelegt).
    pub fn noto() -> Arc<FontData> {
        static NOTO: OnceLock<Arc<FontData>> = OnceLock::new();
        NOTO.get_or_init(|| {
            Arc::new(FontData {
                bytes: FontBytes::Static(NOTO_SANS),
                index: 0,
                postscript: POSTSCRIPT_NAME.into(),
                family: "Noto Sans".into(),
                bold: false,
                italic: false,
            })
        })
        .clone()
    }

    /// Schrift aus Dateibytes. `None`, wenn sie keine TrueType-Konturen hat (`glyf`) oder ihre
    /// Lizenz das Einbetten als Teilmenge nicht erlaubt (OS/2 fsType).
    pub fn from_bytes(bytes: Vec<u8>, index: u32) -> Option<Arc<FontData>> {
        let face = ttf_parser::Face::parse(&bytes, index).ok()?;
        if !embeddable(&face) {
            return None;
        }
        let name = |id: u16| {
            face.names()
                .into_iter()
                .filter(|n| n.name_id == id)
                .find_map(|n| n.to_string())
        };
        // Wie bei der Suche (`system_fonts::describe_one`): fehlt der PostScript-Name (z. B. nur
        // Namen in Mac-Kodierung), wird er aus der Familie gebildet.
        let family = name(ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
            .or_else(|| name(ttf_parser::name_id::FAMILY))?;
        let postscript =
            name(ttf_parser::name_id::POST_SCRIPT_NAME).unwrap_or_else(|| family.replace(' ', ""));
        let (bold, italic) = (face.is_bold(), face.is_italic() || face.is_oblique());
        Some(Arc::new(FontData {
            bytes: FontBytes::Owned(bytes),
            index,
            postscript,
            family,
            bold,
            italic,
        }))
    }

    pub fn bytes(&self) -> &[u8] {
        match &self.bytes {
            FontBytes::Static(b) => b,
            FontBytes::Owned(b) => b,
        }
    }

    pub fn is_noto(&self) -> bool {
        matches!(self.bytes, FontBytes::Static(_))
    }

    /// Geparste Schrift. Das Parsen ist billig (nur Tabellenverzeichnis), deshalb pro Aufruf.
    pub fn face(&self) -> ttf_parser::Face<'_> {
        ttf_parser::Face::parse(self.bytes(), self.index).expect("beim Laden geprüft")
    }

    pub fn has_glyph(&self, c: char) -> bool {
        self.face().glyph_index(c).is_some()
    }

    /// Breiten mehrerer Zeichen in 1/1000 em (ein Parse für alle).
    pub fn widths_per_mille(&self, chars: &[char]) -> Vec<f32> {
        let face = self.face();
        let upm = f32::from(face.units_per_em());
        chars
            .iter()
            .map(|&c| {
                let g = face.glyph_index(c).unwrap_or(ttf_parser::GlyphId(0));
                f32::from(face.glyph_hor_advance(g).unwrap_or(0)) * 1000.0 / upm
            })
            .collect()
    }

    pub fn char_width(&self, c: char, size: f32) -> f32 {
        self.widths_per_mille(&[c])[0] * size / 1000.0
    }

    /// Ober- und Unterlänge in 1/1000 em.
    pub fn ascent_descent(&self) -> (f32, f32) {
        let f = self.face();
        let upm = f32::from(f.units_per_em());
        (
            f32::from(f.ascender()) * 1000.0 / upm,
            f32::from(f.descender()) * 1000.0 / upm,
        )
    }
}

/// Darf die Schrift als Teilmenge eingebettet werden? Braucht TrueType-Konturen und eine
/// Lizenz ohne "Restricted"/"nur Bitmaps"/"keine Teilmengen".
pub fn embeddable(face: &ttf_parser::Face) -> bool {
    let tables = face.tables();
    if tables.glyf.is_none() {
        return false; // CFF-Schriften (OpenType/PostScript) betten wir nicht ein
    }
    match tables.os2 {
        Some(os2) => {
            !matches!(os2.permissions(), Some(ttf_parser::Permissions::Restricted))
                && os2.is_subsetting_allowed()
                && os2.is_outline_embedding_allowed()
        }
        None => true,
    }
}

/// Teilmenge der Schrift, die nach und nach mit Text gefüllt wird.
#[derive(Clone)]
pub struct SubsetFont {
    /// Welche Schrift (Noto Sans oder eine Systemschrift). `Arc`: geteilt, ohne Kopie.
    font: Arc<FontData>,
    remapper: GlyphRemapper,
    /// Neue Glyphennummer -> Text (für ToUnicode) und Breite (1/1000 em).
    glyphs: BTreeMap<u16, (String, f32)>,
}

impl std::fmt::Debug for SubsetFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SubsetFont({}, {} Glyphen)",
            self.font.postscript,
            self.glyphs.len()
        )
    }
}

impl Default for SubsetFont {
    fn default() -> Self {
        Self::new()
    }
}

impl SubsetFont {
    /// Leere Teilmenge der eingebauten Noto Sans.
    pub fn new() -> Self {
        Self::with_font(FontData::noto())
    }

    /// Leere Teilmenge einer beliebigen (einbettbaren) Schrift.
    pub fn with_font(font: Arc<FontData>) -> Self {
        Self {
            font,
            remapper: GlyphRemapper::new(),
            glyphs: BTreeMap::new(),
        }
    }

    /// Die Schrift dieser Teilmenge.
    pub fn font(&self) -> &Arc<FontData> {
        &self.font
    }

    /// Kodiert Text für diese Teilmenge: 2 Byte (neue Glyphennummer) pro Zeichen.
    /// Zeichen ohne Glyphe werden zur leeren Glyphe 0 (.notdef).
    pub fn encode(&mut self, text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() * 2);
        // `Arc::clone`: die Schrift ausleihen, während `self` verändert wird.
        let font = self.font.clone();
        let face = font.face();
        let upm = f32::from(face.units_per_em());
        for c in text.chars() {
            let old = face.glyph_index(c).unwrap_or(ttf_parser::GlyphId(0));
            let new = self.remapper.remap(old.0);
            let width = f32::from(face.glyph_hor_advance(old).unwrap_or(0)) * 1000.0 / upm;
            self.glyphs
                .entry(new)
                .or_insert_with(|| (c.to_string(), width));
            out.extend_from_slice(&new.to_be_bytes());
        }
        out
    }

    /// Breiten (1/1000 em) der bisher benutzten Glyphen, nach neuer Glyphennummer.
    pub fn widths(&self) -> impl Iterator<Item = (u16, f32)> + '_ {
        self.glyphs.iter().map(|(&g, (_, w))| (g, *w))
    }

    /// Text der bisher benutzten Glyphen (wie in der ToUnicode-CMap).
    pub fn texts(&self) -> impl Iterator<Item = (u16, &str)> + '_ {
        self.glyphs.iter().map(|(&g, (t, _))| (g, t.as_str()))
    }

    /// Hex-String-Objekt für einen `Tj`-Operanden.
    pub fn encode_object(&mut self, text: &str) -> Object {
        Object::String(self.encode(text), StringFormat::Hexadecimal)
    }

    /// Schreibt die Schrift als PDF-Objekte. `add` legt ein Objekt an und liefert seine
    /// Nummer – so funktioniert das mit jedem Ziel (Dokument, inkrementelles Update …).
    /// Liefert die Nummer des Type0-Schrift-Dictionarys.
    pub fn write(&self, add: &mut dyn FnMut(Object) -> ObjectId) -> Result<ObjectId> {
        let font = &self.font;
        let data = subsetter::subset(font.bytes(), font.index, &self.remapper)
            .map_err(|e| CoreError::Other(format!("Schrift-Teilmenge: {e:?}")))?;
        let mut file = Stream::new(dictionary! { "Length1" => data.len() as i64 }, data);
        let _ = file.compress();
        let file_id = add(Object::Stream(file));

        // Teilmengen-Präfix: sechs Großbuchstaben, abgeleitet aus den benutzten Glyphen.
        let tag: String = {
            let mut h: u32 = 2_166_136_261;
            for gid in self.remapper.remapped_gids() {
                h = (h ^ u32::from(gid)).wrapping_mul(16_777_619);
            }
            (0..6)
                .map(|i| (b'A' + ((h >> (i * 5)) % 26) as u8) as char)
                .collect()
        };
        let name = format!("{tag}+{}", font.postscript.replace(' ', ""));
        let f = font.face();
        let upm = f32::from(f.units_per_em());
        let per_mille = |v: i16| f32::from(v) * 1000.0 / upm;
        let bbox = f.global_bounding_box();
        let (ascent, descent) = font.ascent_descent();
        let cap = f.capital_height().map_or(ascent * 0.7, per_mille);
        // Flags: nicht-symbolisch (32), kursiv (64), Fett erzwingen (1 << 18).
        let flags = 32 | if font.italic { 64 } else { 0 } | if font.bold { 1 << 18 } else { 0 };
        let italic_angle = f.italic_angle();
        let descriptor = add(Object::Dictionary(dictionary! {
            "Type" => "FontDescriptor",
            "FontName" => Object::Name(name.clone().into_bytes()),
            "Flags" => flags,
            "FontBBox" => vec![
                per_mille(bbox.x_min).into(), per_mille(bbox.y_min).into(),
                per_mille(bbox.x_max).into(), per_mille(bbox.y_max).into(),
            ],
            "ItalicAngle" => italic_angle,
            "Ascent" => ascent,
            "Descent" => descent,
            "CapHeight" => cap,
            "StemV" => if font.bold { 140 } else { 80 },
            "FontFile2" => file_id,
        }));

        // /W: [gid [w]] je Glyphe.
        let mut w = Vec::new();
        for (&gid, (_, width)) in &self.glyphs {
            w.push(Object::from(i64::from(gid)));
            w.push(Object::Array(vec![Object::Real(*width)]));
        }
        let cid_font = add(Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "CIDFontType2",
            "BaseFont" => Object::Name(name.clone().into_bytes()),
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"),
                "Ordering" => Object::string_literal("Identity"),
                "Supplement" => 0,
            },
            "FontDescriptor" => descriptor,
            "W" => w,
            "CIDToGIDMap" => "Identity",
        }));

        let mut cmap = Stream::new(dictionary! {}, self.to_unicode_cmap().into_bytes());
        let _ = cmap.compress();
        let to_unicode = add(Object::Stream(cmap));
        Ok(add(Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => Object::Name(name.into_bytes()),
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![Object::from(cid_font)],
            "ToUnicode" => to_unicode,
        })))
    }

    /// ToUnicode-CMap: Glyphennummer -> Unicode (UTF-16BE).
    fn to_unicode_cmap(&self) -> String {
        let mut s = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
             1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        // Höchstens 100 Einträge pro Block (Vorgabe des CMap-Formats).
        let entries: Vec<_> = self.glyphs.iter().filter(|(gid, _)| **gid != 0).collect();
        for chunk in entries.chunks(100) {
            let _ = writeln!(s, "{} beginbfchar", chunk.len());
            for (gid, (text, _)) in chunk {
                let utf16: String = text.encode_utf16().map(|u| format!("{u:04X}")).collect();
                let _ = writeln!(s, "<{gid:04X}> <{utf16}>");
            }
            s.push_str("endbfchar\n");
        }
        s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_and_glyphs() {
        assert!(has_glyph('ж') && has_glyph('Ω') && has_glyph('ä'));
        assert!(!has_glyph('日'), "CJK ist nicht in Noto Sans");
        assert!(char_width('W', 10.0) > char_width('i', 10.0));
        assert!(
            (text_width("ab", 12.0) - char_width('a', 12.0) - char_width('b', 12.0)).abs() < 1e-4
        );
    }

    #[test]
    fn subset_writes_type0_font() {
        let mut f = SubsetFont::new();
        let bytes = f.encode("Привет Ω");
        assert_eq!(bytes.len(), 16, "2 Byte pro Zeichen");
        let mut objs = Vec::new();
        let id = f
            .write(&mut |o| {
                objs.push(o);
                (objs.len() as u32, 0)
            })
            .unwrap();
        let font = objs[id.0 as usize - 1].as_dict().unwrap();
        assert_eq!(font.get(b"Subtype").unwrap().as_name().unwrap(), b"Type0");
        // Die Teilmenge ist viel kleiner als die ganze Schrift.
        let file = objs[0].as_stream().unwrap();
        assert!(
            file.content.len() < NOTO_SANS.len() / 10,
            "{}",
            file.content.len()
        );
        assert!(
            f.to_unicode_cmap().contains("041F"),
            "П in der ToUnicode-CMap"
        );
    }
}
