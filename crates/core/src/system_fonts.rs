//! Installierte Schriften des Betriebssystems finden (Phase 11).
//!
//! Wofür: Fehlt einer eingebetteten Teilmengen-Schrift beim Bearbeiten ein Zeichen, nehmen wir
//! es aus der gleichnamigen Systemschrift (z. B. "ABCDEF+Arial-BoldMT" → Arial Bold). Außerdem
//! liefert die Liste die Schriftarten, die man im Editor auswählen kann.
//!
//! Gelesen werden nur das Tabellenverzeichnis sowie die Tabellen `name` und `OS/2` jeder Datei
//! (nicht die ganzen Schriften) – so bleibt das Durchsuchen auch bei Hunderten Schriften schnell.
//! Nur TrueType-Schriften (Tabelle `glyf`), deren Lizenz das Einbetten als Teilmenge erlaubt
//! (OS/2 fsType), kommen in die Liste.
//!
//! Dazu kommen die eingebauten Liberation-Schriften (Sans, Serif, Mono), falls sie nicht installiert
//! sind – im Browser, wo es keine Systemschriften gibt, sind sie die einzige Auswahl.

use crate::embedded_font::FontData;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Eine installierte Schrift (ein Schnitt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFont {
    pub family: String,
    /// Schnitt, z. B. "Bold Italic".
    pub style: String,
    pub postscript: String,
    pub full_name: String,
    pub bold: bool,
    pub italic: bool,
    pub path: PathBuf,
    /// Index in einer Schriftsammlung (.ttc).
    pub index: u32,
    /// Eingebaute Schrift: die Daten (dann ist `path` nur ein Name zur Anzeige).
    pub builtin: Option<&'static [u8]>,
}

/// Ordner, in denen Betriebssysteme Schriften ablegen.
fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    if cfg!(windows) {
        let windir =
            std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        dirs.push(windir.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        dirs.push("/System/Library/Fonts".into());
        dirs.push("/Library/Fonts".into());
        if let Some(h) = &home {
            dirs.push(h.join("Library/Fonts"));
        }
    } else {
        dirs.push("/usr/share/fonts".into());
        dirs.push("/usr/local/share/fonts".into());
        if let Some(h) = &home {
            dirs.push(h.join(".local/share/fonts"));
            dirs.push(h.join(".fonts"));
        }
    }
    dirs
}

/// Alle passenden Schriftdateien (rekursiv, höchstens 6 Ebenen tief).
fn font_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth < 6 {
                font_files(&path, depth + 1, out);
            }
        } else if path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "ttc"))
        {
            out.push(path);
        }
    }
}

fn read_at(f: &mut File, offset: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// Liest die Beschreibung einer Schrift (eine Datei kann mehrere enthalten).
fn describe(path: &Path) -> Vec<SystemFont> {
    let Ok(mut f) = File::open(path) else {
        return Vec::new();
    };
    let Some(head) = read_at(&mut f, 0, 12) else {
        return Vec::new();
    };
    // Schriftsammlung: Kopf "ttcf", danach die Anfänge der einzelnen Schriften.
    let offsets: Vec<u32> = if &head[0..4] == b"ttcf" {
        let n = be32(&head, 8).unwrap_or(0).min(64) as usize;
        read_at(&mut f, 12, 4 * n)
            .map(|b| (0..n).filter_map(|i| be32(&b, 4 * i)).collect())
            .unwrap_or_default()
    } else {
        vec![0]
    };
    offsets
        .iter()
        .enumerate()
        .filter_map(|(index, &off)| describe_one(&mut f, path, index as u32, u64::from(off)))
        .collect()
}

fn describe_one(f: &mut File, path: &Path, index: u32, off: u64) -> Option<SystemFont> {
    let header = read_at(f, off, 12)?;
    let num_tables = usize::from(be16(&header, 4)?).min(64);
    let records = read_at(f, off + 12, 16 * num_tables)?;
    let mut tables: HashMap<[u8; 4], (u32, u32)> = HashMap::new();
    for r in records.chunks_exact(16) {
        let tag: [u8; 4] = r[0..4].try_into().ok()?;
        tables.insert(tag, (be32(r, 8)?, be32(r, 12)?));
    }
    if !tables.contains_key(b"glyf") {
        return None; // keine TrueType-Konturen
    }
    let table = |f: &mut File, tag: &[u8; 4]| -> Option<Vec<u8>> {
        let (o, l) = *tables.get(tag)?;
        read_at(f, u64::from(o), (l as usize).min(1 << 20))
    };
    let name_data = table(f, b"name")?;
    let os2_data = table(f, b"OS/2");
    let names = ttf_parser::name::Table::parse(&name_data)?;
    let get = |id: u16| {
        names
            .names
            .into_iter()
            .filter(|n| n.name_id == id)
            .find_map(|n| n.to_string())
    };
    let (bold, italic, allowed) = match os2_data.as_deref().and_then(ttf_parser::os2::Table::parse)
    {
        Some(os2) => (
            os2.is_bold() || os2.weight().to_number() >= 600,
            !matches!(os2.style(), ttf_parser::Style::Normal),
            !matches!(os2.permissions(), Some(ttf_parser::Permissions::Restricted))
                && os2.is_subsetting_allowed()
                && os2.is_outline_embedding_allowed(),
        ),
        None => (false, false, true),
    };
    if !allowed {
        return None;
    }
    let family = get(ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
        .or_else(|| get(ttf_parser::name_id::FAMILY))?;
    // macOS: Namen mit Punkt (".SF NS", ".New York") sind interne Systemschriften, die
    // Apple nicht zur Verwendung in Dokumenten vorsieht.
    if family.starts_with('.') {
        return None;
    }
    let style = get(ttf_parser::name_id::TYPOGRAPHIC_SUBFAMILY)
        .or_else(|| get(ttf_parser::name_id::SUBFAMILY))
        .unwrap_or_else(|| "Regular".into());
    let postscript =
        get(ttf_parser::name_id::POST_SCRIPT_NAME).unwrap_or_else(|| family.replace(' ', ""));
    let full_name =
        get(ttf_parser::name_id::FULL_NAME).unwrap_or_else(|| format!("{family} {style}"));
    Some(SystemFont {
        family,
        style,
        postscript,
        full_name,
        bold,
        italic,
        path: path.to_path_buf(),
        index,
        builtin: None,
    })
}

/// Die eingebauten Liberation-Schriften als Einträge der Schriftliste.
fn builtin_fonts() -> Vec<SystemFont> {
    crate::embedded_font::liberation()
        .into_iter()
        .filter_map(|(postscript, data)| {
            let face = ttf_parser::Face::parse(data, 0).ok()?;
            let name = |id: u16| {
                face.names()
                    .into_iter()
                    .filter(|n| n.name_id == id)
                    .find_map(|n| n.to_string())
            };
            let family = name(ttf_parser::name_id::FAMILY)?;
            let style = name(ttf_parser::name_id::SUBFAMILY).unwrap_or_else(|| "Regular".into());
            Some(SystemFont {
                full_name: name(ttf_parser::name_id::FULL_NAME)
                    .unwrap_or_else(|| format!("{family} {style}")),
                family,
                style,
                postscript: postscript.to_string(),
                bold: face.is_bold(),
                italic: face.is_italic(),
                path: PathBuf::from(format!("(eingebaut) {postscript}")),
                index: 0,
                builtin: Some(data),
            })
        })
        .collect()
}

/// Alle installierten, einbettbaren TrueType-Schriften plus die eingebauten Liberation-Schriften.
/// Installierte werden einmal pro Programmlauf gesucht; die Liste wird neu gebildet, sobald
/// weitere eingebaute Schriften dazukommen (im Browser werden sie nachgeladen).
pub fn system_fonts() -> &'static [SystemFont] {
    static INSTALLED: OnceLock<Vec<SystemFont>> = OnceLock::new();
    /// (Anzahl eingebauter Schriften beim Bilden, fertige Liste).
    static MERGED: Mutex<(usize, &'static [SystemFont])> = Mutex::new((usize::MAX, &[]));
    let installed = INSTALLED.get_or_init(|| {
        let mut files = Vec::new();
        for d in font_dirs() {
            font_files(&d, 0, &mut files);
        }
        files.sort();
        files.dedup();
        files.iter().flat_map(|p| describe(p)).collect()
    });
    let builtin_count = crate::embedded_font::liberation().len();
    let Ok(mut merged) = MERGED.lock() else {
        return &[];
    };
    if merged.0 != builtin_count {
        let mut fonts = installed.clone();
        // Eingebaute hinten anhängen: Das Sortieren ist stabil und `dedup_by` behält den ersten
        // Eintrag – eine installierte Liberation-Schrift hat also Vorrang.
        fonts.extend(builtin_fonts());
        fonts.sort_by(|a, b| (&a.family, &a.style).cmp(&(&b.family, &b.style)));
        fonts.dedup_by(|a, b| a.postscript == b.postscript);
        tracing::debug!(count = fonts.len(), "Schriften gefunden");
        // `Box::leak`: Die Liste lebt bis Programmende, Aufrufer bekommen `&'static`. Neu gebildet
        // wird sie höchstens zweimal (Desktop einmal, Browser zusätzlich nach dem Nachladen).
        *merged = (builtin_count, Box::leak(fonts.into_boxed_slice()));
    }
    merged.1
}

/// Familiennamen (sortiert, ohne Doppelte) – für die Schriftauswahl.
pub fn families() -> Vec<&'static str> {
    let mut v: Vec<&str> = system_fonts().iter().map(|f| f.family.as_str()).collect();
    v.dedup();
    v
}

/// Vergleichsform eines Namens: Kleinbuchstaben, nur Buchstaben und Ziffern.
fn norm(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Zerlegt einen PDF-Schriftnamen ("ABCDEF+Arial-BoldItalicMT", "Arial,Bold") in
/// (Familie, fett, kursiv).
pub fn parse_base_font(base: &str) -> (String, bool, bool) {
    // Teilmengen-Präfix "ABCDEF+" weg.
    let b = base.as_bytes();
    let name = if b.len() > 7 && b[6] == b'+' && b[..6].iter().all(u8::is_ascii_uppercase) {
        &base[7..]
    } else {
        base
    };
    let lower = name.to_ascii_lowercase();
    let bold = ["bold", "black", "heavy", "semibold", "demi"]
        .iter()
        .any(|k| lower.contains(k));
    let italic = lower.contains("italic") || lower.contains("oblique");
    let family = name
        .split(['-', ','])
        .next()
        .unwrap_or(name)
        .trim_end_matches("PSMT")
        .trim_end_matches("MT")
        .trim_end_matches("PS")
        .to_string();
    (family, bold, italic)
}

/// Bester Schnitt einer Familie: exakt fett/kursiv, sonst der nächstbeste.
pub fn find_style(family: &str, bold: bool, italic: bool) -> Option<&'static SystemFont> {
    let fam = norm(family);
    system_fonts()
        .iter()
        .filter(|f| norm(&f.family) == fam || norm(&f.family).replace(' ', "") == fam)
        .min_by_key(|f| {
            // Abweichung: kursiv zählt mehr als fett; "Regular" vor exotischen Schnitten.
            let mut score = 0;
            if f.italic != italic {
                score += 4;
            }
            if f.bold != bold {
                score += 2;
            }
            let s = f.style.to_ascii_lowercase();
            if !matches!(
                s.as_str(),
                "regular" | "bold" | "italic" | "bold italic" | "book" | "roman"
            ) {
                score += 1;
            }
            score
        })
}

/// Systemschrift zu einem PDF-Schriftnamen: zuerst über den PostScript-Namen, dann über
/// Familie und Schnitt.
pub fn find_for_base_font(base: &str) -> Option<&'static SystemFont> {
    let (family, bold, italic) = parse_base_font(base);
    let ps = norm(base.rsplit('+').next().unwrap_or(base));
    system_fonts()
        .iter()
        .find(|f| norm(&f.postscript) == ps)
        .or_else(|| find_style(&family, bold, italic))
        .or_else(|| {
            // Nicht installiert: metrisch gleiche freie Entsprechung (z. B. unter Linux).
            let fam = norm(&family);
            ALIASES
                .iter()
                .filter(|(from, _)| *from == fam)
                .find_map(|(_, to)| find_style(to, bold, italic))
        })
}

/// Gleich breite Ersatzfamilien für verbreitete Schriften (normalisierter Name → Familie).
const ALIASES: &[(&str, &str)] = &[
    ("arial", "Liberation Sans"),
    ("arial", "Arimo"),
    ("helvetica", "Arial"),
    ("helvetica", "Liberation Sans"),
    ("timesnewroman", "Liberation Serif"),
    ("timesnewroman", "Tinos"),
    ("times", "Times New Roman"),
    ("times", "Liberation Serif"),
    ("couriernew", "Liberation Mono"),
    ("couriernew", "Cousine"),
    ("courier", "Courier New"),
    ("courier", "Liberation Mono"),
    ("calibri", "Carlito"),
    ("cambria", "Caladea"),
];

/// Lädt eine Systemschrift (zwischengespeichert – jede Datei wird höchstens einmal gelesen).
pub fn load(font: &SystemFont) -> Option<Arc<FontData>> {
    /// Datei und Index → geladene Schrift (`None` = nicht ladbar, nicht erneut versuchen).
    type Cache = Mutex<HashMap<(PathBuf, u32), Option<Arc<FontData>>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = (font.path.clone(), font.index);
    if let Some(hit) = cache.lock().ok()?.get(&key) {
        return hit.clone();
    }
    let loaded = match font.builtin {
        Some(data) => FontData::from_static(data),
        None => std::fs::read(&font.path)
            .ok()
            .and_then(|b| FontData::from_bytes(b, font.index)),
    };
    if let Ok(mut c) = cache.lock() {
        c.insert(key, loaded.clone());
    }
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pdf_font_names() {
        assert_eq!(
            parse_base_font("ABCDEF+Arial-BoldItalicMT"),
            ("Arial".into(), true, true)
        );
        assert_eq!(parse_base_font("ArialMT"), ("Arial".into(), false, false));
        assert_eq!(
            parse_base_font("TimesNewRomanPS-BoldMT"),
            ("TimesNewRoman".into(), true, false)
        );
        assert_eq!(
            parse_base_font("Calibri,Italic"),
            ("Calibri".into(), false, true)
        );
    }

    /// Die eingebauten Liberation-Schriften gibt es immer (auch ohne installierte Schriften, z. B.
    /// im Browser): alle drei Familien mit vier Schnitten, ladbar, mit Umlauten.
    #[test]
    fn builtin_fonts_are_always_there() {
        let builtin = builtin_fonts();
        assert_eq!(builtin.len(), 12);
        for family in ["Liberation Sans", "Liberation Serif", "Liberation Mono"] {
            for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
                let f = find_style(family, bold, italic).expect(family);
                assert_eq!((f.bold, f.italic), (bold, italic), "{f:?}");
                let data = load(f).expect("ladbar");
                assert!(data.has_glyph('Ä') && data.has_glyph('ß'));
            }
        }
        assert!(families().contains(&"Liberation Serif"));
        // Arial ohne installierte Arial → metrisch gleiche Liberation Sans
        let arial = find_for_base_font("ABCDEF+Arial-BoldMT").expect("Ersatz für Arial");
        assert!(arial.bold);
    }

    /// Läuft überall; findet nur dort etwas, wo Schriften installiert sind (Container: DejaVu,
    /// Liberation …).
    #[test]
    fn finds_installed_fonts() {
        let fonts = system_fonts();
        if fonts.is_empty() {
            return;
        }
        // Jede aufgeführte Schrift muss ladbar sein (höchstens 40 prüfen, das Lesen dauert).
        // Symbolschriften (z. B. ohne „A“) sind erlaubt, aber mindestens eine Schrift hat Latein.
        let mut latin = false;
        for font in fonts.iter().take(40) {
            let data = load(font).unwrap_or_else(|| {
                panic!(
                    "aufgeführte Schrift nicht ladbar: {} ({}, Index {})",
                    font.postscript,
                    font.path.display(),
                    font.index
                )
            });
            latin |= data.has_glyph('A') && data.has_glyph('a');
        }
        assert!(
            latin,
            "keine der ersten Schriften enthält lateinische Buchstaben"
        );
        if let Some(f) = find_style("Liberation Sans", true, false) {
            assert!(f.bold && !f.italic, "{f:?}");
            assert_eq!(
                find_for_base_font("XYZABC+LiberationSans-Bold").map(|f| &f.postscript),
                Some(&f.postscript)
            );
        }
    }
}
