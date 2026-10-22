//! Texterkennung (OCR) mit Tesseract.
//!
//! Tesseract (Apache-2.0) und Leptonica (BSD) werden über `tesseract-rs` aus den Quellen
//! gebaut und fest eingebunden (dafür braucht der Build CMake und einen C++-Compiler).
//! Die Sprachdaten für Deutsch und Englisch (`tessdata_fast`, Apache-2.0) stecken im
//! Programm; Tesseract liest sie aber aus einem Verzeichnis, deshalb schreibt
//! [`tessdata_dir`] sie beim ersten Gebrauch in einen Cache-Ordner.
//!
//! Ergebnis einer Erkennung sind Wörter mit Pixel-Rechteck (Ursprung oben links) – die
//! Umrechnung in PDF-Koordinaten macht der Aufrufer, der weiß, wie die Seite gerendert wurde.
//!
//! Die Crate kennt weder PDF noch UI.

use std::path::{Path, PathBuf};
use tesseract_rs::{TessPageSegMode, TesseractAPI};

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("Texterkennung: {0}")]
    Tesseract(String),
    #[error("Sprachdaten konnten nicht abgelegt werden ({path}): {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, OcrError>;

fn tess<E: std::fmt::Display>(e: E) -> OcrError {
    OcrError::Tesseract(e.to_string())
}

/// Eingebaute Sprachen: (Tesseract-Kürzel, Anzeigename, Daten).
pub const LANGUAGES: &[(&str, &str, &[u8])] = &[
    (
        "deu",
        "Deutsch",
        include_bytes!("../../../assets/tessdata/deu.traineddata"),
    ),
    (
        "eng",
        "Englisch",
        include_bytes!("../../../assets/tessdata/eng.traineddata"),
    ),
];

/// Verzeichnis mit den Sprachdaten; legt sie beim ersten Aufruf im Cache-Ordner ab
/// (je Programmversion, damit neue Daten alte nie mischen).
pub fn tessdata_dir() -> Result<PathBuf> {
    let dir =
        std::env::temp_dir().join(format!("foliocraft-tessdata-{}", env!("CARGO_PKG_VERSION")));
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| OcrError::Io { path, source }
    };
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    for (code, _, data) in LANGUAGES {
        let file = dir.join(format!("{code}.traineddata"));
        let complete = std::fs::metadata(&file).is_ok_and(|m| m.len() == data.len() as u64);
        if !complete {
            // Erst unter anderem Namen schreiben, dann umbenennen: Ein zweiter Prozess sieht
            // nie eine halbe Datei.
            let tmp = dir.join(format!("{code}.{}.tmp", std::process::id()));
            std::fs::write(&tmp, data).map_err(io(&tmp))?;
            std::fs::rename(&tmp, &file).map_err(io(&file))?;
        }
    }
    Ok(dir)
}

/// Ein erkanntes Wort. Rechteck in Pixeln des übergebenen Bilds (Ursprung oben links).
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
    /// Sicherheit 0–100.
    pub confidence: f32,
    /// Zeile, zu der das Wort gehört (Block, Absatz, Zeile) – für die Lesereihenfolge.
    pub line: (u32, u32, u32),
}

/// Ein Graustufen- oder Farbbild im Speicher.
pub struct Image<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// 1 (Grau), 3 (RGB) oder 4 (RGBA).
    pub bytes_per_pixel: u32,
    /// Auflösung (für Tesseracts Größenschätzungen), z. B. 300.
    pub dpi: u32,
}

/// Eine initialisierte Tesseract-Instanz (pro Thread eine; das Laden dauert etwas).
pub struct Engine {
    api: TesseractAPI,
}

impl Engine {
    /// `languages` z. B. "deu+eng".
    pub fn new(languages: &str) -> Result<Self> {
        let dir = tessdata_dir()?;
        let api = TesseractAPI::new();
        api.init(&dir, languages).map_err(tess)?;
        // Automatische Seitenaufteilung (Spalten, Blöcke), ohne Ausrichtungserkennung.
        api.set_page_seg_mode(TessPageSegMode::PSM_AUTO)
            .map_err(tess)?;
        Ok(Self { api })
    }

    /// Erkennt die Wörter eines Bilds.
    pub fn recognize(&self, image: &Image) -> Result<Vec<Word>> {
        let (w, h, bpp) = (
            image.width as i32,
            image.height as i32,
            image.bytes_per_pixel as i32,
        );
        if image.data.len() < (w * h * bpp) as usize {
            return Err(OcrError::Tesseract("Bilddaten zu kurz".into()));
        }
        self.api
            .set_image(image.data, w, h, bpp, w * bpp)
            .map_err(tess)?;
        self.api
            .set_source_resolution(image.dpi as i32)
            .map_err(tess)?;
        self.api.recognize().map_err(tess)?;
        let tsv = self.api.get_tsv_text(0).map_err(tess)?;
        Ok(parse_tsv(&tsv))
    }
}

/// Liest Tesseracts TSV-Ausgabe: level, page, block, par, line, word, left, top, width,
/// height, conf, text. Wörter haben level 5.
fn parse_tsv(tsv: &str) -> Vec<Word> {
    tsv.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(12, '\t').collect();
            if f.len() < 12 || f[0] != "5" {
                return None;
            }
            let n = |i: usize| f[i].trim().parse::<u32>().ok();
            let text = f[11].trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(Word {
                text,
                left: n(6)?,
                top: n(7)?,
                width: n(8)?,
                height: n(9)?,
                confidence: f[10].trim().parse().unwrap_or(0.0),
                line: (n(2)?, n(3)?, n(4)?),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_words() {
        let tsv = "1\t1\t0\t0\t0\t0\t0\t0\t100\t50\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t10\t20\t30\t12\t96.5\tHallo\n\
                   5\t1\t1\t1\t1\t2\t45\t20\t28\t12\t91\tWelt\n\
                   5\t1\t1\t1\t2\t1\t10\t40\t5\t12\t30\t \n";
        let words = parse_tsv(tsv);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Hallo");
        assert_eq!((words[1].left, words[1].width), (45, 28));
        assert_eq!(words[1].line, (1, 1, 1));
    }

    #[test]
    fn languages_are_written_once() {
        let dir = tessdata_dir().unwrap();
        for (code, _, data) in LANGUAGES {
            let len = std::fs::metadata(dir.join(format!("{code}.traineddata")))
                .unwrap()
                .len();
            assert_eq!(len, data.len() as u64);
        }
    }
}
