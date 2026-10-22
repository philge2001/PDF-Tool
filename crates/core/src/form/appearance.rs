//! Darstellung von Formularfeldern: Layout (Schriftgröße, Zeilen) und Appearance Streams.
//!
//! Das Layout wird hier einmal berechnet und von zwei Stellen benutzt: vom Speichern
//! (Appearance Stream in der Datei) und von der App (Overlay beim Ausfüllen). So sehen
//! Feld auf dem Bildschirm und Feld in der Datei gleich aus.
//!
//! Koordinaten sind lokal zum Widget: (0, 0) = linke untere Ecke von /Rect, so wie es
//! /BBox [0 0 Breite Höhe] im Appearance Stream verlangt.

use super::{Align, FieldKind, FieldValue, FormField, Widget};
use crate::annotation::appearance::{ellipse, helvetica_dict, n, pdf_literal};
use crate::font;
use crate::geometry::{Color, Rect};
use lopdf::{Stream, dictionary};
use std::fmt::Write as _;

/// Abstand zwischen Rahmen und Text.
pub const PADDING: f32 = 2.0;
/// Zeilenabstand als Vielfaches der Schriftgröße.
pub const LINE_HEIGHT: f32 = 1.15;
/// Rahmen von Kontrollkästchen/Optionsfeldern, die selbst keinen festlegen.
pub const FALLBACK_BORDER: Color = Color::rgb(0.43, 0.43, 0.43);
/// Hintergrund markierter Listeneinträge.
pub const SELECTION_COLOR: Color = Color::rgb(0.6, 0.75, 0.9);

/// Eine Textzeile, fertig positioniert (`x` = linker Rand, `baseline` = Grundlinie).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub x: f32,
    pub baseline: f32,
    /// Listenfeld: Eintrag ist ausgewählt (Hintergrund wird markiert).
    pub selected: bool,
}

/// Ergebnis des Layouts für Text-, Kombinations- und Listenfelder.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    pub font_size: f32,
    pub lines: Vec<Line>,
}

impl TextLayout {
    /// Höhe einer Zeile (für die Markierung im Listenfeld).
    pub fn line_height(&self) -> f32 {
        self.font_size * LINE_HEIGHT
    }
}

/// Innenbereich (ohne Rahmen und Abstand) in lokalen Koordinaten.
fn inner(w: &Widget) -> Rect {
    let inset = PADDING
        + if w.border.is_some() {
            w.border_width
        } else {
            0.0
        };
    let (bw, bh) = (w.rect.width(), w.rect.height());
    Rect::new(
        inset,
        inset,
        (bw - inset).max(inset),
        (bh - inset).max(inset),
    )
}

fn aligned_x(field: &FormField, area: Rect, text: &str, size: f32) -> f32 {
    let tw = font::text_width(text, size);
    match field.align {
        Align::Left => area.x0,
        Align::Center => area.x0 + (area.width() - tw) / 2.0,
        Align::Right => area.x1 - tw,
    }
}

/// Grundlinie, bei der eine einzelne Zeile vertikal mittig sitzt. 0,7 ≈ Versalhöhe von
/// Helvetica relativ zur Schriftgröße – so wirkt der Text optisch zentriert.
fn centered_baseline(area: Rect, size: f32) -> f32 {
    area.y0 + (area.height() - size * 0.7) / 2.0
}

/// Der Text, den ein Feld anzeigt (Passwörter als Sternchen, Auswahl als Anzeigetext).
pub fn display_text(field: &FormField) -> String {
    match (&field.kind, &field.value) {
        (FieldKind::Text { password: true, .. }, FieldValue::Text(t)) => {
            "*".repeat(t.chars().count())
        }
        (FieldKind::Combo { .. }, FieldValue::Choice(v)) => v
            .first()
            .map(|e| field.display_of(e).to_string())
            .unwrap_or_default(),
        (_, v) => v.as_text(),
    }
}

/// Layout von Text-, Kombinations- und Listenfeldern; `None` für Schalter.
pub fn layout(field: &FormField, widget: &Widget) -> Option<TextLayout> {
    let area = inner(widget);
    match &field.kind {
        FieldKind::Text {
            multiline: true, ..
        } => Some(layout_multiline(field, area)),
        FieldKind::Text { .. } | FieldKind::Combo { .. } => {
            let text = display_text(field);
            let mut size = field.font_size;
            if size <= 0.0 {
                // Automatisch: so groß, wie die Höhe erlaubt (max. 12 pt), notfalls kleiner,
                // damit der Text in die Breite passt.
                size = (area.height() * 0.8).clamp(4.0, 12.0);
                let tw = font::text_width(&text, size);
                if tw > area.width() && tw > 0.0 {
                    size = (size * area.width() / tw).max(4.0);
                }
            }
            let line = Line {
                x: aligned_x(field, area, &text, size),
                baseline: centered_baseline(area, size),
                text,
                selected: false,
            };
            Some(TextLayout {
                font_size: size,
                lines: vec![line],
            })
        }
        FieldKind::List { .. } => Some(layout_list(field, area)),
        _ => None,
    }
}

fn layout_multiline(field: &FormField, area: Rect) -> TextLayout {
    let text = display_text(field);
    let fits = |size: f32| {
        let lines = font::wrap(&text, size, area.width());
        let ok = lines.len() as f32 * size * LINE_HEIGHT <= area.height();
        (lines, ok)
    };
    let (size, lines) = if field.font_size > 0.0 {
        (field.font_size, fits(field.font_size).0)
    } else {
        // Automatisch: von 12 pt in halben Punkten verkleinern, bis alles hineinpasst.
        let mut size = 12.0;
        loop {
            let (lines, ok) = fits(size);
            if ok || size <= 4.0 {
                break (size, lines);
            }
            size -= 0.5;
        }
    };
    let lh = size * LINE_HEIGHT;
    let lines = lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| Line {
            x: aligned_x(field, area, &text, size),
            // Erste Grundlinie eine Oberlänge (≈ Schriftgröße) unter dem oberen Rand.
            baseline: area.y1 - size * 0.9 - i as f32 * lh,
            text,
            selected: false,
        })
        .collect();
    TextLayout {
        font_size: size,
        lines,
    }
}

fn layout_list(field: &FormField, area: Rect) -> TextLayout {
    let size = if field.font_size > 0.0 {
        field.font_size
    } else {
        10.0_f32.min(area.height() / LINE_HEIGHT).max(4.0)
    };
    let lh = size * LINE_HEIGHT;
    let visible = ((area.height() / lh).floor() as usize).max(1);
    let selected: &[String] = match &field.value {
        FieldValue::Choice(v) => v,
        _ => &[],
    };
    // Erster ausgewählter Eintrag soll sichtbar sein.
    let first = field
        .options
        .iter()
        .position(|o| selected.contains(&o.export))
        .unwrap_or(0);
    let top = (first + 1).saturating_sub(visible);
    let lines = field
        .options
        .iter()
        .skip(top)
        .take(visible)
        .enumerate()
        .map(|(i, o)| Line {
            text: o.display.clone(),
            x: aligned_x(field, area, &o.display, size),
            baseline: area.y1 - (i as f32 + 1.0) * lh + size * 0.25,
            selected: selected.contains(&o.export),
        })
        .collect();
    TextLayout {
        font_size: size,
        lines,
    }
}

/// Erscheinungsbild eines Widgets.
pub enum WidgetAppearance {
    /// Text-, Kombinations-, Listenfeld: ein Stream.
    Single(Stream),
    /// Kontrollkästchen/Optionsfeld: je ein Stream für "an" (unter `on_state`) und "Off".
    States {
        on_state: String,
        on: Stream,
        off: Stream,
    },
}

fn rgb(c: Color) -> String {
    format!("{} {} {}", n(c.r), n(c.g), n(c.b))
}

/// Hintergrund und Rahmen; bei Optionsfeldern rund.
fn frame(s: &mut String, w: &Widget, round: bool) {
    let (bw, bh) = (w.rect.width(), w.rect.height());
    let full = Rect::new(0.0, 0.0, bw, bh);
    if let Some(bg) = w.background {
        let _ = writeln!(s, "{} rg", rgb(bg));
        if round {
            ellipse(s, full);
            s.push_str("f\n");
        } else {
            let _ = writeln!(s, "0 0 {} {} re f", n(bw), n(bh));
        }
    }
    if let Some(bc) = w.border {
        let lw = w.border_width.max(0.0);
        if lw > 0.0 {
            let _ = writeln!(s, "{} RG {} w", rgb(bc), n(lw));
            let half = lw / 2.0;
            if round {
                ellipse(s, full.expand(-half));
                s.push_str("S\n");
            } else {
                let _ = writeln!(
                    s,
                    "{} {} {} {} re S",
                    n(half),
                    n(half),
                    n(bw - lw),
                    n(bh - lw)
                );
            }
        }
    }
}

fn form_stream(widget: &Widget, content: String, with_font: bool) -> Stream {
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => vec![0.into(), 0.into(), widget.rect.width().into(), widget.rect.height().into()],
    };
    if with_font {
        dict.set(
            "Resources",
            dictionary! { "Font" => dictionary! { "Helv" => helvetica_dict() } },
        );
    }
    Stream::new(dict, content.into_bytes())
}

/// Erzeugt das Erscheinungsbild für ein Widget; `None` für Schaltflächen/Signaturen.
pub fn build(field: &FormField, widget: &Widget) -> Option<WidgetAppearance> {
    match field.kind {
        FieldKind::CheckBox | FieldKind::Radio => {
            let round = field.kind == FieldKind::Radio;
            let mut off = String::new();
            frame(&mut off, widget, round);
            if widget.border.is_none() {
                // Ohne eigenen Rahmen wäre nur der Haken sichtbar, nicht das Kästchen selbst.
                // Gleicher Ersatzrahmen wie im Overlay der App.
                let mut fallback = widget.clone();
                fallback.background = None;
                fallback.border = Some(FALLBACK_BORDER);
                fallback.border_width = 1.0;
                frame(&mut off, &fallback, round);
            }
            let mut on = off.clone();
            symbol(&mut on, field, widget);
            Some(WidgetAppearance::States {
                on_state: widget.on_state.clone().unwrap_or_else(|| "Yes".into()),
                on: form_stream(widget, on, false),
                off: form_stream(widget, off, false),
            })
        }
        FieldKind::PushButton | FieldKind::Signature => None,
        _ => {
            let layout = layout(field, widget)?;
            let mut s = String::new();
            frame(&mut s, widget, false);
            let area = inner(widget);
            // Markierte Content-Sequenz /Tx: Konvention für den variablen Teil eines Felds.
            s.push_str("/Tx BMC\nq\n");
            let _ = writeln!(
                s,
                "{} {} {} {} re W n",
                n(area.x0 - 1.0),
                n(area.y0 - 1.0),
                n(area.width() + 2.0),
                n(area.height() + 2.0)
            );
            let lh = layout.line_height();
            for l in layout.lines.iter().filter(|l| l.selected) {
                let _ = writeln!(
                    s,
                    "{} rg {} {} {} {} re f",
                    rgb(SELECTION_COLOR),
                    n(area.x0 - 1.0),
                    n(l.baseline - layout.font_size * 0.25),
                    n(area.width() + 2.0),
                    n(lh)
                );
            }
            let _ = writeln!(
                s,
                "BT /Helv {} Tf {} rg",
                n(layout.font_size),
                rgb(field.text_color)
            );
            // Td verschiebt relativ zur vorigen Zeile – deshalb die Differenzen mitführen.
            let (mut px, mut py) = (0.0, 0.0);
            for l in &layout.lines {
                let _ = writeln!(
                    s,
                    "{} {} Td {} Tj",
                    n(l.x - px),
                    n(l.baseline - py),
                    pdf_literal(&font::winansi_encode(&l.text))
                );
                (px, py) = (l.x, l.baseline);
            }
            s.push_str("ET\nQ\nEMC\n");
            Some(WidgetAppearance::Single(form_stream(widget, s, true)))
        }
    }
}

/// Häkchen bzw. Punkt im eingeschalteten Zustand, in der Textfarbe des Felds.
fn symbol(s: &mut String, field: &FormField, widget: &Widget) {
    let (bw, bh) = (widget.rect.width(), widget.rect.height());
    let side = bw.min(bh);
    let (ox, oy) = ((bw - side) / 2.0, (bh - side) / 2.0);
    let c = rgb(field.text_color);
    if field.kind == FieldKind::Radio {
        let r = side * 0.25;
        let (cx, cy) = (bw / 2.0, bh / 2.0);
        let _ = writeln!(s, "{c} rg");
        ellipse(s, Rect::new(cx - r, cy - r, cx + r, cy + r));
        s.push_str("f\n");
    } else {
        let p = |fx: f32, fy: f32| format!("{} {}", n(ox + fx * side), n(oy + fy * side));
        let _ = writeln!(
            s,
            "{c} RG {} w 1 J 1 j {} m {} l {} l S",
            n(side * 0.12),
            p(0.22, 0.52),
            p(0.42, 0.28),
            p(0.8, 0.76)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::FieldId;

    fn text_field(value: &str, size: f32, multiline: bool) -> (FormField, Widget) {
        let w = Widget::new((1, 0), Rect::new(100.0, 100.0, 300.0, 120.0));
        let f = FormField {
            id: FieldId(1),
            name: "t".into(),
            tooltip: String::new(),
            kind: FieldKind::Text {
                multiline,
                password: false,
                max_len: None,
            },
            value: FieldValue::Text(value.into()),
            default: FieldValue::Text(String::new()),
            read_only: false,
            required: false,
            options: Vec::new(),
            font_size: size,
            text_color: Color::BLACK,
            align: Align::Left,
            widgets: vec![w.clone()],
            source: None,
        };
        (f, w)
    }

    #[test]
    fn single_line_auto_size_shrinks_long_text() {
        let (f, w) = text_field("kurz", 0.0, false);
        let short = layout(&f, &w).unwrap();
        assert_eq!(short.font_size, 12.0);
        let (f, w) = text_field(&"sehr lang ".repeat(10), 0.0, false);
        let long = layout(&f, &w).unwrap();
        assert!(long.font_size < 12.0);
        let tw = font::text_width(&long.lines[0].text, long.font_size);
        assert!(tw <= w.rect.width() - 2.0 * PADDING + 0.01);
    }

    #[test]
    fn multiline_wraps_and_fits() {
        let (mut f, mut w) = text_field(&"Wort ".repeat(40), 0.0, true);
        w.rect = Rect::new(0.0, 0.0, 200.0, 60.0);
        f.widgets = vec![w.clone()];
        let l = layout(&f, &w).unwrap();
        assert!(l.lines.len() > 1);
        assert!(l.lines.last().unwrap().baseline >= 0.0);
    }

    #[test]
    fn text_stream_contains_value_and_font() {
        let (f, w) = text_field("Grüße", 11.0, false);
        let Some(WidgetAppearance::Single(s)) = build(&f, &w) else {
            panic!()
        };
        let text = String::from_utf8_lossy(&s.content).into_owned();
        assert!(text.contains("/Helv 11 Tf"), "{text}");
        assert!(text.contains(r"(Gr\374\337e) Tj"), "{text}");
        assert!(text.starts_with("/Tx BMC"));
    }
}
