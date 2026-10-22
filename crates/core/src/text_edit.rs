//! Text im Seiteninhalt bearbeiten und neuen Text hinzufügen (Phase 6.2).
//!
//! **Textblöcke:** Der Interpreter liefert Textausgaben (`Tj`, `TJ` …). Aufeinanderfolgende
//! Ausgaben mit derselben Schrift und Größe, die auf derselben Grundlinie direkt hintereinander
//! stehen, bilden einen Block. Ein fett gesetztes Wort ist also ein eigener Block – so bleibt
//! die Formatierung erhalten.
//!
//! **Bearbeiten:** Die erste Ausgabe des Blocks wird durch den neuen Text ersetzt, die übrigen
//! durch reine Verschiebungen (`[-1234] TJ`) mit genau ihrem alten Vorschub. Am Ende des neuen
//! Texts steht eine Ausgleichsverschiebung auf den alten Vorschub. Dadurch bleibt alles, was
//! danach relativ positioniert ist, an seinem Platz.
//!
//! **Originalschrift:** Eingebettete Teilmengen enthalten nur die benutzten Zeichen. Wir nehmen
//! deshalb nur Zeichencodes, die auf dieser Seite mit dieser Schrift schon vorkommen (bei nicht
//! eingebetteten oder vollständigen einfachen Schriften alle 256 Codes). Fehlende Zeichen
//! kommen aus der eingebauten Noto Sans (Ersatzschrift, eigene Ressource `/FcE`); danach wird
//! wieder auf die Originalschrift umgeschaltet. Fehlt das Leerzeichen, wird es als Abstand gesetzt.
//!
//! **Neuer Text** steht hinter dem (in `q … Q` eingeschlossenen) alten Inhalt, in Noto Sans.
//!
//! Der geänderte Inhalt liegt bis zum Speichern im Modell (`Model::page_content`);
//! `save` schreibt ihn als neuen Content Stream samt Schriften.

use crate::command::{Command, Model};
use crate::content::{self, ExtraFonts, Interpreted, Item, Matrix, TextShow};
use crate::document::Document;
use crate::embedded_font;
use crate::error::{CoreError, Result};
use crate::geometry::{Color, Point, Rect};
use lopdf::content::{Content, Operation};
use lopdf::{Object, ObjectId, StringFormat};
use std::any::Any;
use std::collections::HashMap;

/// Inhalt einer bearbeiteten Seite.
#[derive(Clone, Default)]
pub struct PageContent {
    /// Unkomprimierte Zeichenbefehle (ersetzt alle bisherigen Content Streams).
    pub content: Vec<u8>,
    /// Zusätzliche Schriften (Ressourcenname -> Noto-Teilmenge).
    pub fonts: ExtraFonts,
    /// Neue Bilder (Ressourcenname -> Bild-XObject).
    pub images: content::ExtraImages,
    /// Alter Inhalt steht in `q … Q`: Dahinter gilt wieder der normale Benutzerraum.
    pub isolated: bool,
    /// Änderungsstempel (0 = Inhalt aus der Datei); siehe `Model::content_epoch`.
    pub stamp: u64,
}

impl std::fmt::Debug for PageContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PageContent({} Bytes, {} Schriften, Stempel {})",
            self.content.len(),
            self.fonts.len(),
            self.stamp
        )
    }
}

/// Ein bearbeitbarer Textblock einer Seite.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBlock {
    /// Stempel des Inhalts, aus dem der Block stammt (zum Erkennen veralteter Blöcke).
    pub stamp: u64,
    /// Indizes der Textausgabe-Operationen.
    pub ops: Vec<usize>,
    pub text: String,
    /// Umfassendes Rechteck (PDF-Benutzerraum).
    pub bounds: Rect,
    /// Ressourcenname der Schrift und Größe aus `Tf`.
    pub font: Vec<u8>,
    pub font_size: f32,
    /// Tatsächliche Schrifthöhe auf der Seite (pt).
    pub size_on_page: f32,
    /// Anfang der Grundlinie und ihre Richtung (Einheitsvektor) auf der Seite.
    pub origin: Point,
    pub direction: Point,
    pub render_mode: i64,
}

fn length(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// Projektion von `p` auf die Grundlinie (Abstand entlang, Abstand quer).
fn project(origin: Point, dir: Point, p: Point) -> (f32, f32) {
    let (dx, dy) = (p.x - origin.x, p.y - origin.y);
    (dx * dir.x + dy * dir.y, dy * dir.x - dx * dir.y)
}

/// Fasst Textausgaben zu Blöcken zusammen (siehe Moduldoku). `is_fallback` erkennt
/// Ersatzschriften: Deren Läufe gehören zum Block ihrer Nachbarn (die Schrift des Blocks
/// ist dann die erste andere Schrift).
pub fn group_blocks(
    it: &Interpreted,
    stamp: u64,
    is_fallback: &dyn Fn(&[u8]) -> bool,
) -> Vec<TextBlock> {
    // Laufender Block + Ende seiner Grundlinie (entlang der Richtung).
    let mut blocks: Vec<TextBlock> = Vec::new();
    let mut end_u = 0.0;
    let mut open = false;
    for item in &it.items {
        let Item::Text(t) = item else { continue };
        if t.glyphs.is_empty() {
            continue; // reine Verschiebung – bleibt unangetastet
        }
        let m = t.matrix.0;
        let scale_x = length(m[0], m[1]);
        if scale_x <= f32::EPSILON || t.font_size == 0.0 {
            continue;
        }
        let dir = Point::new(m[0] / scale_x, m[1] / scale_x);
        let em = t.font_size.abs() * scale_x;
        let operator = it.ops.get(t.op).map_or("", |o| o.operator.as_str());
        let next_line = operator == "'" || operator == "\"";

        let joins = open
            && !next_line
            && blocks.last().is_some_and(|b| {
                let (u, v) = project(b.origin, b.direction, t.origin());
                (b.font == t.font || is_fallback(&b.font) || is_fallback(&t.font))
                    && (b.font_size - t.font_size).abs() < 0.01
                    && b.render_mode == t.render_mode
                    && dir.x * b.direction.x + dir.y * b.direction.y > 0.999
                    && v.abs() < 0.2 * em
                    && u >= end_u - 0.5 * em
                    && u <= end_u + 2.0 * em
            });
        if !joins {
            blocks.push(TextBlock {
                stamp,
                ops: Vec::new(),
                text: String::new(),
                bounds: t.bounds,
                font: t.font.clone(),
                font_size: t.font_size,
                size_on_page: t.font_size.abs() * length(m[2], m[3]),
                origin: t.origin(),
                direction: dir,
                render_mode: t.render_mode,
            });
            open = true;
        }
        let Some(b) = blocks.last_mut() else { continue };
        if is_fallback(&b.font) && !is_fallback(&t.font) {
            b.font = t.font.clone();
        }
        b.ops.push(t.op);
        b.bounds = b.bounds.union(&t.bounds);
        // Text mit Leerzeichen für große Lücken (viele PDFs setzen Wortabstände als Verschiebung).
        let mut last_end = if joins { Some(end_u) } else { None };
        for g in &t.glyphs {
            let start = project(
                b.origin,
                b.direction,
                t.matrix.apply(Point::new(g.x, t.rise)),
            )
            .0;
            if let Some(e) = last_end
                && start - e > 0.25 * em
                && !b.text.ends_with(char::is_whitespace)
                && !g.text.starts_with(char::is_whitespace)
            {
                b.text.push(' ');
            }
            b.text.push_str(&g.text);
            let end = t
                .matrix
                .apply(Point::new(g.x + g.advance * t.h_scale, t.rise));
            last_end = Some(project(b.origin, b.direction, end).0);
        }
        end_u = project(b.origin, b.direction, t.end()).0;
    }
    blocks
}

// ---------- Command ----------

/// Ersetzt den Inhalt einer Seite (Text bearbeitet oder hinzugefügt).
#[derive(Debug)]
pub struct ReplacePageContent {
    page: ObjectId,
    before: PageContent,
    after: PageContent,
    label: String,
}

impl ReplacePageContent {
    pub fn new(page: ObjectId, before: PageContent, after: PageContent, label: &str) -> Self {
        Self {
            page,
            before,
            after,
            label: label.to_owned(),
        }
    }
}

fn install(model: &mut Model, page: ObjectId, mut content: PageContent) {
    model.content_epoch += 1;
    content.stamp = model.content_epoch;
    model.page_content.insert(page, content);
}

impl Command for ReplacePageContent {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        install(model, self.page, self.after.clone());
        Ok(())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        install(model, self.page, self.before.clone());
        Ok(())
    }
    fn label(&self) -> String {
        self.label.clone()
    }
}

// ---------- Kodieren ----------

/// Teil des neuen Texts: in der Originalschrift (TJ-Array) oder in der Ersatzschrift.
enum Segment {
    Original(Vec<Object>),
    Fallback(String),
}

/// Kodiert Text für eine Originalschrift – nur mit Codes, die sicher darstellbar sind.
struct Encoder<'a> {
    font: content::FontInfo<'a>,
    /// Text eines Zeichens (auch Ligaturen wie "fi") -> (Bytes, Code).
    map: HashMap<String, (Vec<u8>, u32)>,
}

impl<'a> Encoder<'a> {
    fn new(
        pdf: &'a lopdf::Document,
        resources: Option<&'a lopdf::Dictionary>,
        it: &Interpreted,
        font_name: &[u8],
    ) -> Self {
        let font = content::load_font(pdf, resources, None, font_name);
        let mut map = HashMap::new();
        // 1) Was auf der Seite mit dieser Schrift schon vorkommt, ist sicher enthalten.
        for item in &it.items {
            let Item::Text(t) = item else { continue };
            if t.font != font_name {
                continue;
            }
            for g in &t.glyphs {
                if g.text.is_empty() || g.text.chars().any(char::is_control) {
                    continue;
                }
                let bytes = if g.at.len == 1 {
                    vec![g.code as u8]
                } else {
                    (g.code as u16).to_be_bytes().to_vec()
                };
                map.entry(g.text.clone()).or_insert((bytes, g.code));
            }
        }
        // 2) Einfache Schriften, die nicht als Teilmenge eingebettet sind: alle Codes.
        if !font.is_two_byte() && !(font.embedded && font.is_subset()) {
            for code in 0..=255u8 {
                let text = font.to_unicode(&[code]);
                let usable = !text.is_empty()
                    && !text.chars().any(|c| c.is_control() || c == '\u{fffd}')
                    && (font.width(u32::from(code)) > 0.0 || text.trim().is_empty());
                if usable {
                    map.entry(text).or_insert((vec![code], u32::from(code)));
                }
            }
        }
        Self { font, map }
    }
}

/// Zerlegt `text` in Original- und Ersatzschrift-Teile und berechnet den Vorschub
/// (Textraum-Einheiten, inkl. `Tz`) – wie der Interpreter ihn messen würde.
fn segment(enc: &Encoder, text: &str, show: &TextShow) -> Result<(Vec<Segment>, f32)> {
    let (size, th) = (show.font_size, show.h_scale);
    let chars: Vec<char> = text.chars().collect();
    let mut segs: Vec<Segment> = Vec::new();
    let mut width = 0.0;
    let mut missing = String::new();
    let mut i = 0;
    let encodable = |c: char| enc.map.contains_key(c.encode_utf8(&mut [0; 4]) as &str);
    // Die eigene Noto-Teilmenge aus der Datei: Fehlt etwas, kommt der ganze Text in die
    // (erweiterbare) Ersatzschrift – gleiche Schrift, aber ein zusammenhängender Block.
    let all_fallback =
        enc.font.base_font.ends_with("+NotoSans-Regular") && !text.chars().all(encodable);
    'outer: while i < chars.len() {
        // Leerzeichen zwischen zwei Ersatzzeichen bleibt in der Ersatzschrift.
        let keep_fallback = all_fallback
            || (chars[i].is_whitespace()
                && matches!(segs.last(), Some(Segment::Fallback(_)))
                && chars[i..]
                    .iter()
                    .find(|c| !c.is_whitespace())
                    .is_some_and(|&c| !encodable(c)));
        // Längste passende Zeichenfolge zuerst (Ligaturen).
        for k in (1..=3).rev() {
            if keep_fallback {
                break;
            }
            if i + k > chars.len() {
                continue;
            }
            let s: String = chars[i..i + k].iter().collect();
            if let Some((bytes, code)) = enc.map.get(&s) {
                let tw = if bytes.len() == 1 && *code == 32 {
                    show.word_spacing
                } else {
                    0.0
                };
                width += (enc.font.width(*code) * size + show.char_spacing + tw) * th;
                match segs.last_mut() {
                    Some(Segment::Original(parts)) => match parts.last_mut() {
                        Some(Object::String(b, _)) => b.extend_from_slice(bytes),
                        _ => parts.push(Object::String(bytes.clone(), StringFormat::Literal)),
                    },
                    _ => segs.push(Segment::Original(vec![Object::String(
                        bytes.clone(),
                        StringFormat::Literal,
                    )])),
                }
                i += k;
                continue 'outer;
            }
        }
        let c = chars[i];
        i += 1;
        // Leerzeichen ohne Glyphe in der Originalschrift: als Abstand setzen.
        if c.is_whitespace()
            && !keep_fallback
            && let Some(Segment::Original(parts)) = segs.last_mut()
        {
            let n = -embedded_font::char_width(' ', 1000.0);
            parts.push(Object::Real(n));
            width += -n / 1000.0 * size * th;
            continue;
        }
        if !embedded_font::has_glyph(c) {
            missing.push(c);
            continue;
        }
        width += (embedded_font::char_width(c, size) + show.char_spacing) * th;
        match segs.last_mut() {
            Some(Segment::Fallback(s)) => s.push(c),
            _ => segs.push(Segment::Fallback(c.to_string())),
        }
    }
    if !missing.is_empty() {
        return Err(CoreError::Other(format!(
            "Diese Zeichen sind in keiner verfügbaren Schrift enthalten: {missing}"
        )));
    }
    Ok((segs, width))
}

/// Verschiebung um `width` (Textraum, inkl. `Tz`) als `[n] TJ`.
fn shift_op(width: f32, size: f32, th: f32) -> Option<Operation> {
    let unit = size * th;
    if width.abs() < 1e-4 || unit.abs() < 1e-6 {
        return None;
    }
    Some(Operation::new(
        "TJ",
        vec![Object::Array(vec![Object::Real(-width * 1000.0 / unit)])],
    ))
}

/// Schließt den bisherigen Inhalt in `q … Q` ein (einmalig): Ein offener Grafikzustand
/// (z. B. `cm` ohne `Q`) würde sonst neu angehängte Inhalte verschieben.
pub(crate) fn isolate(pc: &mut PageContent) {
    if pc.isolated {
        return;
    }
    let mut c = b"q\n".to_vec();
    c.extend_from_slice(&pc.content);
    c.extend_from_slice(b"\nQ\n");
    pc.content = c;
    pc.isolated = true;
}

/// Hängt Operationen an den Inhalt an. Vorher ein Zeilenumbruch: lopdf schreibt hinter die
/// letzte Operation kein Trennzeichen, sonst würde z. B. aus `Q` und `q` der Operator `Qq`.
pub(crate) fn append_ops(pc: &mut PageContent, ops: Vec<Operation>) -> Result<()> {
    if pc.content.last().is_some_and(|b| !b.is_ascii_whitespace()) {
        pc.content.push(b'\n');
    }
    pc.content
        .extend_from_slice(&Content { operations: ops }.encode()?);
    Ok(())
}

/// Ressourcenname der Ersatzschrift: vorhandener Eintrag oder ein freier Name.
fn fallback_name(
    pc: &PageContent,
    resources: Option<&lopdf::Dictionary>,
    pdf: &lopdf::Document,
) -> Vec<u8> {
    if let Some(name) = pc.fonts.keys().next() {
        return name.clone();
    }
    let taken = |n: &[u8]| {
        resources
            .and_then(|r| r.get(b"Font").ok())
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .is_some_and(|d| d.has(n))
    };
    let mut name = b"FcE".to_vec();
    let mut i = 1;
    while taken(&name) {
        name = format!("FcE{i}").into_bytes();
        i += 1;
    }
    name
}

// ---------- Dokument ----------

impl Document {
    /// Änderungsstempel des Inhalts einer Seite (0 = unverändert aus der Datei).
    pub fn content_stamp(&self, page: ObjectId) -> u64 {
        self.model().page_content.get(&page).map_or(0, |c| c.stamp)
    }

    /// Aktueller Inhalt einer Seite (bearbeitet oder aus der Datei).
    pub fn page_content(&self, page_idx: usize) -> Result<PageContent> {
        let page = self.page(page_idx)?.object_id;
        if let Some(c) = self.model().page_content.get(&page) {
            return Ok(c.clone());
        }
        if self.pdf().get_object(page).is_err() {
            return Err(CoreError::Other(
                "Diese Seite wurde eingefügt und ist noch nicht gespeichert. \
                 Bitte zuerst speichern, dann bearbeiten."
                    .into(),
            ));
        }
        Ok(PageContent {
            content: content::page_content_bytes(self.pdf(), page)?,
            ..PageContent::default()
        })
    }

    pub(crate) fn interpret_content<'a>(
        &'a self,
        page: ObjectId,
        pc: &'a PageContent,
    ) -> Result<Interpreted> {
        let ops = Content::decode(&pc.content)?.operations;
        Ok(content::interpret_with(
            self.pdf(),
            ops,
            content::page_resources(self.pdf(), page),
            Matrix::IDENTITY,
            Some(&pc.fonts),
            Some(&pc.images),
        ))
    }

    /// Bearbeitbare Textblöcke einer Seite.
    pub fn text_blocks(&self, page_idx: usize) -> Result<Vec<TextBlock>> {
        let pc = self.page_content(page_idx)?;
        let page = self.page(page_idx)?.object_id;
        let it = self.interpret_content(page, &pc)?;
        let resources = content::page_resources(self.pdf(), page);
        let is_fallback = |name: &[u8]| {
            pc.fonts.contains_key(name)
                || content::load_font(self.pdf(), resources, None, name)
                    .base_font
                    .ends_with("+NotoSans-Regular")
        };
        Ok(group_blocks(&it, pc.stamp, &is_fallback))
    }

    /// Ersetzt den Text eines Blocks (ein Undo-Schritt).
    pub fn edit_text_block(
        &mut self,
        page_idx: usize,
        block: &TextBlock,
        new_text: &str,
    ) -> Result<()> {
        let page = self.page(page_idx)?.object_id;
        let before = self.page_content(page_idx)?;
        if before.stamp != block.stamp {
            return Err(CoreError::Other(
                "Der Seiteninhalt hat sich inzwischen geändert.".into(),
            ));
        }
        let after = self.build_edit(page, &before, block, new_text)?;
        self.execute(Box::new(ReplacePageContent::new(
            page,
            before,
            after,
            "Text bearbeiten",
        )))
    }

    fn build_edit(
        &self,
        page: ObjectId,
        before: &PageContent,
        block: &TextBlock,
        new_text: &str,
    ) -> Result<PageContent> {
        let pdf = self.pdf();
        let resources = content::page_resources(pdf, page);
        let it = self.interpret_content(page, before)?;
        let shows: Vec<&TextShow> = block
            .ops
            .iter()
            .map(|&op| {
                it.items.iter().find_map(|item| match item {
                    Item::Text(t) if t.op == op => Some(t),
                    _ => None,
                })
            })
            .collect::<Option<_>>()
            .ok_or_else(|| CoreError::Other("Textblock passt nicht zum Seiteninhalt.".into()))?;
        let Some(&first) = shows.first() else {
            return Err(CoreError::Other("Leerer Textblock.".into()));
        };

        let mut after = before.clone();
        let mut new_ops: Vec<Operation> = Vec::new();
        // ' und " springen erst in die nächste Zeile – das muss erhalten bleiben.
        let original = &it.ops[first.op];
        match original.operator.as_str() {
            "'" => new_ops.push(Operation::new("T*", vec![])),
            "\"" => {
                let a = &original.operands;
                new_ops.push(Operation::new(
                    "Tw",
                    vec![a.first().cloned().unwrap_or(0.into())],
                ));
                new_ops.push(Operation::new(
                    "Tc",
                    vec![a.get(1).cloned().unwrap_or(0.into())],
                ));
                new_ops.push(Operation::new("T*", vec![]));
            }
            _ => {}
        }

        let size = Object::Real(first.font_size);
        // Beginnt der Block in einer Ersatzschrift, erst auf die Schrift des Blocks umschalten.
        if first.font != block.font {
            new_ops.push(Operation::new(
                "Tf",
                vec![Object::Name(block.font.clone()), size.clone()],
            ));
        }
        let new_width = if after.fonts.contains_key(&block.font) {
            // Block steht schon in der Ersatzschrift: einfach neu kodieren.
            let sub = after.fonts.get_mut(&block.font).expect("gerade geprüft");
            let missing: String = new_text
                .chars()
                .filter(|&c| !embedded_font::has_glyph(c))
                .collect();
            if !missing.is_empty() {
                return Err(CoreError::Other(format!(
                    "Diese Zeichen sind in keiner verfügbaren Schrift enthalten: {missing}"
                )));
            }
            new_ops.push(Operation::new(
                "TJ",
                vec![Object::Array(vec![sub.encode_object(new_text)])],
            ));
            new_text
                .chars()
                .map(|c| {
                    (embedded_font::char_width(c, first.font_size) + first.char_spacing)
                        * first.h_scale
                })
                .sum()
        } else {
            let enc = Encoder::new(pdf, resources, &it, &block.font);
            let (segs, width) = segment(&enc, new_text, first)?;
            let format = if enc.font.is_two_byte() {
                StringFormat::Hexadecimal
            } else {
                StringFormat::Literal
            };
            let fallback = fallback_name(&after, resources, pdf);
            for seg in segs {
                match seg {
                    Segment::Original(mut parts) => {
                        for p in &mut parts {
                            if let Object::String(_, f) = p {
                                *f = format;
                            }
                        }
                        new_ops.push(Operation::new("TJ", vec![Object::Array(parts)]));
                    }
                    Segment::Fallback(text) => {
                        let sub = after.fonts.entry(fallback.clone()).or_default();
                        new_ops.push(Operation::new(
                            "Tf",
                            vec![Object::Name(fallback.clone()), size.clone()],
                        ));
                        new_ops.push(Operation::new(
                            "TJ",
                            vec![Object::Array(vec![sub.encode_object(&text)])],
                        ));
                        new_ops.push(Operation::new(
                            "Tf",
                            vec![Object::Name(block.font.clone()), size.clone()],
                        ));
                    }
                }
            }
            width
        };
        // Ausgleich: Danach steht die Textmatrix genau dort, wo sie vorher stand.
        new_ops.extend(shift_op(
            first.width - new_width,
            first.font_size,
            first.h_scale,
        ));
        if first.font != block.font {
            new_ops.push(Operation::new(
                "Tf",
                vec![Object::Name(first.font.clone()), size.clone()],
            ));
        }

        let rest: HashMap<usize, &TextShow> = shows[1..].iter().map(|t| (t.op, *t)).collect();
        let mut ops = Vec::with_capacity(it.ops.len() + new_ops.len());
        let mut new_ops = Some(new_ops);
        for (i, op) in it.ops.into_iter().enumerate() {
            if i == first.op {
                ops.extend(new_ops.take().unwrap_or_default());
            } else if let Some(t) = rest.get(&i) {
                ops.extend(shift_op(t.width, t.font_size, t.h_scale));
            } else {
                ops.push(op);
            }
        }
        after.content = Content { operations: ops }.encode()?;
        Ok(after)
    }

    /// Fügt neuen Text in Noto Sans hinzu. `origin` = Anfang der ersten Grundlinie
    /// (PDF-Benutzerraum); der Text steht in der Anzeige aufrecht (auch auf gedrehten Seiten).
    /// Zeilenumbrüche (`\n`) ergeben neue Zeilen mit 1,2-fachem Abstand.
    pub fn add_text(
        &mut self,
        page_idx: usize,
        origin: Point,
        text: &str,
        size: f32,
        color: Color,
    ) -> Result<()> {
        let info = self.page(page_idx)?;
        let (page, rotation) = (info.object_id, info.rotation);
        let before = self.page_content(page_idx)?;
        let missing: String = text
            .chars()
            .filter(|&c| c != '\n' && !embedded_font::has_glyph(c))
            .collect();
        if !missing.is_empty() {
            return Err(CoreError::Other(format!(
                "Diese Zeichen sind in keiner verfügbaren Schrift enthalten: {missing}"
            )));
        }
        let mut after = before.clone();
        isolate(&mut after);
        let name = fallback_name(
            &after,
            content::page_resources(self.pdf(), page),
            self.pdf(),
        );
        let sub = after.fonts.entry(name.clone()).or_default();
        // Aufrecht in der Anzeige: um die Seitendrehung gegen den Uhrzeigersinn drehen.
        let (sin, cos) = (f32::from(rotation)).to_radians().sin_cos();
        let mut ops = vec![
            Operation::new("q", vec![]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(name), size.into()]),
            Operation::new("rg", vec![color.r.into(), color.g.into(), color.b.into()]),
            Operation::new("TL", vec![(size * 1.2).into()]),
            Operation::new(
                "Tm",
                vec![
                    cos.into(),
                    sin.into(),
                    (-sin).into(),
                    cos.into(),
                    origin.x.into(),
                    origin.y.into(),
                ],
            ),
        ];
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                ops.push(Operation::new("T*", vec![]));
            }
            if !line.is_empty() {
                ops.push(Operation::new("Tj", vec![sub.encode_object(line)]));
            }
        }
        ops.push(Operation::new("ET", vec![]));
        ops.push(Operation::new("Q", vec![]));
        append_ops(&mut after, ops)?;
        self.execute(Box::new(ReplacePageContent::new(
            page,
            before,
            after,
            "Text hinzufügen",
        )))
    }
}
