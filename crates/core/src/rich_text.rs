//! Formatierter Text (Phase 11): Absätze aus Läufen mit eigener Schrift, Größe, Farbe sowie
//! fett, kursiv, unterstrichen und durchgestrichen – zum Bearbeiten vorhandener Absätze und
//! für neuen Text.
//!
//! **Schriftwahl je Zeichen** (`Resolver`):
//! - *Originalschrift* der PDF, solange Fett/Kursiv unverändert sind und die Schrift das Zeichen
//!   sicher enthält (siehe `text_edit::Encoder`).
//! - Sonst die gleichnamige **Systemschrift** im gewünschten Schnitt (z. B. Arial Bold), als
//!   Teilmenge eingebettet – so sehen fehlende Zeichen aus wie der Rest.
//! - Sonst die eingebaute **Noto Sans**.
//! - Gibt es keinen fetten/kursiven Schnitt, wird er nachgeahmt: fett = Kontur in Textfarbe
//!   (`Tr 2`), kursiv = geneigte Textmatrix.
//!
//! **Setzen:** Jedes Stück mit gleicher Schrift und gleichem Stil bekommt eine eigene absolute
//! Position (`Tm`); die Breiten stammen aus denselben Schriften, die auch gezeichnet werden.
//! Unter- und Durchstreichungen sind gefüllte Rechtecke hinter dem Text.

use crate::content::{self, Interpreted, TextShow};
use crate::document::Document;
use crate::embedded_font::{FontData, SubsetFont};
use crate::error::{CoreError, Result};
use crate::geometry::{Color, Point};
use crate::system_fonts;
use crate::text_edit::{Encoder, PageContent};
use lopdf::content::Operation;
use lopdf::{Object, StringFormat};
use std::collections::HashMap;
use std::sync::Arc;

/// Welche Schrift ein Lauf verwendet.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FontRef {
    /// Schrift der Seite (Ressourcenname aus `/Font`).
    Original(Vec<u8>),
    /// Installierte Systemschrift (Familienname).
    Family(String),
    /// Eingebaute Noto Sans.
    Noto,
}

/// Aussehen eines Laufs. `size` = Schrifthöhe auf der Seite in pt.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font: FontRef,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Color,
}

impl TextStyle {
    pub fn noto(size: f32, color: Color) -> Self {
        Self {
            font: FontRef::Noto,
            size,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color,
        }
    }
}

/// Text mit einheitlichem Aussehen.
#[derive(Debug, Clone, PartialEq)]
pub struct StyledRun {
    pub text: String,
    pub style: TextStyle,
}

/// Gesamter Text der Läufe.
pub fn plain_text(runs: &[StyledRun]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

/// Gleiche Nachbarn zusammenfassen, leere Läufe weglassen.
pub fn normalize(runs: Vec<StyledRun>) -> Vec<StyledRun> {
    let mut out: Vec<StyledRun> = Vec::new();
    for r in runs {
        if r.text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.style == r.style => last.text.push_str(&r.text),
            _ => out.push(r),
        }
    }
    out
}

/// Farbe aus den Füll-Operationen einer Textausgabe (Grau, RGB, CMYK; sonst Schwarz).
pub fn color_from_fill(fill: &[Operation]) -> Color {
    let nums = |op: &Operation| -> Vec<f32> {
        op.operands
            .iter()
            .filter_map(|o| o.as_float().ok())
            .collect()
    };
    let cmyk = |v: &[f32]| {
        Color::rgb(
            (1.0 - v[0]) * (1.0 - v[3]),
            (1.0 - v[1]) * (1.0 - v[3]),
            (1.0 - v[2]) * (1.0 - v[3]),
        )
    };
    for op in fill.iter().rev() {
        let v = nums(op);
        match (op.operator.as_str(), v.len()) {
            ("rg", 3) | ("sc", 3) | ("scn", 3) => return Color::rgb(v[0], v[1], v[2]),
            ("g", 1) | ("sc", 1) | ("scn", 1) => return Color::rgb(v[0], v[0], v[0]),
            ("k", 4) | ("sc", 4) | ("scn", 4) => return cmyk(&v),
            _ => {}
        }
    }
    Color::BLACK
}

/// Geometrie und Grundwerte, in die gesetzt wird.
pub(crate) struct Base {
    /// Textraum → Seite ohne Schriftgröße (a, b, c, d); a/b = Schreibrichtung.
    pub matrix: [f32; 4],
    pub dir: Point,
    pub up: Point,
    pub char_spacing: f32,
    pub h_scale: f32,
    pub render_mode: i64,
    /// Originalfarbe samt ihren Operationen (z. B. CMYK) – unverändert wird sie wiederholt.
    pub fill: Option<(Color, Vec<Operation>)>,
    /// Zeilenabstand als Vielfaches der Schriftgröße.
    pub line_factor: f32,
    /// Einrückung der ersten Zeile (pt).
    pub indent: f32,
}

impl Base {
    /// Waagerecht in der Anzeige (Seite um `rotation` gedreht), für neuen Text.
    pub(crate) fn upright(rotation: u16) -> Self {
        let (sin, cos) = f32::from(rotation).to_radians().sin_cos();
        Self {
            matrix: [cos, sin, -sin, cos],
            dir: Point::new(cos, sin),
            up: Point::new(-sin, cos),
            char_spacing: 0.0,
            h_scale: 1.0,
            render_mode: 0,
            fill: None,
            line_factor: 1.2,
            indent: 0.0,
        }
    }

    /// Wie die erste Textausgabe eines Absatzes.
    pub(crate) fn from_show(
        t: &TextShow,
        dir: Point,
        up: Point,
        line_factor: f32,
        indent: f32,
    ) -> Self {
        let m = t.matrix.0;
        Self {
            matrix: [m[0], m[1], m[2], m[3]],
            dir,
            up,
            char_spacing: t.char_spacing,
            h_scale: t.h_scale,
            render_mode: t.render_mode,
            fill: Some((color_from_fill(&t.fill), t.fill.clone())),
            line_factor,
            indent,
        }
    }

    fn sx(&self) -> f32 {
        self.matrix[0].hypot(self.matrix[1]).max(f32::EPSILON)
    }

    fn sy(&self) -> f32 {
        self.matrix[2].hypot(self.matrix[3]).max(f32::EPSILON)
    }
}

/// Konkrete Schrift eines Zeichens.
#[derive(Debug, Clone, PartialEq)]
enum Face {
    /// Originalschrift (Ressourcenname).
    Orig(Vec<u8>),
    /// Eingebettete Teilmenge (Noto oder Systemschrift).
    Sub(Arc<FontData>),
}

#[derive(Debug, Clone, PartialEq)]
struct Concrete {
    face: Face,
    fake_bold: bool,
    fake_italic: bool,
}

/// Originalschrift einer Seite: Codes, Stil und passende Systemschrift.
struct Orig<'a> {
    enc: Encoder<'a>,
    family: String,
    bold: bool,
    italic: bool,
}

/// Wählt je Zeichen die Schrift (siehe Moduldoku) und merkt sich geladene Schriften.
pub(crate) struct Resolver<'a> {
    pdf: &'a lopdf::Document,
    resources: Option<&'a lopdf::Dictionary>,
    it: &'a Interpreted,
    origs: HashMap<Vec<u8>, Orig<'a>>,
    systems: HashMap<(String, bool, bool), Option<Arc<FontData>>>,
    /// Ressourcenname → Schrift einer Teilmenge, die schon in `PageContent::fonts` steht
    /// (z. B. aus einer früheren Bearbeitung).
    own: HashMap<Vec<u8>, Arc<FontData>>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(
        pdf: &'a lopdf::Document,
        resources: Option<&'a lopdf::Dictionary>,
        it: &'a Interpreted,
        pc: &PageContent,
    ) -> Self {
        Self {
            pdf,
            resources,
            it,
            origs: HashMap::new(),
            systems: HashMap::new(),
            own: pc
                .fonts
                .iter()
                .map(|(k, v)| (k.clone(), v.font().clone()))
                .collect(),
        }
    }

    fn orig(&mut self, name: &[u8]) -> &Orig<'a> {
        if !self.origs.contains_key(name) {
            let enc = Encoder::new(self.pdf, self.resources, self.it, name);
            let (family, bold, italic) = system_fonts::parse_base_font(&enc.font.base_font);
            self.origs.insert(
                name.to_vec(),
                Orig {
                    enc,
                    family,
                    bold,
                    italic,
                },
            );
        }
        &self.origs[name]
    }

    /// Systemschrift einer Familie im gewünschten Schnitt (inkl. Ersatzfamilien).
    fn system(&mut self, family: &str, bold: bool, italic: bool) -> Option<Arc<FontData>> {
        let key = (family.to_string(), bold, italic);
        if let Some(hit) = self.systems.get(&key) {
            return hit.clone();
        }
        let wanted = format!(
            "{family}{}",
            match (bold, italic) {
                (true, true) => "-BoldItalic",
                (true, false) => "-Bold",
                (false, true) => "-Italic",
                (false, false) => "",
            }
        );
        let found = system_fonts::find_style(family, bold, italic)
            .or_else(|| system_fonts::find_for_base_font(&wanted))
            .and_then(system_fonts::load);
        self.systems.insert(key, found.clone());
        found
    }

    /// Schriftfamilie einer Originalschrift (für die Anzeige im Editor).
    pub(crate) fn family_of(&mut self, name: &[u8]) -> String {
        self.orig(name).family.clone()
    }

    fn resolve(&mut self, style: &TextStyle, c: char) -> Result<Concrete> {
        let (b, i) = (style.bold, style.italic);
        let sub = |f: Arc<FontData>| Concrete {
            fake_bold: b && !f.bold,
            fake_italic: i && !f.italic,
            face: Face::Sub(f),
        };
        let key = c.to_string();
        match &style.font {
            FontRef::Original(name) => {
                // Unsere eigene Teilmenge (frühere Bearbeitung): wie eine Familie behandeln.
                if let Some(f) = self.own.get(name).cloned() {
                    if f.has_glyph(c) {
                        if f.bold == b && f.italic == i {
                            return Ok(sub(f));
                        }
                        if let Some(v) = self.system(&f.family, b, i).filter(|v| v.has_glyph(c)) {
                            return Ok(sub(v));
                        }
                        return Ok(sub(f));
                    }
                    return self.noto(b, i, c);
                }
                let (has, same, family, ob, oi) = {
                    let o = self.orig(name);
                    (
                        o.enc.map.contains_key(&key),
                        o.bold == b && o.italic == i,
                        o.family.clone(),
                        o.bold,
                        o.italic,
                    )
                };
                if has && same {
                    return Ok(Concrete {
                        face: Face::Orig(name.clone()),
                        fake_bold: false,
                        fake_italic: false,
                    });
                }
                if let Some(f) = self.system(&family, b, i).filter(|f| f.has_glyph(c)) {
                    return Ok(sub(f));
                }
                if has {
                    // Kein passender Schnitt installiert: Originalschrift, Stil nachgeahmt.
                    return Ok(Concrete {
                        face: Face::Orig(name.clone()),
                        fake_bold: b && !ob,
                        fake_italic: i && !oi,
                    });
                }
                self.noto(b, i, c)
            }
            FontRef::Family(family) => {
                let family = family.clone();
                match self.system(&family, b, i).filter(|f| f.has_glyph(c)) {
                    Some(f) => Ok(sub(f)),
                    None => self.noto(b, i, c),
                }
            }
            FontRef::Noto => self.noto(b, i, c),
        }
    }

    fn noto(&self, b: bool, i: bool, c: char) -> Result<Concrete> {
        let f = FontData::noto();
        if !f.has_glyph(c) && !c.is_whitespace() {
            return Err(CoreError::Other(format!(
                "Dieses Zeichen ist in keiner verfügbaren Schrift enthalten: {c}"
            )));
        }
        Ok(Concrete {
            face: Face::Sub(f),
            fake_bold: b,
            fake_italic: i,
        })
    }

    /// Breite eines Zeichens in 1/1000 em der konkreten Schrift.
    fn width(&mut self, face: &Face, c: char) -> f32 {
        match face {
            Face::Orig(name) => {
                let o = self.orig(name);
                o.enc
                    .map
                    .get(&c.to_string())
                    .map_or(0.0, |(_, code)| o.enc.font.width(*code) * 1000.0)
            }
            Face::Sub(f) => f.widths_per_mille(&[c])[0],
        }
    }
}

/// Ein gesetztes Zeichen.
struct Atom {
    c: char,
    run: usize,
    face: Concrete,
    /// Vorschub auf der Seite (pt).
    advance: f32,
}

/// Ressourcenname der Teilmenge einer Schrift (vorhanden oder neu).
fn subset_name(
    after: &mut PageContent,
    resources: Option<&lopdf::Dictionary>,
    pdf: &lopdf::Document,
    font: &Arc<FontData>,
) -> Vec<u8> {
    if let Some((name, _)) = after
        .fonts
        .iter()
        .find(|(_, s)| s.font().postscript == font.postscript && s.font().index == font.index)
    {
        return name.clone();
    }
    let taken = |n: &[u8]| {
        resources
            .and_then(|r| r.get(b"Font").ok())
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .is_some_and(|d| d.has(n))
    };
    // Noto heißt wie bisher "FcE…", Systemschriften "FcS…".
    let prefix = if font.is_noto() { "FcE" } else { "FcS" };
    let mut i = 0;
    loop {
        let name = if i == 0 && font.is_noto() {
            prefix.as_bytes().to_vec()
        } else {
            format!("{prefix}{i}").into_bytes()
        };
        if !taken(&name) && !after.fonts.contains_key(&name) {
            after
                .fonts
                .insert(name.clone(), SubsetFont::with_font(font.clone()));
            return name;
        }
        i += 1;
    }
}

/// Wie gesetzt wird.
pub(crate) struct Layout {
    /// Anfang der ersten Grundlinie (vor der Einrückung).
    pub origin: Point,
    /// Breite für den Umbruch (pt); `None` = nur harte Umbrüche.
    pub width: Option<f32>,
    pub justify: bool,
}

/// Neigung für nachgeahmte Kursivschrift (≈ 12°).
const SKEW: f32 = 0.21;

/// Setzt formatierte Läufe und liefert die Operationen (ohne Markierung/`q`).
pub(crate) fn layout_ops(
    resolver: &mut Resolver,
    after: &mut PageContent,
    base: &Base,
    runs: &[StyledRun],
    layout: &Layout,
) -> Result<Vec<Operation>> {
    let (sx, sy, th) = (base.sx(), base.sy(), base.h_scale);
    // 1) Zeichen mit Schrift und Vorschub.
    let mut atoms: Vec<Atom> = Vec::new();
    for (ri, r) in runs.iter().enumerate() {
        let tf = r.style.size / sy;
        for c in r.text.chars() {
            if c == '\n' {
                atoms.push(Atom {
                    c,
                    run: ri,
                    face: Concrete {
                        face: Face::Sub(FontData::noto()),
                        fake_bold: false,
                        fake_italic: false,
                    },
                    advance: 0.0,
                });
                continue;
            }
            let face = resolver.resolve(&r.style, c)?;
            let w = resolver.width(&face.face, c);
            let advance = (w / 1000.0 * tf + base.char_spacing) * th * sx;
            atoms.push(Atom {
                c,
                run: ri,
                face,
                advance,
            });
        }
    }

    // 2) Zeilen: harte Umbrüche, dann Wortumbruch auf die Breite.
    let mut lines: Vec<(Vec<usize>, bool)> = Vec::new(); // (Atom-Indizes, letzte des Abschnitts)
    let mut start = 0;
    let mut first_line = true;
    for end in (0..=atoms.len()).filter(|&k| k == atoms.len() || atoms[k].c == '\n') {
        let section: Vec<usize> = (start..end).collect();
        start = end + 1;
        let mut current: Vec<usize> = Vec::new();
        let mut width = if first_line { base.indent } else { 0.0 };
        let mut last_space: Option<usize> = None; // Position in `current`
        for &k in &section {
            let a = &atoms[k];
            current.push(k);
            width += a.advance;
            if a.c == ' ' {
                last_space = Some(current.len() - 1);
                continue;
            }
            if let Some(max) = layout.width
                && width > max + 0.01
                && current.len() > 1
            {
                // Am letzten Leerzeichen umbrechen (das Leerzeichen bleibt am Zeilenende),
                // sonst mitten im Wort.
                let cut = last_space.map_or(current.len() - 1, |p| p + 1);
                let rest = current.split_off(cut);
                lines.push((std::mem::take(&mut current), false));
                current = rest;
                width = current.iter().map(|&k| atoms[k].advance).sum();
                last_space = current.iter().rposition(|&k| atoms[k].c == ' ');
            }
        }
        lines.push((current, true));
        first_line = false;
    }

    // 3) Ausgeben.
    let name = |n: &[u8]| Object::Name(n.to_vec());
    let mut ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tc", vec![base.char_spacing.into()]),
        Operation::new("Tw", vec![0.into()]),
        Operation::new("Tz", vec![(th * 100.0).into()]),
        Operation::new("Ts", vec![0.into()]),
    ];
    let mut deco: Vec<Operation> = Vec::new();
    let resources = resolver.resources;
    let pdf = resolver.pdf;
    let mut down = 0.0;
    let mut prev_size: Option<f32> = None;
    for (li, (line, last)) in lines.iter().enumerate() {
        // Zeilenhöhe nach der größten Schrift der Zeile.
        let max_size = line
            .iter()
            .map(|&k| runs[atoms[k].run].style.size)
            .fold(0.0_f32, f32::max);
        let size = if max_size > 0.0 {
            max_size
        } else {
            prev_size.unwrap_or(runs.first().map_or(12.0, |r| r.style.size))
        };
        if li > 0 {
            down += base.line_factor * size;
        }
        prev_size = Some(size);
        let indent = if li == 0 { base.indent } else { 0.0 };
        // Blocksatz: Restbreite auf die Leerzeichen (ohne das am Zeilenende).
        let trailing = line
            .iter()
            .rev()
            .take_while(|&&k| atoms[k].c == ' ')
            .count();
        let body = &line[..line.len() - trailing];
        let spaces = body.iter().filter(|&&k| atoms[k].c == ' ').count();
        let natural: f32 = body.iter().map(|&k| atoms[k].advance).sum::<f32>() + indent;
        let extra = match layout.width {
            Some(max) if layout.justify && !last && spaces > 0 && natural < max => {
                (max - natural) / spaces as f32
            }
            _ => 0.0,
        };
        // Leerzeichen am Ende einer umbrochenen Zeile werden nicht gesetzt (der Umbruch
        // ersetzt sie; sonst stünde die Zeile über den rechten Rand hinaus).
        let line: &[usize] = if *last { line } else { body };
        let line_origin = Point::new(
            layout.origin.x - base.up.x * down,
            layout.origin.y - base.up.y * down,
        );
        let mut x = indent;
        let mut k = 0;
        while k < line.len() {
            // Stück: gleiche Schrift, gleicher Lauf; nach einem Leerzeichen im Blocksatz Schluss.
            let first = &atoms[line[k]];
            let mut j = k + 1;
            while j < line.len() {
                let a = &atoms[line[j]];
                let prev = &atoms[line[j - 1]];
                if a.face != first.face
                    || runs[a.run].style != runs[first.run].style
                    || (extra > 0.0 && prev.c == ' ')
                {
                    break;
                }
                j += 1;
            }
            let piece: Vec<&Atom> = line[k..j].iter().map(|&i| &atoms[i]).collect();
            let text: String = piece.iter().map(|a| a.c).collect();
            let width: f32 = piece.iter().map(|a| a.advance).sum();
            let style = &runs[first.run].style;
            let tf = style.size / sy;
            let at = Point::new(
                line_origin.x + base.dir.x * x,
                line_origin.y + base.dir.y * x,
            );
            // Schrift wählen und Text kodieren.
            let (font_name, encoded) = match &first.face.face {
                Face::Orig(n) => {
                    let o = resolver.orig(n);
                    let two = o.enc.font.is_two_byte();
                    let mut bytes = Vec::new();
                    for c in text.chars() {
                        if let Some((b, _)) = o.enc.map.get(&c.to_string()) {
                            bytes.extend_from_slice(b);
                        }
                    }
                    let fmt = if two {
                        StringFormat::Hexadecimal
                    } else {
                        StringFormat::Literal
                    };
                    (n.clone(), Object::String(bytes, fmt))
                }
                Face::Sub(f) => {
                    let n = subset_name(after, resources, pdf, f);
                    let sub = after.fonts.get_mut(&n).expect("gerade angelegt");
                    (n, sub.encode_object(&text))
                }
            };
            // Farbe: Originalfarbe (mit ihren Operationen) oder RGB.
            let color_ops: Vec<Operation> = match &base.fill {
                Some((c, fill_ops)) if *c == style.color && !fill_ops.is_empty() => {
                    fill_ops.clone()
                }
                _ => vec![Operation::new(
                    "rg",
                    vec![
                        style.color.r.into(),
                        style.color.g.into(),
                        style.color.b.into(),
                    ],
                )],
            };
            ops.extend(color_ops);
            if first.face.fake_bold {
                let c = style.color;
                ops.push(Operation::new(
                    "RG",
                    vec![c.r.into(), c.g.into(), c.b.into()],
                ));
                ops.push(Operation::new("w", vec![(style.size * 0.03).into()]));
                ops.push(Operation::new("Tr", vec![2.into()]));
            } else {
                ops.push(Operation::new("Tr", vec![base.render_mode.into()]));
            }
            let [a, b, mut c, mut d] = base.matrix;
            if first.face.fake_italic {
                c += SKEW * a;
                d += SKEW * b;
            }
            ops.push(Operation::new("Tf", vec![name(&font_name), tf.into()]));
            ops.push(Operation::new(
                "Tm",
                vec![
                    a.into(),
                    b.into(),
                    c.into(),
                    d.into(),
                    at.x.into(),
                    at.y.into(),
                ],
            ));
            ops.push(Operation::new("Tj", vec![encoded]));

            // Unter-/Durchstreichung (ohne Leerzeichen am Ende des Stücks).
            let visible: f32 = width
                - piece
                    .iter()
                    .rev()
                    .take_while(|a| a.c == ' ')
                    .map(|a| a.advance)
                    .sum::<f32>();
            for (on, offset) in [(style.underline, -0.12), (style.strike, 0.28)] {
                if !on || visible <= 0.0 {
                    continue;
                }
                let t = style.size * 0.06;
                let mid = offset * style.size;
                let p = |along: f32, up: f32| {
                    Point::new(
                        at.x + base.dir.x * along + base.up.x * up,
                        at.y + base.dir.y * along + base.up.y * up,
                    )
                };
                let c = style.color;
                deco.push(Operation::new(
                    "rg",
                    vec![c.r.into(), c.g.into(), c.b.into()],
                ));
                let corners = [
                    p(0.0, mid - t / 2.0),
                    p(visible, mid - t / 2.0),
                    p(visible, mid + t / 2.0),
                    p(0.0, mid + t / 2.0),
                ];
                deco.push(Operation::new(
                    "m",
                    vec![corners[0].x.into(), corners[0].y.into()],
                ));
                for q in &corners[1..] {
                    deco.push(Operation::new("l", vec![q.x.into(), q.y.into()]));
                }
                deco.push(Operation::new("h", vec![]));
                deco.push(Operation::new("f", vec![]));
            }
            x += width + piece.iter().filter(|a| a.c == ' ').count() as f32 * extra;
            k = j;
        }
    }
    ops.push(Operation::new("ET", vec![]));
    ops.extend(deco);
    Ok(ops)
}

impl Document {
    /// Anzeigename einer Schrift der Seite ("Arial Bold" statt "/F1").
    pub fn font_display_name(&self, page_idx: usize, resource: &[u8]) -> String {
        let Ok(page) = self.page(page_idx).map(|p| p.object_id) else {
            return String::from_utf8_lossy(resource).into_owned();
        };
        let pc = self.page_content(page_idx).ok();
        if let Some(f) = pc.as_ref().and_then(|pc| pc.fonts.get(resource)) {
            return f.font().family.clone();
        }
        let res = content::page_resources(self.pdf(), page);
        let f = content::load_font(self.pdf(), res, None, resource);
        let b = f.base_font.as_bytes();
        let name = if b.len() > 7 && b[6] == b'+' {
            &f.base_font[7..]
        } else {
            &f.base_font
        };
        if name.is_empty() {
            String::from_utf8_lossy(resource).into_owned()
        } else {
            name.to_string()
        }
    }
}
