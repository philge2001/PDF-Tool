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
//! geschrieben – als formatierte Läufe (`rich_text`): Jede Textausgabe behält ihre Schrift,
//! Größe und Farbe (ein fettes Wort bleibt fett), Richtung und Zeilenabstand kommen von der
//! ersten Zeile, umbrochen wird auf die Breite des Kastens. Fehlende Zeichen kommen aus der
//! passenden Systemschrift, sonst aus Noto Sans. Der neue Absatz steht in
//! `/FcPara BMC … EMC` – beim nächsten Bearbeiten wird dieser Abschnitt komplett ersetzt,
//! statt weitere leere Reste anzuhäufen.

use crate::content::{Interpreted, Item, TextShow};
use crate::document::Document;
use crate::error::{CoreError, Result};
use crate::geometry::{Point, Rect};
use crate::rich_text::{
    Base, FontRef, Layout, Resolver, StyledRun, TextStyle, color_from_fill, layout_ops, normalize,
};
use crate::text_edit::{ReplacePageContent, append_ops, isolate, length, project, shift_op};
use lopdf::Object;
use lopdf::content::{Content, Operation};

/// Name des Abschnitts, in dem PDF Tool neu gesetzte Absätze ablegt.
const MARK: &[u8] = b"FcPara";

/// Markierung `/FcBr MP` hinter einer Zeile, die mit einem harten Umbruch (Enter) endet. In der
/// Datei stehen sonst nur einzeln platzierte Zeilen – ohne sie würde ein neu gesetzter Absatz
/// beim nächsten Umbruch (z. B. nach dem Ändern der Breite) zu einem Fließtext zusammenlaufen.
pub(crate) const BREAK_MARK: &[u8] = b"FcBr";

/// Ist `op` eine Umbruch-Markierung (`/FcBr MP`)?
pub(crate) fn is_break_mark(op: &Operation) -> bool {
    op.operator == "MP"
        && op
            .operands
            .first()
            .and_then(|o| o.as_name().ok())
            .is_some_and(|n| n == BREAK_MARK)
}

/// Ein erkannter Absatz.
#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    /// Stempel des Inhalts, aus dem er stammt.
    pub stamp: u64,
    /// Indizes aller Textausgabe-Operationen des Absatzes.
    pub shows: Vec<usize>,
    /// Text, Zeilen mit Leerzeichen verbunden (Trennstriche am Zeilenende entfernt); harte
    /// Umbrüche als `\n` (siehe `breaks`).
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
    /// Je Zeile die Zahl harter Umbrüche davor (`breaks[0]` = 0; 2 = Leerzeile dazwischen).
    pub breaks: Vec<usize>,
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

    /// Zeilen einschließlich Leerzeilen aus harten Umbrüchen (für die Höhe des Kastens).
    pub fn rows(&self) -> usize {
        self.lines
            + self
                .breaks
                .iter()
                .map(|b| b.saturating_sub(1))
                .sum::<usize>()
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
    /// Erste und letzte Operation der Zeile.
    first_op: usize,
    last_op: usize,
    /// Ende des ersten Worts entlang der Richtung (ab `origin`), falls die Zeile mehrere hat.
    first_word: Option<f32>,
}

impl Line<'_> {
    /// Breite des ersten Worts (bei nur einem Wort die ganze Zeile).
    fn first_word(&self) -> f32 {
        self.first_word.unwrap_or(self.end)
    }
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
                first_op: t.op,
                last_op: t.op,
                first_word: None,
            });
        }
        let Some(l) = out.last_mut() else { continue };
        // Leerzeichen für Lücken (wie bei den Textblöcken).
        let mut last_end = if joins { Some(l.end) } else { None };
        for g in &t.glyphs {
            let start = project(l.origin, l.dir, t.matrix.apply(Point::new(g.x, t.rise))).0;
            let gap = last_end.is_some_and(|e| start - e > 0.25 * em);
            if gap
                && !l.text.ends_with(char::is_whitespace)
                && !g.text.starts_with(char::is_whitespace)
            {
                l.text.push(' ');
            }
            // Erstes Wort endet vor dem ersten Leerzeichen bzw. der ersten Lücke.
            if l.first_word.is_none()
                && (gap || g.text.starts_with(char::is_whitespace))
                && !l.text.trim().is_empty()
            {
                l.first_word = last_end;
            }
            l.text.push_str(&g.text);
            let end = t
                .matrix
                .apply(Point::new(g.x + g.advance * t.h_scale, t.rise));
            last_end = Some(project(l.origin, l.dir, end).0);
        }
        l.end = l.end.max(project(l.origin, l.dir, t.end()).0);
        l.bounds = l.bounds.union(&t.bounds);
        l.first_op = l.first_op.min(t.op);
        l.last_op = l.last_op.max(t.op);
        l.shows.push(t);
    }
    out
}

/// Je Zeile die Zahl der Umbruch-Markierungen zwischen ihr und der vorigen Zeile.
fn break_marks(it: &Interpreted, lines: &[Line]) -> Vec<usize> {
    let mut out = vec![0; lines.len()];
    for i in 1..lines.len() {
        let (a, b) = (lines[i - 1].last_op, lines[i].first_op);
        if a < b {
            out[i] = it.ops[a + 1..b].iter().filter(|o| is_break_mark(o)).count();
        }
    }
    out
}

/// Fasst Zeilen zu Absätzen zusammen.
pub fn group_paragraphs(it: &Interpreted, stamp: u64) -> Vec<Paragraph> {
    let lines = lines(it);
    let marks = break_marks(it, &lines);
    // Abstand zur vorigen Grundlinie (nach unten positiv), geteilt durch die Zahl der Zeilen
    // dazwischen: Leerzeilen aus harten Umbrüchen (`\n\n`) gehören zum Absatz.
    let step = |i: usize| {
        let prev = &lines[i - 1];
        -project(prev.origin, prev.dir, lines[i].origin).1 / marks[i].max(1) as f32
    };
    // Absätze als Bereiche von Zeilenindizes.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let joins = groups.last().is_some_and(|g| {
            let first = &lines[g[0]];
            if dot(l.dir, first.dir) < 0.999
                || l.shows[0].render_mode != first.shows[0].render_mode
                || l.em < first.em * 0.8
                || l.em > first.em * 1.25
            {
                return false;
            }
            // Zeilenabstand (gleichmäßig) und linker Rand.
            let s = step(i);
            if s < 1.0 * first.em || s > 2.2 * first.em {
                return false;
            }
            if g.len() >= 2 {
                let s1 = step(g[1]);
                if (s - s1).abs() > 0.15 * s1 {
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
                step(g[1])
            } else {
                1.2 * first.em
            };
            // Harte Umbrüche: markiert, sonst wenn das erste Wort der Zeile noch in die vorige
            // gepasst hätte (fremde PDFs: Adressen, Aufzählungen ohne Zeichen, Gedichte).
            let breaks: Vec<usize> = g
                .iter()
                .enumerate()
                .map(|(k, &i)| {
                    if k == 0 {
                        0
                    } else if marks[i] > 0 {
                        marks[i]
                    } else {
                        let fits = ends[k - 1] + 0.28 * first.em + lines[i].first_word()
                            < width - 0.1 * first.em;
                        usize::from(fits)
                    }
                })
                .collect();
            // Blocksatz: mindestens zwei volle Zeilen, die rechts (fast) gleich enden.
            let full = &ends[..ends.len() - 1];
            let justified =
                full.len() >= 2 && full.iter().all(|e| (width - e).abs() <= 0.25 * first.em);
            // Text: Zeilen verbinden, Trennstrich vor Kleinbuchstaben entfernen.
            let mut text = String::new();
            for (k, &i) in g.iter().enumerate() {
                let t = lines[i].text.trim();
                if breaks[k] > 0 {
                    text.push_str(&"\n".repeat(breaks[k]));
                } else if text.ends_with('-')
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
                breaks,
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

impl Document {
    /// Absätze einer Seite.
    pub fn paragraphs(&self, page_idx: usize) -> Result<Vec<Paragraph>> {
        let pc = self.page_content(page_idx)?;
        let page = self.page(page_idx)?.object_id;
        let it = self.interpret_content(page, &pc)?;
        Ok(group_paragraphs(&it, pc.stamp))
    }

    /// Setzt einen Absatz mit neuem (unformatiertem) Text in einen (neuen) Kasten. Ein
    /// Undo-Schritt. Ist der Text unverändert, bleibt die Formatierung aller Läufe erhalten;
    /// sonst gilt die Formatierung des ersten Laufs. `\n` erzwingt einen Zeilenumbruch.
    pub fn set_paragraph(
        &mut self,
        page_idx: usize,
        para: &Paragraph,
        text: &str,
        frame: ParaFrame,
    ) -> Result<()> {
        let runs = self.paragraph_runs(page_idx, para)?;
        let runs = if text == para.text || crate::rich_text::plain_text(&runs) == text {
            runs
        } else {
            let style = runs
                .first()
                .map(|r| r.style.clone())
                .ok_or_else(|| CoreError::Other("Absatz passt nicht zum Seiteninhalt.".into()))?;
            vec![StyledRun {
                text: text.to_string(),
                style,
            }]
        };
        self.set_paragraph_rich(page_idx, para, &runs, frame)
    }

    /// Formatierte Läufe eines Absatzes: je Textausgabe Schrift, Größe, Farbe, fett/kursiv
    /// (aus dem Schriftnamen, nachgeahmtes Fett über `Tr 2`). Zeilen werden mit Leerzeichen
    /// verbunden, Trennstriche am Zeilenende entfernt – wie `Paragraph::text`.
    pub fn paragraph_runs(&self, page_idx: usize, para: &Paragraph) -> Result<Vec<StyledRun>> {
        let page = self.page(page_idx)?.object_id;
        let pc = self.page_content(page_idx)?;
        if pc.stamp != para.stamp {
            return Err(CoreError::Other(
                "Der Seiteninhalt hat sich inzwischen geändert.".into(),
            ));
        }
        let it = self.interpret_content(page, &pc)?;
        let resources = crate::content::page_resources(self.pdf(), page);
        let mut resolver = Resolver::new(self.pdf(), resources, &it, &pc);
        let style_of = |t: &TextShow, resolver: &mut Resolver| -> TextStyle {
            let m = t.matrix.0;
            let (bold, italic) = match pc.fonts.get(&t.font) {
                Some(sub) => (sub.font().bold, sub.font().italic),
                None => {
                    let f = crate::content::load_font(self.pdf(), resources, None, &t.font);
                    let (_, b, i) = crate::system_fonts::parse_base_font(&f.base_font);
                    (b, i)
                }
            };
            let _ = resolver.family_of(&t.font);
            // Geneigte Textmatrix (nachgeahmte Kursive): Spalten nicht senkrecht.
            let skew =
                (m[0] * m[2] + m[1] * m[3]).abs() > 0.1 * length(m[0], m[1]) * length(m[2], m[3]);
            let font = match pc.fonts.get(&t.font) {
                Some(sub) if sub.font().is_noto() => FontRef::Noto,
                Some(sub) => FontRef::Family(sub.font().family.clone()),
                None => FontRef::Original(t.font.clone()),
            };
            TextStyle {
                font,
                size: t.font_size.abs() * length(m[2], m[3]),
                bold: bold || t.render_mode == 2,
                italic: italic || skew,
                underline: false,
                strike: false,
                color: color_from_fill(&t.fill),
            }
        };
        let mut runs: Vec<StyledRun> = Vec::new();
        for (li, l) in lines(&it)
            .iter()
            .filter(|l| l.shows.iter().all(|t| para.shows.contains(&t.op)))
            .enumerate()
        {
            // Zeilen verbinden: harter Umbruch als `\n`, Trennstrich vor Kleinbuchstaben weg,
            // sonst Leerzeichen.
            let hard = para.breaks.get(li).copied().unwrap_or(0);
            if li > 0 && hard > 0 {
                if let Some(r) = runs.last_mut() {
                    let t = r.text.trim_end().to_string();
                    r.text = t + &"\n".repeat(hard);
                }
            } else if li > 0 {
                let next_lower = l
                    .text
                    .trim_start()
                    .chars()
                    .next()
                    .is_some_and(char::is_lowercase);
                let prev = runs.last_mut();
                match prev {
                    Some(r) if r.text.trim_end().ends_with('-') && next_lower => {
                        let t = r.text.trim_end().trim_end_matches('-').to_string();
                        r.text = t;
                    }
                    Some(r) => {
                        let t = r.text.trim_end().to_string();
                        r.text = t + " ";
                    }
                    None => {}
                }
            }
            let mut last_end: Option<f32> = None;
            for t in &l.shows {
                let style = style_of(t, &mut resolver);
                let em = style.size;
                let mut text = String::new();
                for g in &t.glyphs {
                    let start = project(l.origin, l.dir, t.matrix.apply(Point::new(g.x, t.rise))).0;
                    let at_start = text.is_empty()
                        && runs
                            .last()
                            .is_none_or(|r| r.text.ends_with(char::is_whitespace));
                    if let Some(e) = last_end
                        && start - e > 0.25 * em
                        && !text.ends_with(char::is_whitespace)
                        && !at_start
                        && !g.text.starts_with(char::is_whitespace)
                    {
                        text.push(' ');
                    }
                    text.push_str(&g.text);
                    let end = t
                        .matrix
                        .apply(Point::new(g.x + g.advance * t.h_scale, t.rise));
                    last_end = Some(project(l.origin, l.dir, end).0);
                }
                if li == 0 && runs.is_empty() {
                    text = text.trim_start().to_string();
                }
                runs.push(StyledRun { text, style });
            }
        }
        if let Some(r) = runs.last_mut() {
            let t = r.text.trim_end().to_string();
            r.text = t;
        }
        Ok(normalize(runs))
    }

    /// Setzt einen Absatz aus formatierten Läufen in einen (neuen) Kasten. Ein Undo-Schritt.
    pub fn set_paragraph_rich(
        &mut self,
        page_idx: usize,
        para: &Paragraph,
        runs: &[StyledRun],
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
        // Umbruch-Markierungen zwischen den alten Zeilen fallen mit weg.
        let span = para.shows.iter().min().zip(para.shows.iter().max());
        let mut ops = Vec::with_capacity(it.ops.len());
        for (i, op) in it.ops.iter().enumerate() {
            if range.is_some_and(|(a, b)| (a..=b).contains(&i)) {
                continue;
            }
            if span.is_some_and(|(&a, &b)| (a..=b).contains(&i)) && is_break_mark(op) {
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

        // 2) Neu setzen (Geometrie der ersten Zeile; Hochstellung bleibt weg).
        let resources = crate::content::page_resources(self.pdf(), page);
        let line_factor = if para.size_on_page > 0.0 {
            para.line_height / para.size_on_page
        } else {
            1.2
        };
        let base = Base::from_show(first, para.direction, para.up, line_factor, para.indent);
        let mut resolver = Resolver::new(self.pdf(), resources, &it, &before);
        let layout = Layout {
            origin: frame.origin,
            width: frame.width,
            justify: frame.justify,
        };
        let body = layout_ops(&mut resolver, &mut after, &base, runs, &layout)?;
        let name = |n: &[u8]| Object::Name(n.to_vec());
        let mut new_ops = vec![
            Operation::new("BMC", vec![name(MARK)]),
            Operation::new("q", vec![]),
        ];
        new_ops.extend(body);
        new_ops.push(Operation::new("Q", vec![]));
        new_ops.push(Operation::new("EMC", vec![]));
        append_ops(&mut after, new_ops)?;
        self.execute(Box::new(ReplacePageContent::new(
            page,
            before,
            after,
            "Absatz bearbeiten",
        )))
    }

    /// Fügt neuen, formatierten Text hinzu. `origin` = Anfang der ersten Grundlinie
    /// (PDF-Benutzerraum); der Text steht in der Anzeige aufrecht (auch auf gedrehten Seiten).
    /// `width` = Umbruchbreite (pt), `None` = nur bei `\n` umbrechen.
    pub fn add_rich_text(
        &mut self,
        page_idx: usize,
        origin: Point,
        runs: &[StyledRun],
        width: Option<f32>,
    ) -> Result<()> {
        let info = self.page(page_idx)?;
        let (page, rotation) = (info.object_id, info.rotation);
        let before = self.page_content(page_idx)?;
        let it = self.interpret_content(page, &before)?;
        let mut after = before.clone();
        isolate(&mut after);
        let resources = crate::content::page_resources(self.pdf(), page);
        let mut resolver = Resolver::new(self.pdf(), resources, &it, &before);
        let layout = Layout {
            origin,
            width,
            justify: false,
        };
        let body = layout_ops(
            &mut resolver,
            &mut after,
            &Base::upright(rotation),
            runs,
            &layout,
        )?;
        let mut ops = vec![Operation::new("q", vec![])];
        ops.extend(body);
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
