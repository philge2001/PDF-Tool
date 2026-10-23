//! Content-Stream-Interpreter: spielt die Zeichenbefehle einer Seite durch und liefert
//! Text-, Pfad- und Bildobjekte mit ihrer Position auf der Seite.
//!
//! Wie ein Renderer, nur ohne zu zeichnen. Dafür wird der Grafikzustand mitgeführt:
//! - CTM (aktuelle Transformationsmatrix, `cm`), gesichert/wiederhergestellt mit `q`/`Q`,
//! - Textzustand (Schrift `Tf`, Abstände `Tc`/`Tw`, Breite `Tz`, Zeilenabstand `TL`,
//!   Hochstellung `Ts`, Darstellungsart `Tr`),
//! - Textmatrix und Zeilenmatrix (`BT`, `Td`, `TD`, `Tm`, `T*`).
//!
//! Für jedes gezeigte Zeichen ergibt sich so ein Rechteck im PDF-Benutzerraum. Die Breiten
//! kommen aus der Schrift (`/Widths`, `/W`, Standardschriften). Sind sie nicht sicher bekannt,
//! steht `exact = false` daran – wer Inhalte entfernt (Schwärzen), muss dann vorsichtiger sein.
//!
//! Die Ergebnisse verweisen per Index auf die Operationen (`ops`), damit Bearbeiter
//! (Schwärzen, später Text bearbeiten) genau diese Operationen ändern können.

use crate::document::inherited;
use crate::error::Result;
use crate::geometry::{Point, Rect};
use lopdf::Encoding;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document as PdfDoc, Object, ObjectId};
use std::collections::HashMap;
use std::rc::Rc;

/// Affine Abbildung [a b c d e f]: x' = a·x + c·y + e, y' = b·x + d·y + f (wie in PDF).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix(pub [f32; 6]);

impl Matrix {
    pub const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(x: f32, y: f32) -> Self {
        Matrix([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// `self` zuerst, dann `other` (PDF-Schreibweise: self × other).
    pub fn then(&self, other: &Matrix) -> Matrix {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Matrix([
            a * a2 + b * c2,
            a * b2 + b * d2,
            c * a2 + d * c2,
            c * b2 + d * d2,
            e * a2 + f * c2 + e2,
            e * b2 + f * d2 + f2,
        ])
    }

    /// Umkehrabbildung (`None`, wenn die Matrix nicht umkehrbar ist, z. B. Breite 0).
    pub fn invert(&self) -> Option<Matrix> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-9 {
            return None;
        }
        Some(Matrix([
            d / det,
            -b / det,
            -c / det,
            a / det,
            (c * f - d * e) / det,
            (b * e - a * f) / det,
        ]))
    }

    pub fn apply(&self, p: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }

    /// Achsparalleles Rechteck um das abgebildete Rechteck `r`.
    pub fn apply_rect(&self, r: Rect) -> Rect {
        let pts = [
            Point::new(r.x0, r.y0),
            Point::new(r.x1, r.y0),
            Point::new(r.x0, r.y1),
            Point::new(r.x1, r.y1),
        ]
        .map(|p| self.apply(p));
        Rect::bounding(pts).unwrap_or(r)
    }

    pub fn from_operands(ops: &[Object]) -> Option<Matrix> {
        let v: Vec<f32> = ops.iter().filter_map(|o| o.as_float().ok()).collect();
        (v.len() == 6).then(|| Matrix([v[0], v[1], v[2], v[3], v[4], v[5]]))
    }

    pub fn from_object(o: &Object) -> Option<Matrix> {
        Self::from_operands(o.as_array().ok()?)
    }
}

// ---------- Schriften ----------

/// Was wir zur Geometrie einer Schrift wissen.
pub struct FontInfo<'a> {
    /// Zwei Byte pro Zeichencode (Type0 mit Identity-CMap & Co.), sonst eines.
    two_byte: bool,
    widths: Widths,
    /// Breiten sicher bekannt?
    pub exact: bool,
    /// Ober- und Unterlänge relativ zur Schriftgröße (z. B. 0.8 / -0.2).
    ascent: f32,
    descent: f32,
    /// Für Type3: Glyphenraum -> Textraum (sonst 1/1000).
    scale: f32,
    /// Zum Übersetzen in Unicode (aus lopdf, inkl. ToUnicode).
    encoding: Option<Encoding<'a>>,
    /// /BaseFont, z. B. "ABCDEF+Arial-BoldMT".
    pub base_font: String,
    /// Schriftdatei eingebettet (/FontFile, /FontFile2, /FontFile3)?
    pub embedded: bool,
    /// Eigene Zuordnung Code -> Text (für Schriften, die erst im Modell existieren).
    unicode: Option<HashMap<u32, String>>,
}

enum Widths {
    /// Einfache Schrift: /FirstChar + /Widths, /MissingWidth.
    Simple {
        first: u32,
        widths: Vec<f32>,
        missing: f32,
    },
    /// CID-Schrift: /DW und /W (CID -> Breite).
    Cid {
        default: f32,
        map: HashMap<u32, f32>,
    },
    /// Standardschrift Helvetica (Breiten aus `crate::font`).
    Helvetica,
    /// Feste Breite (Courier).
    Fixed(f32),
}

fn num(pdf: &PdfDoc, o: &Object) -> Option<f32> {
    pdf.dereference(o).ok()?.1.as_float().ok()
}

fn deref_dict<'a>(pdf: &'a PdfDoc, o: &'a Object) -> Option<&'a Dictionary> {
    pdf.dereference(o).ok()?.1.as_dict().ok()
}

fn deref_array<'a>(pdf: &'a PdfDoc, o: &'a Object) -> Option<&'a Vec<Object>> {
    pdf.dereference(o).ok()?.1.as_array().ok()
}

impl<'a> FontInfo<'a> {
    pub fn load(pdf: &'a PdfDoc, font: &'a Dictionary) -> Self {
        let name = |d: &Dictionary, k: &[u8]| {
            d.get(k)
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .unwrap_or_default()
        };
        let subtype = name(font, b"Subtype");
        let base_font = name(font, b"BaseFont");
        // Mit Größenlimit: eine präparierte ToUnicode-CMap darf nicht unbegrenzt entpacken.
        let encoding = font.get_font_encoding_with_limit(pdf, 16 << 20).ok();
        // Lebensdauer ausdrücklich: Das Ergebnis zeigt in `pdf`, nicht in das übergebene `d`.
        let descriptor = |d: &'a Dictionary| -> Option<&'a Dictionary> {
            d.get(b"FontDescriptor")
                .ok()
                .and_then(|o| deref_dict(pdf, o))
        };
        let metrics = |fd: Option<&Dictionary>| {
            let get = |k: &[u8]| fd.and_then(|fd| fd.get(k).ok()).and_then(|o| num(pdf, o));
            let ascent = get(b"Ascent")
                .filter(|a| *a > 0.0)
                .map_or(0.8, |a| a / 1000.0);
            let descent = get(b"Descent")
                .filter(|d| *d < 0.0)
                .map_or(-0.2, |d| d / 1000.0);
            (ascent, descent, get(b"MissingWidth").unwrap_or(0.0))
        };

        if subtype == "Type0" {
            let desc = font
                .get(b"DescendantFonts")
                .ok()
                .and_then(|o| deref_array(pdf, o))
                .and_then(|a| a.first())
                .and_then(|o| deref_dict(pdf, o));
            let (ascent, descent, _) = metrics(desc.and_then(descriptor));
            let default = desc
                .and_then(|d| d.get(b"DW").ok())
                .and_then(|o| num(pdf, o))
                .unwrap_or(1000.0);
            let mut map = HashMap::new();
            if let Some(w) = desc
                .and_then(|d| d.get(b"W").ok())
                .and_then(|o| deref_array(pdf, o))
            {
                parse_cid_widths(pdf, w, &mut map);
            }
            // Bei Identity-CMaps ist Code = CID. Andere CMaps behandeln wir genähert.
            let enc = name(font, b"Encoding");
            let exact = enc.starts_with("Identity-");
            return Self {
                two_byte: true,
                widths: Widths::Cid { default, map },
                exact,
                ascent,
                descent,
                scale: 0.001,
                encoding,
                base_font,
                embedded: has_font_file(desc.and_then(descriptor)),
                unicode: None,
            };
        }

        let fd = descriptor(font);
        let (mut ascent, mut descent, missing) = metrics(fd);
        let mut scale = 0.001;
        if subtype == "Type3" {
            // Type3: Breiten im Glyphenraum, Umrechnung über /FontMatrix.
            if let Some(m) = font.get(b"FontMatrix").ok().and_then(Matrix::from_object) {
                scale = m.0[0];
            }
            if let Some(bb) = font.get(b"FontBBox").ok().and_then(|o| deref_array(pdf, o)) {
                let v: Vec<f32> = bb.iter().filter_map(|o| num(pdf, o)).collect();
                if v.len() == 4 && v[3] > v[1] {
                    ascent = v[3] * scale;
                    descent = v[1] * scale;
                }
            }
        }
        let first = font
            .get(b"FirstChar")
            .ok()
            .and_then(|o| num(pdf, o))
            .unwrap_or(0.0) as u32;
        let widths: Option<Vec<f32>> = font
            .get(b"Widths")
            .ok()
            .and_then(|o| deref_array(pdf, o))
            .map(|a| a.iter().map(|o| num(pdf, o).unwrap_or(0.0)).collect());
        let (widths, exact) = match widths {
            Some(widths) => (
                Widths::Simple {
                    first,
                    widths,
                    missing,
                },
                true,
            ),
            // Standardschriften ohne /Widths: Breiten kennen wir für Helvetica und Courier.
            None => {
                let plain = base_font.split('+').next_back().unwrap_or(&base_font);
                if plain.starts_with("Courier") {
                    (Widths::Fixed(600.0), true)
                } else if plain == "Helvetica" || plain == "Helvetica-Oblique" || plain == "Arial" {
                    (Widths::Helvetica, true)
                } else {
                    (Widths::Fixed(500.0), false)
                }
            }
        };
        Self {
            two_byte: false,
            widths,
            exact,
            ascent,
            descent,
            scale,
            encoding,
            base_font,
            embedded: has_font_file(fd),
            unicode: None,
        }
    }

    /// Eine Teilmenge der eingebauten Schrift, die (noch) nur im Modell existiert.
    pub fn from_subset(font: &crate::embedded_font::SubsetFont) -> Self {
        let (ascent, descent) = crate::embedded_font::ascent_descent();
        Self {
            two_byte: true,
            widths: Widths::Cid {
                default: 0.0,
                map: font.widths().map(|(g, w)| (u32::from(g), w)).collect(),
            },
            exact: true,
            ascent: ascent / 1000.0,
            descent: descent / 1000.0,
            scale: 0.001,
            encoding: None,
            base_font: "NotoSans-Regular".into(),
            embedded: true,
            unicode: Some(
                font.texts()
                    .map(|(g, t)| (u32::from(g), t.to_owned()))
                    .collect(),
            ),
        }
    }

    /// Teilmengen-Schrift ("ABCDEF+Name")? Dann enthält sie nur die benutzten Glyphen.
    pub fn is_subset(&self) -> bool {
        let b = self.base_font.as_bytes();
        b.len() > 7 && b[6] == b'+' && b[..6].iter().all(u8::is_ascii_uppercase)
    }

    /// Ganz ohne Schrift (z. B. `Tf` mit unbekanntem Namen): grob geschätzt.
    fn unknown() -> Self {
        Self {
            two_byte: false,
            widths: Widths::Fixed(500.0),
            exact: false,
            ascent: 0.8,
            descent: -0.2,
            scale: 0.001,
            encoding: None,
            base_font: String::new(),
            embedded: false,
            unicode: None,
        }
    }

    /// Breite eines Zeichencodes in Textraum-Einheiten (vor Multiplikation mit der Größe).
    pub fn width(&self, code: u32) -> f32 {
        let w = match &self.widths {
            Widths::Simple {
                first,
                widths,
                missing,
            } => code
                .checked_sub(*first)
                .and_then(|i| widths.get(i as usize))
                .copied()
                .unwrap_or(*missing),
            Widths::Cid { default, map } => map.get(&code).copied().unwrap_or(*default),
            Widths::Helvetica => {
                let c = crate::pdf_string::decode(&[code as u8])
                    .chars()
                    .next()
                    .unwrap_or(' ');
                crate::font::char_width(c, 1000.0)
            }
            Widths::Fixed(w) => *w,
        };
        w * self.scale
    }

    /// Zerlegt Bytes in Zeichencodes: (Startbyte, Länge, Code).
    pub fn codes(&self, bytes: &[u8]) -> Vec<(usize, usize, u32)> {
        if self.two_byte {
            bytes
                .chunks(2)
                .enumerate()
                .map(|(i, c)| {
                    let code = c.iter().fold(0u32, |acc, &b| acc << 8 | u32::from(b));
                    (i * 2, c.len(), code)
                })
                .collect()
        } else {
            bytes
                .iter()
                .enumerate()
                .map(|(i, &b)| (i, 1, u32::from(b)))
                .collect()
        }
    }

    pub fn to_unicode(&self, bytes: &[u8]) -> String {
        if let Some(map) = &self.unicode {
            return self
                .codes(bytes)
                .into_iter()
                .filter_map(|(_, _, c)| map.get(&c).map(String::as_str))
                .collect();
        }
        self.encoding
            .as_ref()
            .and_then(|e| e.bytes_to_string(bytes).ok())
            .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
    }

    pub fn is_two_byte(&self) -> bool {
        self.two_byte
    }
}

fn has_font_file(fd: Option<&Dictionary>) -> bool {
    fd.is_some_and(|fd| {
        [&b"FontFile"[..], b"FontFile2", b"FontFile3"]
            .iter()
            .any(|k| fd.has(k))
    })
}

/// /W: [c [w1 w2 …]] oder [c_first c_last w].
fn parse_cid_widths(pdf: &PdfDoc, w: &[Object], map: &mut HashMap<u32, f32>) {
    let mut i = 0;
    while i < w.len() {
        let Some(first) = num(pdf, &w[i]) else { break };
        let first = first as u32;
        match w.get(i + 1).and_then(|o| deref_array(pdf, o)) {
            Some(list) => {
                for (k, v) in list.iter().enumerate() {
                    if let Some(v) = num(pdf, v) {
                        map.insert(first + k as u32, v);
                    }
                }
                i += 2;
            }
            None => {
                let (Some(last), Some(v)) = (
                    w.get(i + 1).and_then(|o| num(pdf, o)),
                    w.get(i + 2).and_then(|o| num(pdf, o)),
                ) else {
                    break;
                };
                // Begrenzt: kaputte Dateien mit riesigen Bereichen sollen nicht den Speicher füllen.
                for cid in first..=(last as u32).min(first + 65_535) {
                    map.insert(cid, v);
                }
                i += 3;
            }
        }
    }
}

// ---------- Ergebnisse ----------

/// Wo ein Zeichen in den Operationen steht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphRef {
    /// Index der Operation (Tj, TJ, ', ").
    pub op: usize,
    /// Bei TJ: Index im Array; sonst 0.
    pub part: usize,
    /// Bytes innerhalb des Strings.
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Clone)]
pub struct Glyph {
    pub at: GlyphRef,
    pub code: u32,
    pub text: String,
    /// Rechteck auf der Seite (PDF-Benutzerraum, achsparallel).
    pub rect: Rect,
    /// Vorschub in Textraum-Einheiten (w0·Tfs + Tc + Tw), ohne horizontale Skalierung.
    pub advance: f32,
    /// Abstand vom Anfang der Textausgabe (x im Textraum, inkl. `Tz`).
    pub x: f32,
}

/// Eine Textausgabe-Operation mit ihren Zeichen.
#[derive(Debug, Clone)]
pub struct TextShow {
    pub op: usize,
    /// Name der Schrift in den Ressourcen.
    pub font: Vec<u8>,
    pub font_size: f32,
    /// Zeichen- und Wortabstand (`Tc`, `Tw`) und horizontale Skalierung (`Tz`/100).
    pub char_spacing: f32,
    pub word_spacing: f32,
    pub h_scale: f32,
    pub render_mode: i64,
    /// Operationen, die die Füllfarbe setzen (Farbraum `cs` und Farbe), zum Wiederholen.
    pub fill: Vec<Operation>,
    pub glyphs: Vec<Glyph>,
    /// Breiten sicher? Sonst ist `bounds` eine großzügige Obergrenze.
    pub exact: bool,
    /// Umfassendes Rechteck aller Zeichen (bei `exact == false` großzügig geschätzt).
    pub bounds: Rect,
    pub two_byte: bool,
    /// Textmatrix × CTM beim Start (Textraum -> Seite, ohne Schriftgröße).
    pub matrix: Matrix,
    /// Gesamtvorschub in x-Richtung des Textraums (inkl. TJ-Verschiebungen und `Tz`).
    pub width: f32,
    /// Hochstellung (`Ts`) – die Grundlinie liegt bei y = rise im Textraum.
    pub rise: f32,
}

impl TextShow {
    /// Anfang der Grundlinie auf der Seite.
    pub fn origin(&self) -> Point {
        self.matrix.apply(Point::new(0.0, self.rise))
    }
    /// Ende der Grundlinie auf der Seite (nach dem letzten Vorschub).
    pub fn end(&self) -> Point {
        self.matrix.apply(Point::new(self.width, self.rise))
    }
}

impl TextShow {
    pub fn text(&self) -> String {
        self.glyphs.iter().map(|g| g.text.as_str()).collect()
    }
}

#[derive(Debug, Clone)]
pub enum Item {
    Text(TextShow),
    /// Pfad: Operationen `start..=paint`. `painted` = füllt/zeichnet (nicht nur Clip mit `n`).
    Path {
        start: usize,
        paint: usize,
        bounds: Rect,
        painted: bool,
    },
    /// Bild-XObject (`Do`) oder Inline-Bild (`BI`, dann `name == None`).
    Image {
        op: usize,
        name: Option<Vec<u8>>,
        /// Abbildung des Einheitsquadrats auf die Seite.
        matrix: Matrix,
        bounds: Rect,
    },
    /// Formular-XObject (`Do`): Inhalt muss separat interpretiert werden.
    Form {
        op: usize,
        name: Vec<u8>,
        id: Option<ObjectId>,
        /// CTM inklusive /Matrix des Formulars.
        matrix: Matrix,
        bounds: Rect,
    },
    /// Flächenfüllung mit Farbverlauf (`sh`) – deckt den aktuellen Clip-Bereich.
    Shading {
        op: usize,
    },
}

/// Interpretierte Operationen eines Content Streams.
pub struct Interpreted {
    pub ops: Vec<Operation>,
    pub items: Vec<Item>,
    /// Markierte Inhalte (`BDC … <</MCID n>> … EMC`): je MCID das umfassende Rechteck und
    /// der Text darin – für Strukturbaum/Barrierefreiheit (Tagged PDF).
    pub marked: std::collections::BTreeMap<i64, MarkedContent>,
}

/// Inhalt eines markierten Abschnitts (siehe [`Interpreted::marked`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarkedContent {
    pub bounds: Option<Rect>,
    pub text: String,
    /// Enthält ein Bild (Figure).
    pub has_image: bool,
}

impl Interpreted {
    /// Text aller Zeichen (z. B. für Tests).
    pub fn text(&self) -> String {
        let mut s = String::new();
        for item in &self.items {
            if let Item::Text(t) = item {
                s.push_str(&t.text());
                s.push('\n');
            }
        }
        s
    }
}

// ---------- Interpreter ----------

#[derive(Clone)]
struct State {
    ctm: Matrix,
    font: Vec<u8>,
    size: f32,
    char_spacing: f32,
    word_spacing: f32,
    h_scale: f32,
    leading: f32,
    rise: f32,
    render_mode: i64,
    /// Füllfarbraum (`cs`) und Füllfarbe (`g`, `rg`, `k`, `sc`, `scn`).
    fill_space: Option<Operation>,
    fill_color: Option<Operation>,
}

impl State {
    fn new(ctm: Matrix) -> Self {
        Self {
            ctm,
            font: Vec::new(),
            size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render_mode: 0,
            fill_space: None,
            fill_color: None,
        }
    }
}

/// Bytes aller Content Streams einer Seite (Arrays werden verkettet).
pub fn page_content_bytes(pdf: &PdfDoc, page: ObjectId) -> Result<Vec<u8>> {
    let dict = pdf.get_dictionary(page)?;
    let mut out = Vec::new();
    let streams: Vec<&Object> = match dict.get(b"Contents") {
        Ok(Object::Array(a)) => a.iter().collect(),
        Ok(o) => vec![o],
        Err(_) => Vec::new(),
    };
    for s in streams {
        let (_, obj) = pdf.dereference(s)?;
        if let Ok(stream) = obj.as_stream() {
            let data = stream
                .decompressed_content()
                .unwrap_or_else(|_| stream.content.clone());
            out.extend_from_slice(&data);
            out.push(b'\n');
        }
    }
    Ok(out)
}

/// Ressourcen einer Seite (auch geerbt).
pub fn page_resources(pdf: &PdfDoc, page: ObjectId) -> Option<&Dictionary> {
    deref_dict(pdf, inherited(pdf, page, b"Resources")?)
}

/// Interpretiert eine Seite.
pub fn interpret_page(pdf: &PdfDoc, page: ObjectId) -> Result<Interpreted> {
    let bytes = page_content_bytes(pdf, page)?;
    let ops = Content::decode(&bytes)?.operations;
    Ok(interpret(
        pdf,
        ops,
        page_resources(pdf, page),
        Matrix::IDENTITY,
    ))
}

/// Interpretiert Operationen mit den gegebenen Ressourcen; `ctm` = Anfangsmatrix
/// (Seite: Identität, Formular: CTM beim `Do` × /Matrix).
pub fn interpret(
    pdf: &PdfDoc,
    ops: Vec<Operation>,
    resources: Option<&Dictionary>,
    ctm: Matrix,
) -> Interpreted {
    interpret_with(pdf, ops, resources, ctm, None, None)
}

/// Wie [`interpret`], mit zusätzlichen Schriften aus dem Modell.
pub fn interpret_with<'a>(
    pdf: &'a PdfDoc,
    ops: Vec<Operation>,
    resources: Option<&'a Dictionary>,
    ctm: Matrix,
    extra_fonts: Option<&'a ExtraFonts>,
    extra_images: Option<&'a ExtraImages>,
) -> Interpreted {
    let mut it = Interp {
        pdf,
        resources,
        fonts: HashMap::new(),
        extra_fonts,
        extra_images,
        marked: Default::default(),
    };
    let items = it.run(&ops, ctm);
    let marked = std::mem::take(&mut it.marked);
    Interpreted { ops, items, marked }
}

struct Interp<'a> {
    pdf: &'a PdfDoc,
    resources: Option<&'a Dictionary>,
    /// Schriften nach Ressourcenname (einmal geladen). `Rc` = geteilter Besitz ohne Kopie.
    fonts: HashMap<Vec<u8>, Rc<FontInfo<'a>>>,
    /// Schriften und Bilder, die nur im Modell existieren (Vorrang vor den Ressourcen).
    extra_fonts: Option<&'a ExtraFonts>,
    extra_images: Option<&'a ExtraImages>,
    /// Gesammelte markierte Inhalte (siehe [`Interpreted::marked`]).
    marked: std::collections::BTreeMap<i64, MarkedContent>,
}

/// Ressourcenname -> neues Bild-XObject, das noch nicht in der Datei steht.
pub type ExtraImages = std::collections::BTreeMap<Vec<u8>, crate::image_edit::NewImage>;

/// Ressourcenname -> eingebettete Teilmenge, die noch nicht in der Datei steht.
pub type ExtraFonts = std::collections::BTreeMap<Vec<u8>, crate::embedded_font::SubsetFont>;

/// Lädt die Schrift `name` aus den Ressourcen (oder `extra`).
pub fn load_font<'a>(
    pdf: &'a PdfDoc,
    resources: Option<&'a Dictionary>,
    extra: Option<&'a ExtraFonts>,
    name: &[u8],
) -> FontInfo<'a> {
    if let Some(sub) = extra.and_then(|e| e.get(name)) {
        return FontInfo::from_subset(sub);
    }
    let found = resources
        .and_then(|r| deref_dict(pdf, r.get(b"Font").ok()?))
        .and_then(|fonts| deref_dict(pdf, fonts.get(name).ok()?));
    match found {
        Some(d) => FontInfo::load(pdf, d),
        None => FontInfo::unknown(),
    }
}

impl<'a> Interp<'a> {
    fn resource(&self, category: &[u8], name: &[u8]) -> Option<(&'a Object, Option<ObjectId>)> {
        let cat = deref_dict(self.pdf, self.resources?.get(category).ok()?)?;
        let entry = cat.get(name).ok()?;
        let id = entry.as_reference().ok();
        Some((self.pdf.dereference(entry).ok()?.1, id))
    }

    fn font(&mut self, name: &[u8]) -> Rc<FontInfo<'a>> {
        if let Some(f) = self.fonts.get(name) {
            return f.clone();
        }
        let info = load_font(self.pdf, self.resources, self.extra_fonts, name);
        let rc = Rc::new(info);
        self.fonts.insert(name.to_vec(), rc.clone());
        rc
    }

    fn run(&mut self, ops: &[Operation], ctm: Matrix) -> Vec<Item> {
        let mut items = Vec::new();
        let mut gs = State::new(ctm);
        let mut stack: Vec<State> = Vec::new();
        let mut tm = Matrix::IDENTITY; // Textmatrix
        let mut tlm = Matrix::IDENTITY; // Zeilenmatrix
        // Laufender Pfad: Start-Operation, Punkte im Benutzerraum, Clip angefordert?
        let mut path_start: Option<usize> = None;
        let mut path_pts: Vec<Point> = Vec::new();
        let f = |o: &Object| o.as_float().unwrap_or(0.0);
        // Verschachtelte markierte Abschnitte; je Ebene die MCID (falls vorhanden).
        let mut mc_stack: Vec<Option<i64>> = Vec::new();

        for (i, op) in ops.iter().enumerate() {
            let a = &op.operands;
            let items_before = items.len();
            match op.operator.as_str() {
                "BMC" => mc_stack.push(None),
                "BDC" => {
                    // Eigenschaften direkt oder als Name in /Properties der Ressourcen.
                    let props = match a.get(1) {
                        Some(Object::Dictionary(d)) => Some(d),
                        Some(Object::Name(n)) => self
                            .resource(b"Properties", n)
                            .and_then(|(o, _)| o.as_dict().ok()),
                        _ => None,
                    };
                    mc_stack.push(props.and_then(|d| d.get(b"MCID").and_then(Object::as_i64).ok()));
                }
                "EMC" => {
                    mc_stack.pop();
                }
                _ => {}
            }
            match op.operator.as_str() {
                "q" => stack.push(gs.clone()),
                "Q" => {
                    if let Some(s) = stack.pop() {
                        gs = s;
                    }
                }
                "cm" => {
                    if let Some(m) = Matrix::from_operands(a) {
                        gs.ctm = m.then(&gs.ctm);
                    }
                }
                // Textzustand
                "Tf" => {
                    if let (Some(name), Some(size)) = (a.first(), a.get(1)) {
                        gs.font = name.as_name().map(<[u8]>::to_vec).unwrap_or_default();
                        gs.size = f(size);
                    }
                }
                "Tc" => gs.char_spacing = a.first().map_or(0.0, f),
                "Tw" => gs.word_spacing = a.first().map_or(0.0, f),
                "Tz" => gs.h_scale = a.first().map_or(100.0, f) / 100.0,
                "TL" => gs.leading = a.first().map_or(0.0, f),
                "Ts" => gs.rise = a.first().map_or(0.0, f),
                "Tr" => gs.render_mode = a.first().and_then(|o| o.as_i64().ok()).unwrap_or(0),
                // Füllfarbe (für Text-Neusatz); g/rg/k setzen den Farbraum gleich mit.
                "cs" => {
                    gs.fill_space = Some(op.clone());
                    gs.fill_color = None;
                }
                "g" | "rg" | "k" => {
                    gs.fill_space = None;
                    gs.fill_color = Some(op.clone());
                }
                "sc" | "scn" => gs.fill_color = Some(op.clone()),
                // Textpositionierung
                "BT" => {
                    tm = Matrix::IDENTITY;
                    tlm = Matrix::IDENTITY;
                }
                "Td" | "TD" => {
                    let (tx, ty) = (a.first().map_or(0.0, f), a.get(1).map_or(0.0, f));
                    if op.operator == "TD" {
                        gs.leading = -ty;
                    }
                    tlm = Matrix::translate(tx, ty).then(&tlm);
                    tm = tlm;
                }
                "Tm" => {
                    if let Some(m) = Matrix::from_operands(a) {
                        tlm = m;
                        tm = m;
                    }
                }
                "T*" => {
                    tlm = Matrix::translate(0.0, -gs.leading).then(&tlm);
                    tm = tlm;
                }
                // Textausgabe
                "Tj" | "'" | "\"" | "TJ" => {
                    if op.operator == "'" || op.operator == "\"" {
                        if op.operator == "\"" {
                            gs.word_spacing = a.first().map_or(0.0, f);
                            gs.char_spacing = a.get(1).map_or(0.0, f);
                        }
                        tlm = Matrix::translate(0.0, -gs.leading).then(&tlm);
                        tm = tlm;
                    }
                    let parts: Vec<&Object> = match op.operator.as_str() {
                        "TJ" => a
                            .first()
                            .and_then(|o| o.as_array().ok())
                            .map(|arr| arr.iter().collect())
                            .unwrap_or_default(),
                        "\"" => a.get(2).into_iter().collect(),
                        _ => a.first().into_iter().collect(),
                    };
                    let show = self.show(i, &parts, &gs, &mut tm);
                    items.push(Item::Text(show));
                }
                // Pfade
                "m" | "l" | "c" | "v" | "y" | "re" | "h" => {
                    path_start.get_or_insert(i);
                    let nums: Vec<f32> = a.iter().map(f).collect();
                    if op.operator == "re" && nums.len() == 4 {
                        let (x, y, w, h) = (nums[0], nums[1], nums[2], nums[3]);
                        for p in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                            path_pts.push(gs.ctm.apply(Point::new(p.0, p.1)));
                        }
                    } else {
                        // Kontrollpunkte von Kurven liegen außen – das Rechteck ist dadurch
                        // höchstens etwas zu groß, nie zu klein.
                        for p in nums.chunks_exact(2) {
                            path_pts.push(gs.ctm.apply(Point::new(p[0], p[1])));
                        }
                    }
                }
                "W" | "W*" => {
                    path_start.get_or_insert(i);
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                    if let Some(start) = path_start.take() {
                        items.push(Item::Path {
                            start,
                            paint: i,
                            bounds: Rect::bounding(path_pts.drain(..))
                                .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0)),
                            painted: op.operator != "n",
                        });
                    }
                    path_pts.clear();
                }
                "Do" => {
                    let Some(name) = a.first().and_then(|o| o.as_name().ok()) else {
                        continue;
                    };
                    if self.extra_images.is_some_and(|e| e.contains_key(name)) {
                        items.push(Item::Image {
                            op: i,
                            name: Some(name.to_vec()),
                            matrix: gs.ctm,
                            bounds: gs.ctm.apply_rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
                        });
                        continue;
                    }
                    let Some((obj, id)) = self.resource(b"XObject", name) else {
                        continue;
                    };
                    let Ok(stream) = obj.as_stream() else {
                        continue;
                    };
                    let subtype = stream
                        .dict
                        .get(b"Subtype")
                        .and_then(Object::as_name)
                        .unwrap_or_default();
                    let unit = Rect::new(0.0, 0.0, 1.0, 1.0);
                    if subtype == b"Image" {
                        items.push(Item::Image {
                            op: i,
                            name: Some(name.to_vec()),
                            matrix: gs.ctm,
                            bounds: gs.ctm.apply_rect(unit),
                        });
                    } else if subtype == b"Form" {
                        let fm = stream
                            .dict
                            .get(b"Matrix")
                            .ok()
                            .and_then(Matrix::from_object)
                            .unwrap_or(Matrix::IDENTITY);
                        let matrix = fm.then(&gs.ctm);
                        let bbox = stream
                            .dict
                            .get(b"BBox")
                            .ok()
                            .and_then(|o| o.as_array().ok())
                            .map(|v| {
                                v.iter()
                                    .filter_map(|o| o.as_float().ok())
                                    .collect::<Vec<_>>()
                            })
                            .filter(|v| v.len() == 4)
                            .map(|v| Rect::new(v[0], v[1], v[2], v[3]).normalized())
                            .unwrap_or(unit);
                        items.push(Item::Form {
                            op: i,
                            name: name.to_vec(),
                            id,
                            matrix,
                            bounds: matrix.apply_rect(bbox),
                        });
                    }
                }
                "BI" => {
                    items.push(Item::Image {
                        op: i,
                        name: None,
                        matrix: gs.ctm,
                        bounds: gs.ctm.apply_rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
                    });
                }
                "sh" => items.push(Item::Shading { op: i }),
                _ => {}
            }
            // Neue Elemente dem innersten markierten Abschnitt mit MCID zuordnen.
            if let Some(mcid) = mc_stack.iter().rev().find_map(|m| *m) {
                for item in &items[items_before..] {
                    let entry = self.marked.entry(mcid).or_default();
                    let (bounds, text, image) = match item {
                        Item::Text(t) => (Some(t.bounds), t.text(), false),
                        Item::Image { bounds, .. } => (Some(*bounds), String::new(), true),
                        Item::Path {
                            bounds,
                            painted: true,
                            ..
                        } => (Some(*bounds), String::new(), false),
                        Item::Form { bounds, .. } => (Some(*bounds), String::new(), false),
                        _ => (None, String::new(), false),
                    };
                    if let Some(b) = bounds.filter(|b| b.width() > 0.0 || b.height() > 0.0) {
                        entry.bounds = Some(entry.bounds.map_or(b, |e| e.union(&b)));
                    }
                    entry.text.push_str(&text);
                    entry.has_image |= image;
                }
            }
        }
        items
    }

    /// Zeigt Strings (und bei TJ Verschiebungen) und bewegt die Textmatrix weiter.
    fn show(&mut self, op: usize, parts: &[&Object], gs: &State, tm: &mut Matrix) -> TextShow {
        let font = self.font(&gs.font);
        let size = gs.size;
        let th = gs.h_scale;
        let start = *tm;
        let mut x = 0.0;
        let mut glyphs = Vec::new();
        // Für ungenaue Breiten: Obergrenze, falls jedes Zeichen 1 em breit wäre.
        let mut worst = *tm;
        let mut worst_rect: Option<Rect> = None;
        for (part, o) in parts.iter().enumerate() {
            match o {
                Object::String(bytes, _) => {
                    for (start, len, code) in font.codes(bytes) {
                        let w0 = font.width(code);
                        // Wortabstand gilt nur für das Ein-Byte-Zeichen 32 (Leerzeichen).
                        let tw = if len == 1 && code == 32 {
                            gs.word_spacing
                        } else {
                            0.0
                        };
                        let advance = w0 * size + gs.char_spacing + tw;
                        // Zeichen im Textraum: Breite w0·Tfs·Th, Höhe von Unter- bis Oberlänge.
                        let trm = Matrix([size * th, 0.0, 0.0, size, 0.0, gs.rise])
                            .then(tm)
                            .then(&gs.ctm);
                        let r =
                            trm.apply_rect(Rect::new(0.0, font.descent, w0.max(0.0), font.ascent));
                        let text = font.to_unicode(&bytes[start..start + len]);
                        glyphs.push(Glyph {
                            at: GlyphRef {
                                op,
                                part,
                                start,
                                len,
                            },
                            code,
                            text,
                            rect: r,
                            advance,
                            x,
                        });
                        x += advance * th;
                        *tm = Matrix::translate(advance * th, 0.0).then(tm);
                        if !font.exact {
                            let wtrm = Matrix([size * th, 0.0, 0.0, size, 0.0, gs.rise])
                                .then(&worst)
                                .then(&gs.ctm);
                            let wr =
                                wtrm.apply_rect(Rect::new(0.0, font.descent, 1.0, font.ascent));
                            worst_rect = Some(worst_rect.map_or(wr, |x| x.union(&wr)));
                            let max_adv = (size + gs.char_spacing.max(0.0) + tw.max(0.0)) * th;
                            worst = Matrix::translate(max_adv, 0.0).then(&worst);
                        }
                    }
                }
                other => {
                    // TJ-Zahl: Verschiebung in Tausendstel Textraum-Einheiten (positiv = nach links).
                    let n = other.as_float().unwrap_or(0.0);
                    let tx = -n / 1000.0 * size * th;
                    x += tx;
                    *tm = Matrix::translate(tx, 0.0).then(tm);
                    if !font.exact {
                        // Für die Obergrenze nur nach rechts verschieben.
                        worst = Matrix::translate(tx.max(0.0), 0.0).then(&worst);
                    }
                }
            }
        }
        let exact_bounds = glyphs
            .iter()
            .map(|g| g.rect)
            .reduce(|a, b| a.union(&b))
            .unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0));
        let bounds = match worst_rect {
            Some(w) => w.union(&exact_bounds),
            None => exact_bounds,
        };
        TextShow {
            op,
            font: gs.font.clone(),
            font_size: size,
            char_spacing: gs.char_spacing,
            word_spacing: gs.word_spacing,
            h_scale: th,
            render_mode: gs.render_mode,
            fill: gs
                .fill_space
                .iter()
                .chain(&gs.fill_color)
                .cloned()
                .collect(),
            glyphs,
            exact: font.exact,
            bounds,
            two_byte: font.two_byte,
            matrix: start.then(&gs.ctm),
            width: x,
            rise: gs.rise,
        }
    }
}

/// Inhalt eines Formular-XObjects interpretieren (mit dessen eigenen Ressourcen, sonst denen
/// der Seite). `matrix` = CTM beim `Do` × /Matrix des Formulars.
pub fn interpret_form<'a>(
    pdf: &'a PdfDoc,
    form: &'a lopdf::Stream,
    parent_resources: Option<&'a Dictionary>,
    matrix: Matrix,
) -> Result<Interpreted> {
    let data = form
        .decompressed_content()
        .unwrap_or_else(|_| form.content.clone());
    let ops = Content::decode(&data)?.operations;
    let res = form
        .dict
        .get(b"Resources")
        .ok()
        .and_then(|o| deref_dict(pdf, o))
        .or(parent_resources);
    Ok(interpret(pdf, ops, res, matrix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_order() {
        let scale = Matrix([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        let move_ = Matrix::translate(10.0, 0.0);
        // Erst skalieren, dann verschieben.
        let p = scale.then(&move_).apply(Point::new(1.0, 1.0));
        assert_eq!((p.x, p.y), (12.0, 2.0));
        let p = move_.then(&scale).apply(Point::new(1.0, 1.0));
        assert_eq!((p.x, p.y), (22.0, 2.0));
    }

    #[test]
    fn inverse_roundtrip() {
        let m = Matrix([2.0, 0.5, -1.0, 3.0, 10.0, -4.0]);
        let p = Point::new(3.0, 7.0);
        let back = m.invert().unwrap().apply(m.apply(p));
        assert!((back.x - p.x).abs() < 1e-4 && (back.y - p.y).abs() < 1e-4);
        assert!(Matrix([1.0, 0.0, 0.0, 0.0, 0.0, 0.0]).invert().is_none());
    }

    #[test]
    fn cid_width_ranges() {
        let pdf = PdfDoc::new();
        let mut map = HashMap::new();
        let w = vec![
            Object::from(1),
            Object::Array(vec![500.into(), 600.into()]),
            Object::from(10),
            Object::from(12),
            Object::from(333),
        ];
        parse_cid_widths(&pdf, &w, &mut map);
        assert_eq!(map.get(&1), Some(&500.0));
        assert_eq!(map.get(&2), Some(&600.0));
        assert_eq!(map.get(&11), Some(&333.0));
        assert_eq!(map.get(&13), None);
    }
}
