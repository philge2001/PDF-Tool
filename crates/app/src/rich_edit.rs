//! Editor für formatierten Text (Phase 11): ein egui-Textfeld, dessen Zeichen je einen Stil
//! haben (Schrift, Größe, fett, kursiv, unterstrichen, durchgestrichen, Farbe), dazu die
//! Formatleiste. Markierter Text wird umformatiert; ohne Markierung gilt der Stil für die
//! nächsten getippten Zeichen.
//!
//! Die Anzeige im Editor ist eine Annäherung (Noto Sans, fett über eine fette Systemschrift,
//! falls vorhanden); wie es wirklich aussieht, zeigt die Seite nach dem Übernehmen.

use eframe::egui::{self, Color32, FontFamily, FontId, RichText, Stroke, text::LayoutJob};
use pdftool_core::rich_text::{FontRef, StyledRun, TextStyle, normalize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// egui-Familie für fette Anzeige (eine fette Systemschrift, sonst Noto Sans).
pub const BOLD_FAMILY: &str = "pdftool-bold";

/// Lädt einmalig eine fette Systemschrift für die Anzeige im Editor.
pub fn ensure_bold_font(ctx: &egui::Context) {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let family = FontFamily::Name(BOLD_FAMILY.into());
    let insert = |name: &str, data: egui::FontData, priority| {
        ctx.add_font(egui::epaint::text::FontInsert::new(
            name,
            data,
            vec![egui::epaint::text::InsertFontFamily {
                family: family.clone(),
                priority,
            }],
        ));
    };
    let candidates = [
        "Noto Sans",
        "Segoe UI",
        "Arial",
        "Liberation Sans",
        "DejaVu Sans",
        "Helvetica",
    ];
    let bold = candidates.iter().find_map(|f| {
        pdftool_core::system_fonts::find_style(f, true, false)
            .filter(|s| s.bold && !s.italic)
            .and_then(pdftool_core::system_fonts::load)
    });
    if let Some(b) = bold {
        let mut data = egui::FontData::from_owned(b.bytes().to_vec());
        data.index = b.index;
        insert(
            "pdftool-bold-face",
            data,
            egui::epaint::text::FontPriority::Highest,
        );
    }
    // Fallback für Zeichen, die der fetten Schrift fehlen (und falls es keine gibt).
    insert(
        "pdftool-bold-noto",
        egui::FontData::from_static(pdftool_core::embedded_font::NOTO_SANS),
        egui::epaint::text::FontPriority::Lowest,
    );
}

/// Text mit einem Stil je Zeichen.
#[derive(Debug, Clone)]
pub struct RichEdit {
    pub text: String,
    /// Ein Stil pro Zeichen (`text.chars()`).
    styles: Vec<TextStyle>,
    /// Text beim letzten Abgleich (zum Erkennen von Eingaben).
    prev: String,
    /// Letzte Markierung als Zeichenbereich [von, bis).
    selection: (usize, usize),
    /// Stil für die nächsten getippten Zeichen (nach einer Änderung ohne Markierung).
    typing: Option<TextStyle>,
    /// Stil, wenn der Text leer ist.
    default: TextStyle,
}

impl RichEdit {
    pub fn from_runs(runs: &[StyledRun], default: TextStyle) -> Self {
        let mut text = String::new();
        let mut styles = Vec::new();
        for r in runs {
            text.push_str(&r.text);
            styles.extend(std::iter::repeat_n(r.style.clone(), r.text.chars().count()));
        }
        let n = styles.len();
        Self {
            prev: text.clone(),
            text,
            styles,
            selection: (n, n),
            typing: None,
            default,
        }
    }

    /// Zurück in Läufe (gleiche Nachbarn zusammengefasst).
    pub fn runs(&self) -> Vec<StyledRun> {
        let runs = self
            .text
            .chars()
            .zip(&self.styles)
            .map(|(c, s)| StyledRun {
                text: c.to_string(),
                style: s.clone(),
            })
            .collect();
        normalize(runs)
    }

    /// Stile nach einer Eingabe nachziehen: gemeinsamen Anfang/Ende behalten, Neues bekommt
    /// den Stil an der Einfügestelle (bzw. den gewählten Stil fürs Tippen).
    fn sync(&mut self) {
        if self.text == self.prev {
            return;
        }
        let old: Vec<char> = self.prev.chars().collect();
        let new: Vec<char> = self.text.chars().collect();
        let p = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let max_s = old.len().min(new.len()) - p;
        let s = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(max_s)
            .take_while(|(a, b)| a == b)
            .count();
        let inserted = new.len() - p - s;
        let style = self.typing.take().unwrap_or_else(|| {
            p.checked_sub(1)
                .and_then(|i| self.styles.get(i))
                .or_else(|| self.styles.get(p))
                .cloned()
                .unwrap_or_else(|| self.default.clone())
        });
        let tail = self
            .styles
            .split_off((old.len() - s).min(self.styles.len()));
        self.styles.truncate(p);
        self.styles.extend(std::iter::repeat_n(style, inserted));
        self.styles.extend(tail);
        self.prev = self.text.clone();
    }

    fn range(&self) -> std::ops::Range<usize> {
        let (a, b) = self.selection;
        let n = self.styles.len();
        a.min(b).min(n)..a.max(b).min(n)
    }

    /// Stil der Markierung (erstes Zeichen) bzw. an der Schreibmarke.
    pub fn current(&self) -> TextStyle {
        let r = self.range();
        if let Some(t) = &self.typing {
            return t.clone();
        }
        if r.is_empty() {
            r.start
                .checked_sub(1)
                .and_then(|i| self.styles.get(i))
                .or_else(|| self.styles.first())
                .cloned()
                .unwrap_or_else(|| self.default.clone())
        } else {
            self.styles[r.start].clone()
        }
    }

    /// Ändert den Stil der Markierung; ohne Markierung den Stil fürs Weitertippen.
    pub fn apply(&mut self, f: &dyn Fn(&mut TextStyle)) {
        let r = self.range();
        if r.is_empty() {
            let mut t = self.current();
            f(&mut t);
            if self.styles.is_empty() {
                self.default = t.clone();
            }
            self.typing = Some(t);
        } else {
            for s in &mut self.styles[r] {
                f(s);
            }
        }
    }

    /// Das Textfeld. `zoom` = Bildschirmpunkte pro pt.
    pub fn text_edit(
        &mut self,
        ui: &mut egui::Ui,
        zoom: f32,
        width: f32,
        rows: usize,
        focus: bool,
    ) -> egui::Response {
        let styles = self.styles.clone();
        let fallback = self.typing.clone().unwrap_or_else(|| self.default.clone());
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = LayoutJob::default();
            job.wrap.max_width = wrap;
            let text = buf.as_str();
            // Für jedes Zeichen den Stil; neue (noch nicht abgeglichene) Zeichen wie `fallback`.
            let mut start = 0;
            let chars: Vec<(usize, char)> = text.char_indices().collect();
            let style_at = |i: usize| styles.get(i).unwrap_or(&fallback);
            let mut k = 0;
            while k < chars.len() {
                let st = style_at(k);
                let mut j = k + 1;
                while j < chars.len() && style_at(j) == st {
                    j += 1;
                }
                let end = chars.get(j).map_or(text.len(), |(b, _)| *b);
                job.append(&text[start..end], 0.0, format(ui, st, zoom));
                start = end;
                k = j;
            }
            if text.is_empty() {
                job.append("", 0.0, format(ui, &fallback, zoom));
            }
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let out = egui::TextEdit::multiline(&mut self.text)
            .background_color(Color32::WHITE)
            .desired_width(width)
            .desired_rows(rows)
            .layouter(&mut layouter)
            .show(ui);
        if focus {
            out.response.request_focus();
        }
        if let Some(r) = out.cursor_range {
            let [a, b] = r.sorted_cursors();
            let sel = (a.index.0, b.index.0);
            if sel != self.selection {
                self.selection = sel;
            }
        }
        self.sync();
        out.response.response
    }
}

/// Anzeigeformat eines Stils im Editor.
fn format(ui: &egui::Ui, st: &TextStyle, zoom: f32) -> egui::TextFormat {
    let family = if st.bold {
        FontFamily::Name(BOLD_FAMILY.into())
    } else {
        FontFamily::Name(crate::app::NOTO_FAMILY.into())
    };
    let c = st.color;
    let color = Color32::from_rgb(
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8,
    );
    // Das Eingabefeld ist weiß wie Papier; sehr helle Schrift wäre darauf unsichtbar.
    let _ = ui;
    let color = if c.r + c.g + c.b > 2.6 {
        Color32::from_gray(150)
    } else {
        color
    };
    let size = (st.size * zoom).clamp(9.0, 48.0);
    egui::TextFormat {
        font_id: FontId::new(size, family),
        color,
        italics: st.italic,
        underline: if st.underline {
            Stroke::new(1.0, color)
        } else {
            Stroke::NONE
        },
        strikethrough: if st.strike {
            Stroke::new(1.0, color)
        } else {
            Stroke::NONE
        },
        ..Default::default()
    }
}

/// Auswahlliste der Schriften: Originalschriften des Absatzes, Noto Sans, Systemfamilien.
#[derive(Clone, Default)]
pub struct FontChoices {
    /// (Schrift, Anzeigename).
    pub originals: Vec<(FontRef, String)>,
}

pub fn font_label(font: &FontRef, choices: &FontChoices) -> String {
    match font {
        FontRef::Noto => "Noto Sans (eingebaut)".into(),
        FontRef::Family(f) => f.clone(),
        FontRef::Original(_) => choices
            .originals
            .iter()
            .find(|(f, _)| f == font)
            .map_or_else(
                || "Originalschrift".into(),
                |(_, n)| format!("{n} (Original)"),
            ),
    }
}

/// Passt `name` zur Suche? Jedes Wort der Suche muss (ohne Groß-/Kleinschreibung) vorkommen,
/// z. B. "lib ser" findet "Liberation Serif".
pub fn font_matches(name: &str, query: &str) -> bool {
    let name = name.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| name.contains(word))
}

/// Eine Stiländerung aus der Formatleiste (z. B. "fett an"). `Arc`: lässt sich günstig
/// weiterreichen; `dyn Fn`: jede Änderung ist eine eigene kleine Funktion.
pub type StyleChange = Arc<dyn Fn(&mut TextStyle)>;

/// Formatleiste. Gibt die gewünschte Änderung zurück (wird auf Markierung oder ganzen Absatz
/// angewendet). `current` = Stil an der Markierung.
pub fn format_bar(
    ui: &mut egui::Ui,
    id: &str,
    current: &TextStyle,
    choices: &FontChoices,
) -> Option<StyleChange> {
    let mut change: Option<StyleChange> = None;
    ui.horizontal_wrapped(|ui| {
        // ----- Schriftart (mit Suchfeld) -----
        // Der Suchtext lebt in egui's Zwischenspeicher (`data_mut`), solange die Liste offen ist.
        let search_id = egui::Id::unique((id, "font_search"));
        let shown = egui::ComboBox::from_id_salt((id, "font"))
            .width(170.0)
            .height(400.0)
            .selected_text(font_label(&current.font, choices))
            .show_ui(ui, |ui| {
                ui.set_min_width(240.0);
                let mut query: Option<String> = ui.data_mut(|d| d.get_temp(search_id));
                let first_frame = query.is_none();
                let q = query.get_or_insert_with(String::new);
                let r = ui.add(
                    egui::TextEdit::singleline(q)
                        .hint_text("Schrift suchen …")
                        .desired_width(f32::INFINITY),
                );
                if first_frame {
                    r.request_focus();
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.separator();

                // Alle Einträge: (Schrift, Anzeigename). Originale zuerst, dann Noto, dann System.
                let mut entries: Vec<(FontRef, String)> = choices
                    .originals
                    .iter()
                    .map(|(f, _)| (f.clone(), font_label(f, choices)))
                    .collect();
                entries.push((FontRef::Noto, font_label(&FontRef::Noto, choices)));
                let originals = entries.len();
                entries.extend(
                    pdftool_core::system_fonts::families()
                        .iter()
                        .map(|fam| (FontRef::Family(fam.to_string()), fam.to_string())),
                );
                let mut hits = 0;
                for (i, (f, label)) in entries.into_iter().enumerate() {
                    if !font_matches(&label, q) {
                        continue;
                    }
                    if i == originals && hits > 0 {
                        ui.separator();
                    }
                    let clicked = ui.selectable_label(current.font == f, &label).clicked();
                    // Enter im Suchfeld übernimmt den ersten Treffer.
                    if clicked || (enter && hits == 0) {
                        change = Some(Arc::new(move |s| s.font = f.clone()));
                        ui.close();
                    }
                    hits += 1;
                }
                if hits == 0 {
                    ui.weak("Keine Schrift gefunden");
                }
                ui.data_mut(|d| d.insert_temp(search_id, query.unwrap_or_default()));
            });
        // Liste zu: Suchtext vergessen, damit sie beim nächsten Öffnen leer startet.
        if shown.inner.is_none() {
            ui.data_mut(|d| d.remove::<String>(search_id));
        }
        // ----- Größe -----
        let mut size = current.size;
        let r = ui.add(
            egui::DragValue::new(&mut size)
                .range(4.0..=200.0)
                .speed(0.2)
                .max_decimals(1)
                .suffix(" pt"),
        );
        if r.changed() {
            change = Some(Arc::new(move |s| s.size = size));
        }
        egui::ComboBox::from_id_salt((id, "sizes"))
            .width(14.0)
            .selected_text("")
            .show_ui(ui, |ui| {
                for v in [
                    8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 36.0, 48.0,
                    72.0,
                ] {
                    if ui
                        .selectable_label((size - v).abs() < 0.05, format!("{v} pt"))
                        .clicked()
                    {
                        change = Some(Arc::new(move |s| s.size = v));
                    }
                }
            });
        ui.separator();
        // ----- Stil -----
        let toggle = |ui: &mut egui::Ui, on: bool, text: RichText, tip: &str| {
            ui.add(egui::Button::selectable(on, text).min_size(egui::vec2(26.0, 24.0)))
                .on_hover_text(tip)
                .clicked()
        };
        if toggle(ui, current.bold, RichText::new("F").strong(), "Fett") {
            let v = !current.bold;
            change = Some(Arc::new(move |s| s.bold = v));
        }
        if toggle(ui, current.italic, RichText::new("K").italics(), "Kursiv") {
            let v = !current.italic;
            change = Some(Arc::new(move |s| s.italic = v));
        }
        if toggle(
            ui,
            current.underline,
            RichText::new("U").underline(),
            "Unterstrichen",
        ) {
            let v = !current.underline;
            change = Some(Arc::new(move |s| s.underline = v));
        }
        if toggle(
            ui,
            current.strike,
            RichText::new("S").strikethrough(),
            "Durchgestrichen",
        ) {
            let v = !current.strike;
            change = Some(Arc::new(move |s| s.strike = v));
        }
        // ----- Farbe -----
        let c = current.color;
        let mut rgb = c.to_rgb8();
        if ui
            .color_edit_button_srgb(&mut rgb)
            .on_hover_text("Textfarbe")
            .changed()
        {
            let color = pdftool_core::Color::from_rgb8(rgb);
            change = Some(Arc::new(move |s| s.color = color));
        }
    });
    change
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdftool_core::Color;

    fn style(bold: bool) -> TextStyle {
        TextStyle {
            bold,
            ..TextStyle::noto(12.0, Color::BLACK)
        }
    }

    #[test]
    fn font_search() {
        assert!(font_matches("Liberation Serif", ""));
        assert!(font_matches("Liberation Serif", "lib ser"));
        assert!(font_matches("DejaVu Sans", "SANS"));
        assert!(!font_matches("DejaVu Sans", "serif"));
    }

    #[test]
    fn styles_follow_edits() {
        let runs = vec![
            StyledRun {
                text: "ab".into(),
                style: style(false),
            },
            StyledRun {
                text: "CD".into(),
                style: style(true),
            },
        ];
        let mut e = RichEdit::from_runs(&runs, style(false));
        // Tippen am Ende des fetten Teils: übernimmt "fett".
        e.text = "abCDx".into();
        e.sync();
        assert_eq!(e.runs()[1].text, "CDx");
        // Löschen in der Mitte.
        e.text = "aCDx".into();
        e.sync();
        assert_eq!(e.runs()[0].text, "a");
        // Markierung "CD" kursiv machen.
        e.selection = (1, 3);
        e.apply(&|s| s.italic = true);
        let r = e.runs();
        assert_eq!(r.len(), 3, "{r:#?}");
        assert!(r[1].style.italic && r[1].text == "CD");
        // Ohne Markierung: Stil gilt fürs Weitertippen.
        e.selection = (4, 4);
        e.apply(&|s| s.underline = true);
        e.text = "aCDxy".into();
        e.sync();
        let r = e.runs();
        assert!(r.last().unwrap().style.underline && r.last().unwrap().text == "y");
    }
}
