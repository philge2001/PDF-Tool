//! Absätze erkennen und neu setzen (Text bearbeiten über mehrere Zeilen, Kasten verschieben
//! und in der Breite ändern).
//!
//! **Erkennen:** Textausgaben auf derselben Grundlinie werden zu Zeilen zusammengefasst (egal in
//! welcher Schrift). Aufeinanderfolgende Zeilen bilden einen Absatz, wenn Richtung und
//! Schriftgröße passen, der linke Rand gleich ist (die erste Zeile darf eingerückt sein) und
//! der Zeilenabstand gleichmäßig ist. Blocksatz wird erkannt, wenn die Zeilen (außer der
//! letzten) rechts bündig enden.
//!
//! **Neu setzen:** Die alten Textausgaben werden durch reine Verschiebungen ersetzt (der Rest
//! des Inhalts bleibt, wo er ist). Der Absatz wird dann hinter dem bisherigen Inhalt neu
//! geschrieben: in der Schrift, Größe, Farbe, Richtung und mit dem Zeilenabstand seiner
//! ersten Zeile, umbrochen auf die Breite des Kastens. Fehlende Zeichen kommen aus Noto Sans.
//! Der neue Absatz steht in `/FcPara BMC … EMC` – beim nächsten Bearbeiten wird dieser
//! Abschnitt komplett ersetzt, statt weitere leere Reste anzuhäufen.
//!
//! *Grenze:* Formatierung innerhalb des Absatzes (z. B. ein fettes Wort) wird vereinheitlicht.

use crate::content::{Interpreted, Item, TextShow};
use crate::document::Document;
use crate::embedded_font;
use crate::error::{CoreError, Result};
use crate::geometry::{Point, Rect};
use crate::text_edit::{
    Encoder, PageContent, ReplacePageContent, Segment, append_ops, fallback_name, isolate, length,
    project, segment, shift_op,
};
use lopdf::content::{Content, Operation};
use lopdf::{Object, StringFormat};

/// Name des Abschnitts, in dem PDF Tool neu gesetzte Absätze ablegt.
const MARK: &[u8] = b"FcPara";

/// Ein erkannter Absatz.
#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    /// Stempel des Inhalts, aus dem er stammt.
    pub stamp: u64,
    /// Indizes aller Textausgabe-Operationen des Absatzes.
    pub shows: Vec<usize>,
    /// Text, Zeilen mit Leerzeichen verbunden (Trennstriche am Zeilenende entfernt).
    pub text: String,
    /// Umfassendes Rechteck der Zeichen (PDF-Benutzerraum).
    pub bounds: Rect,
    /// Linker Rand auf der ersten Grundlinie (PDF-Raum).
    pub origin: Point,
    /// Einrückung der ersten Zeile (entlang der Schreibrichtung, pt).
    pub indent: f32,
    /// Schreibrichtung und "nach oben" (Einheitsvektoren auf der Seite).
    pub direction: Point,
    pub up: Point,
    /// Abstand der Grundlinien (pt).
    pub line_height: f32,
    /// Breite der längsten Zeile ab dem linken Rand (pt).
    pub width: f32,
    pub lines: usize,
    pub justified: bool,
    /// Schrift (Ressourcenname) und Größe der ersten Zeile.
    pub font: Vec<u8>,
    pub font_size: f32,
    /// Schrifthöhe auf der Seite (pt).
    pub size_on_page: f32,
}

/// Wohin und wie breit ein Absatz gesetzt wird.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParaFrame {
    /// Linker Rand auf der ersten Grundlinie (PDF-Raum).
    pub origin: Point,
    /// Breite für den Umbruch (pt); `None` = nicht umbrechen (nur harte Zeilenumbrüche).
    pub width: Option<f32>,
    pub justify: bool,
}

impl Paragraph {
    /// Der Kasten, in dem der Absatz heute steht.
    pub fn frame(&self) -> ParaFrame {
        ParaFrame {
            origin: self.origin,
            // Einzeilige Absätze (Überschriften) nicht umbrechen, solange niemand die Breite ändert.
            // Etwas Spielraum: Die längste Zeile muss beim erneuten Umbruch sicher wieder passen.
            width: (self.lines > 1).then_some(self.width + 0.5),
            justify: self.justified,
        }
    }

    /// Kasten als Rechteck (für die Anzeige): von der Oberlänge der ersten bis zur Unterlänge
    /// der letzten Zeile, `width` breit. Nur bei waagerechter Schrift exakt.
    pub fn frame_rect(&self, frame: &ParaFrame, lines: usize) -> Rect {
        let w = frame.width.unwrap_or(self.width);
        let top = 0.8 * self.size_on_page;
        let bottom = (lines.max(1) - 1) as f32 * self.line_height + 0.25 * self.size_on_page;
        let corner = |along: f32, up: f32| {
            Point::new(
                frame.origin.x + self.direction.x * along + self.up.x * up,
                frame.origin.y + self.direction.y * along + self.up.y * up,
            )
        };
        Rect::bounding([
            corner(0.0, top),
            corner(w, top),
            corner(0.0, -bottom),
            corner(w, -bottom),
        ])
        .unwrap_or(self.bounds)
    }
}

/// Eine Zeile während der Erkennung.
struct Line<'a> {
    shows: Vec<&'a TextShow>,
    origin: Point,
    dir: Point,
    em: f32,
    /// Anfang und Ende entlang der Richtung, gemessen ab `origin`.
    end: f32,
    text: String,
    bounds: Rect,
}

fn dot(a: Point, b: Point) -> f32 {
    a.x * b.x + a.y * b.y
}

/// Zeilen aus den Textausgaben (in Inhaltsreihenfolge).
fn lines<'a>(it: &'a Interpreted) -> Vec<Line<'a>> {
    let mut out: Vec<Line> = Vec::new();
    for item in &it.items {
        let Item::Text(t) = item else { continue };
        if t.glyphs.is_empty() || t.font_size == 0.0 {
            continue;
        }
        let m = t.matrix.0;
        let scale_x = length(m[0], m[1]);
        if scale_x <= f32::EPSILON {
            continue;
        }
        let dir = Point::new(m[0] / scale_x, m[1] / scale_x);
        let em = t.font_size.abs() * length(m[2], m[3]);
        let operator = it.ops.get(t.op).map_or("", |o| o.operator.as_str());
        let joins = operator != "'"
            && operator != "\""
            && out.last().is_some_and(|l| {
                let (u, v) = project(l.origin, l.dir, t.origin());
                l.shows[0].render_mode == t.render_mode
                    && dot(dir, l.dir) > 0.999
                    && v.abs() < 0.2 * em.max(l.em)
                    && u >= l.end - 0.5 * em
                    && u <= l.end + 2.0 * em
            });
        if !joins {
            out.push(Line {
                shows: Vec::new(),
                origin: t.origin(),
                dir,
                em,
                end: 0.0,
                text: String::new(),
                bounds: t.bounds,
            });
        }
        let Some(l) = out.last_mut() else { continue };
        // Leerzeichen für Lücken (wie bei den Textblöcken).
        let mut last_end = if joins { Some(l.end) } else { None };
        for g in &t.glyphs {
            let start = project(l.origin, l.dir, t.matrix.apply(Point::new(g.x, t.rise))).0;
            if let Some(e) = last_end
                && start - e > 0.25 * em
                && !l.text.ends_with(char::is_whitespace)
                && !g.text.starts_with(char::is_whitespace)
            {
                l.text.push(' ');
            }
            l.text.push_str(&g.text);
            let end = t
                .matrix
                .apply(Point::new(g.x + g.advance * t.h_scale, t.rise));
            last_end = Some(project(l.origin, l.dir, end).0);
        }
        l.end = l.end.max(project(l.origin, l.dir, t.end()).0);
        l.bounds = l.bounds.union(&t.bounds);
        l.shows.push(t);
    }
    out
}

/// Fasst Zeilen zu Absätzen zusammen.
pub fn group_paragraphs(it: &Interpreted, stamp: u64) -> Vec<Paragraph> {
    let lines = lines(it);
    // Absätze als Bereiche von Zeilenindizes.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let joins = groups.last().is_some_and(|g| {
            let first = &lines[g[0]];
            let prev = &lines[*g.last().expect("nicht leer")];
            if dot(l.dir, first.dir) < 0.999
                || l.shows[0].render_mode != first.shows[0].render_mode
                || l.em < first.em * 0.8
                || l.em > first.em * 1.25
            {
                return false;
            }
            // Abstand zur vorigen Grundlinie (nach unten positiv) und linker Rand.
            let (_, v) = project(prev.origin, prev.dir, l.origin);
            let step = -v;
            if step < 1.0 * first.em || step > 2.2 * first.em {
                return false;
            }
            if g.len() >= 2 {
                let (_, v1) = project(first.origin, first.dir, lines[g[1]].origin);
                if (step + v1).abs() > 0.15 * -v1 {
                    return false;
                }
            }
            // Linker Rand: ab der zweiten Zeile gleich; die erste darf eingerückt sein.
            let reference = if g.len() >= 2 { &lines[g[1]] } else { first };
            let (du, _) = project(reference.origin, reference.dir, l.origin);
            if g.len() >= 2 {
                du.abs() <= 0.6 * first.em
            } else {
                du.abs() <= 0.6 * first.em || (-du > 0.0 && -du <= 4.0 * first.em)
            }
        });
        if joins {
            groups.last_mut().expect("geprüft").push(i);
        } else {
            groups.push(vec![i]);
        }
    }

    groups
        .into_iter()
        .map(|g| {
            let first = &lines[g[0]];
            let show = first.shows[0];
            let dir = first.dir;
            let up = {
                let m = show.matrix.0;
                // Senkrecht zur Schreibrichtung, auf der Seite der y-Achse des Textraums.
                let n = Point::new(-dir.y, dir.x);
                if n.x * m[2] + n.y * m[3] >= 0.0 {
                    n
                } else {
                    Point::new(dir.y, -dir.x)
                }
            };
            // Linker Rand: kleinster Zeilenanfang (die erste Zeile kann eingerückt sein).
            let starts: Vec<f32> = g
                .iter()
                .map(|&i| project(first.origin, dir, lines[i].origin).0)
                .collect();
            let left = starts.iter().copied().fold(f32::INFINITY, f32::min);
            let origin = Point::new(first.origin.x + dir.x * left, first.origin.y + dir.y * left);
            let ends: Vec<f32> = g
                .iter()
                .map(|&i| {
                    let l = &lines[i];
                    project(first.origin, dir, l.origin).0 + l.end - left
                })
                .collect();
            let width = ends.iter().copied().fold(0.0, f32::max);
            let line_height = if g.len() >= 2 {
                -project(first.origin, dir, lines[g[1]].origin).1
            } else {
                1.2 * first.em
            };
            // Blocksatz: mindestens zwei volle Zeilen, die rechts (fast) gleich enden.
            let full = &ends[..ends.len() - 1];
            let justified =
                full.len() >= 2 && full.iter().all(|e| (width - e).abs() <= 0.25 * first.em);
            // Text: Zeilen verbinden, Trennstrich vor Kleinbuchstaben entfernen.
            let mut text = String::new();
            for &i in &g {
                let t = lines[i].text.trim();
                if text.ends_with('-')
                    && t.chars().next().is_some_and(char::is_lowercase)
                    && text.len() > 1
                {
                    text.pop();
                } else if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(t);
            }
            Paragraph {
                stamp,
                shows: g
                    .iter()
                    .flat_map(|&i| lines[i].shows.iter().map(|t| t.op))
                    .collect(),
                text,
                bounds: g
                    .iter()
                    .map(|&i| lines[i].bounds)
                    .reduce(|a, b| a.union(&b))
                    .unwrap_or(first.bounds),
                origin,
                indent: starts[0] - left,
                direction: dir,
                up,
                line_height,
                width,
                lines: g.len(),
                justified,
                font: show.font.clone(),
                font_size: show.font_size,
                size_on_page: first.em,
            }
        })
        .collect()
}

/// Abschnitt `/FcPara BMC … EMC`, der die Operation `op` enthält (Indizes von BMC und EMC).
fn marked_range(ops: &[Operation], op: usize) -> Option<(usize, usize)> {
    let start = (0..op).rev().find(|&i| {
        ops[i].operator == "BMC"
            && ops[i]
                .operands
                .first()
                .and_then(|o| o.as_name().ok())
                .is_some_and(|n| n == MARK)
    })?;
    // Passendes EMC suchen (verschachtelte BMC/BDC mitzählen).
    let mut depth = 0;
    for (i, o) in ops.iter().enumerate().skip(start) {
        match o.operator.as_str() {
            "BMC" | "BDC" => depth += 1,
            "EMC" => {
                depth -= 1;
                if depth == 0 {
                    return (i > op).then_some((start, i));
                }
            }
            _ => {}
        }
    }
    None
}

/// Teilt einen String aus Zeichencodes hinter jedem Leerzeichen-Code (`space`).
fn split_after_spaces(bytes: &[u8], space: &[u8]) -> Vec<Vec<u8>> {
    let step = space.len().max(1);
    let mut parts = vec![Vec::new()];
    for code in bytes.chunks(step) {
        parts
            .last_mut()
            .expect("nicht leer")
            .extend_from_slice(code);
        if code == space {
            parts.push(Vec::new());
        }
    }
    parts.retain(|p| !p.is_empty());
    parts
}

impl Document {
    /// Absätze einer Seite.
    pub fn paragraphs(&self, page_idx: usize) -> Result<Vec<Paragraph>> {
        let pc = self.page_content(page_idx)?;
        let page = self.page(page_idx)?.object_id;
        let it = self.interpret_content(page, &pc)?;
        Ok(group_paragraphs(&it, pc.stamp))
    }

    /// Setzt einen Absatz mit neuem Text in einen (neuen) Kasten. Ein Undo-Schritt.
    /// `\n` im Text erzwingt einen Zeilenumbruch.
    pub fn set_paragraph(
        &mut self,
        page_idx: usize,
        para: &Paragraph,
        text: &str,
        frame: ParaFrame,
    ) -> Result<()> {
        let page = self.page(page_idx)?.object_id;
        let before = self.page_content(page_idx)?;
        if before.stamp != para.stamp {
            return Err(CoreError::Other(
                "Der Seiteninhalt hat sich inzwischen geändert.".into(),
            ));
        }
        let it = self.interpret_content(page, &before)?;
        let show_of = |op: usize| {
            it.items.iter().find_map(|item| match item {
                Item::Text(t) if t.op == op => Some(t),
                _ => None,
            })
        };
        let first = para
            .shows
            .first()
            .and_then(|&op| show_of(op))
            .ok_or_else(|| CoreError::Other("Absatz passt nicht zum Seiteninhalt.".into()))?;

        // 1) Alte Zeilen entfernen: eigener Abschnitt ganz, sonst Verschiebungen statt Text.
        let range = marked_range(&it.ops, first.op);
        let mut ops = Vec::with_capacity(it.ops.len());
        for (i, op) in it.ops.iter().enumerate() {
            if range.is_some_and(|(a, b)| (a..=b).contains(&i)) {
                continue;
            }
            if para.shows.contains(&i) {
                if let Some(t) = show_of(i) {
                    ops.extend(shift_op(t.width, t.font_size, t.h_scale));
                }
                continue;
            }
            ops.push(op.clone());
        }
        let mut after = before.clone();
        after.content = Content { operations: ops }.encode()?;
        isolate(&mut after);

        // 2) Neu setzen.
        let new_ops = self.paragraph_ops(page, &mut after, &it, first, para, text, &frame)?;
        append_ops(&mut after, new_ops)?;
        self.execute(Box::new(ReplacePageContent::new(
            page,
            before,
            after,
            "Absatz bearbeiten",
        )))
    }

    /// Operationen für den neu gesetzten Absatz (umbrechen, kodieren, Blocksatz).
    #[allow(clippy::too_many_arguments)] // alles Teile desselben Vorgangs
    fn paragraph_ops(
        &self,
        page: lopdf::ObjectId,
        after: &mut PageContent,
        it: &Interpreted,
        first: &TextShow,
        para: &Paragraph,
        text: &str,
        frame: &ParaFrame,
    ) -> Result<Vec<Operation>> {
        let pdf = self.pdf();
        let resources = crate::content::page_resources(pdf, page);
        // Steht der Absatz schon in unserer Ersatzschrift (nur im Modell), gibt es keine
        // "Originalschrift": alles kommt aus der Teilmenge.
        let own = after.fonts.contains_key(&first.font);
        let enc = Encoder::new(pdf, resources, it, &first.font);
        let fallback = if own {
            first.font.clone()
        } else {
            fallback_name(after, resources, pdf)
        };
        let (size, th) = (first.font_size, first.h_scale);
        let m = first.matrix.0;
        let scale_x = length(m[0], m[1]).max(f32::EPSILON);
        let unit = (size * th).abs().max(f32::EPSILON);

        // Breite eines Zeichens im Textraum (wie `segment` sie rechnet). Das Platzhalterzeichen
        // U+E000 steht für die Einrückung der ersten Zeile.
        const INDENT: char = '\u{E000}';
        let indent_text = para.indent / scale_x;
        let char_w = |c: char, _: f32| -> f32 {
            if c == INDENT {
                return indent_text;
            }
            let key = c.to_string();
            match (!own).then(|| enc.map.get(&key)).flatten() {
                Some((bytes, code)) => {
                    let tw = if bytes.len() == 1 && *code == 32 {
                        first.word_spacing
                    } else {
                        0.0
                    };
                    (enc.font.width(*code) * size + first.char_spacing + tw) * th
                }
                None => (embedded_font::char_width(c, size) + first.char_spacing) * th,
            }
        };
        let max_text = frame.width.map(|w| w / scale_x);

        // Zeilen: (Text, letzte Zeile ihres Abschnitts?).
        let mut lines: Vec<(String, bool)> = Vec::new();
        for (k, piece) in text.split('\n').enumerate() {
            let wrapped = match max_text {
                Some(max) => {
                    let input = if k == 0 && para.indent > 0.0 {
                        format!("{INDENT}{piece}")
                    } else {
                        piece.to_string()
                    };
                    crate::font::wrap_with(&input, size, max, &char_w)
                        .into_iter()
                        .map(|l| l.replace(INDENT, ""))
                        .collect()
                }
                None => vec![piece.to_string()],
            };
            let n = wrapped.len();
            lines.extend(
                wrapped
                    .into_iter()
                    .enumerate()
                    .map(|(i, l)| (l, i + 1 == n)),
            );
        }

        let name = |n: &[u8]| Object::Name(n.to_vec());
        let mut ops = vec![
            Operation::new("BMC", vec![name(MARK)]),
            Operation::new("q", vec![]),
            Operation::new("BT", vec![]),
        ];
        ops.extend(first.fill.iter().cloned());
        ops.push(Operation::new("Tf", vec![name(&first.font), size.into()]));
        ops.push(Operation::new("Tc", vec![first.char_spacing.into()]));
        ops.push(Operation::new("Tw", vec![first.word_spacing.into()]));
        ops.push(Operation::new("Tz", vec![(th * 100.0).into()]));
        ops.push(Operation::new("Ts", vec![first.rise.into()]));
        ops.push(Operation::new("Tr", vec![first.render_mode.into()]));

        let format = if enc.font.is_two_byte() {
            StringFormat::Hexadecimal
        } else {
            StringFormat::Literal
        };
        let space = (!own)
            .then(|| enc.map.get(" ").map(|(b, _)| b.clone()))
            .flatten();
        let dir = para.direction;
        for (i, (line, last)) in lines.iter().enumerate() {
            let along = if i == 0 { para.indent } else { 0.0 };
            let down = i as f32 * para.line_height;
            // Grundlinie dieser Zeile; Tm ohne die Hochstellung (die setzt `Ts`).
            let o = Point::new(
                frame.origin.x + dir.x * along - para.up.x * down - m[2] * first.rise,
                frame.origin.y + dir.y * along - para.up.y * down - m[3] * first.rise,
            );
            ops.push(Operation::new(
                "Tm",
                vec![
                    m[0].into(),
                    m[1].into(),
                    m[2].into(),
                    m[3].into(),
                    o.x.into(),
                    o.y.into(),
                ],
            ));
            let (segs, width) = segment(&enc, line, first)?;
            // Blocksatz: Restbreite gleichmäßig auf die Leerzeichen verteilen.
            let spaces = line.chars().filter(|&c| c == ' ').count();
            let extra = match max_text {
                Some(max) if frame.justify && !last && spaces > 0 && width < max => {
                    (max - width - if i == 0 { indent_text } else { 0.0 }).max(0.0) / spaces as f32
                }
                _ => 0.0,
            };
            let gap = Object::Real(-extra * 1000.0 / unit);
            for seg in segs {
                match seg {
                    Segment::Original(parts) => {
                        let mut arr = Vec::new();
                        for part in parts {
                            match part {
                                Object::String(bytes, _) => match &space {
                                    Some(sp) if extra > 0.0 => {
                                        for chunk in split_after_spaces(&bytes, sp) {
                                            let ends = chunk.ends_with(sp);
                                            arr.push(Object::String(chunk, format));
                                            if ends {
                                                arr.push(gap.clone());
                                            }
                                        }
                                    }
                                    _ => arr.push(Object::String(bytes, format)),
                                },
                                // Leerzeichen als Abstand (Schrift ohne Leerzeichen-Glyphe).
                                Object::Real(n) => {
                                    arr.push(Object::Real(n - extra * 1000.0 / unit))
                                }
                                other => arr.push(other),
                            }
                        }
                        ops.push(Operation::new("TJ", vec![Object::Array(arr)]));
                    }
                    Segment::Fallback(t) => {
                        let sub = after.fonts.entry(fallback.clone()).or_default();
                        if !own {
                            ops.push(Operation::new("Tf", vec![name(&fallback), size.into()]));
                        }
                        let mut arr = Vec::new();
                        for piece in t.split_inclusive(' ') {
                            arr.push(sub.encode_object(piece));
                            if extra > 0.0 && piece.ends_with(' ') {
                                arr.push(gap.clone());
                            }
                        }
                        ops.push(Operation::new("TJ", vec![Object::Array(arr)]));
                        if !own {
                            ops.push(Operation::new("Tf", vec![name(&first.font), size.into()]));
                        }
                    }
                }
            }
        }
        ops.push(Operation::new("ET", vec![]));
        ops.push(Operation::new("Q", vec![]));
        ops.push(Operation::new("EMC", vec![]));
        Ok(ops)
    }
}
