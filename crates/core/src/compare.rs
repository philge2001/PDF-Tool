//! Dokumentvergleich auf Wortebene (Phase 8).
//!
//! Beide Dokumente werden in Wörter zerlegt (aus dem Seitentext mit Zeichenrechtecken,
//! den der Render-Worker liefert). Die Wortfolgen des *ganzen* Dokuments werden verglichen –
//! so werden auch eingefügte oder gelöschte Seiten richtig zugeordnet. Für jede Änderung
//! gibt es die betroffenen Wörter beider Seiten samt Rechtecken zum Hervorheben.

use crate::geometry::Rect;
use crate::text::PageText;
use similar::{Algorithm, DiffOp};

/// Ein Wort mit Seite und Rechteck (PDF-Benutzerraum).
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub page: usize,
    pub rect: Rect,
}

/// Zerlegt den Text einer Seite in Wörter (getrennt an Leerraum; Zeichen ohne Rechteck,
/// z. B. von PDFium eingefügte Zeilenumbrüche, trennen ebenfalls).
pub fn words(page: usize, text: &PageText) -> Vec<Word> {
    let mut out = Vec::new();
    let mut current: Option<Word> = None;
    for c in &text.chars {
        if c.ch.is_whitespace() || !c.has_area() {
            out.extend(current.take());
            continue;
        }
        match &mut current {
            Some(w) => {
                w.text.push(c.ch);
                w.rect = w.rect.union(&c.rect);
            }
            None => {
                current = Some(Word {
                    text: c.ch.to_string(),
                    page,
                    rect: c.rect,
                })
            }
        }
    }
    out.extend(current);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Nur im alten Dokument.
    Deleted,
    /// Nur im neuen Dokument.
    Inserted,
    /// Wörter ersetzt.
    Replaced,
}

/// Eine Änderung: betroffene Wörter im alten (`old`) und im neuen Dokument (`new`).
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub kind: ChangeKind,
    pub old: Vec<Word>,
    pub new: Vec<Word>,
    /// Seite, auf der die Änderung im alten bzw. neuen Dokument liegt (bei reinen Einfügungen/
    /// Löschungen: die Stelle, an der der Text im jeweils anderen Dokument stehen würde).
    pub old_page: usize,
    pub new_page: usize,
}

impl Change {
    pub fn old_text(&self) -> String {
        join(&self.old)
    }
    pub fn new_text(&self) -> String {
        join(&self.new)
    }
}

fn join(words: &[Word]) -> String {
    words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Vergleicht zwei Wortfolgen (alt, neu).
pub fn compare(old: &[Word], new: &[Word]) -> Vec<Change> {
    let a: Vec<&str> = old.iter().map(|w| w.text.as_str()).collect();
    let b: Vec<&str> = new.iter().map(|w| w.text.as_str()).collect();
    // Patience-Diff: robust bei verschobenen Absätzen, gut lesbare Ergebnisse.
    let ops = similar::capture_diff_slices(Algorithm::Patience, &a, &b);
    // Seite an einer Position (für Einfügungen/Löschungen: das Wort davor, sonst Seite 0).
    let page_at = |words: &[Word], i: usize| {
        words
            .get(i)
            .or_else(|| i.checked_sub(1).and_then(|j| words.get(j)))
            .map_or(0, |w| w.page)
    };
    let mut changes = Vec::new();
    for op in ops {
        let (kind, old_range, new_range) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (
                ChangeKind::Deleted,
                old_index..old_index + old_len,
                new_index..new_index,
            ),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (
                ChangeKind::Inserted,
                old_index..old_index,
                new_index..new_index + new_len,
            ),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (
                ChangeKind::Replaced,
                old_index..old_index + old_len,
                new_index..new_index + new_len,
            ),
        };
        changes.push(Change {
            kind,
            old_page: page_at(old, old_range.start),
            new_page: page_at(new, new_range.start),
            old: old[old_range].to_vec(),
            new: new[new_range].to_vec(),
        });
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::CharBox;

    /// Seite aus Text: jedes Zeichen 10 pt breit, Leerzeichen trennen.
    fn page(text: &str) -> PageText {
        PageText::new(
            text.chars()
                .enumerate()
                .map(|(i, ch)| CharBox {
                    ch,
                    rect: Rect::new(i as f32 * 10.0, 0.0, i as f32 * 10.0 + 10.0, 10.0),
                })
                .collect(),
        )
    }

    #[test]
    fn word_rects() {
        let w = words(3, &page("ab  cde"));
        assert_eq!(w.len(), 2);
        assert_eq!(w[1].text, "cde");
        assert_eq!((w[1].rect.x0, w[1].rect.x1), (40.0, 70.0));
        assert_eq!(w[1].page, 3);
    }

    #[test]
    fn detects_insert_delete_replace_across_pages() {
        let mut old = words(0, &page("Der Betrag ist zahlbar bis Freitag"));
        old.extend(words(1, &page("Mit freundlichen Grüßen")));
        let mut new = words(0, &page("Der Betrag ist sofort zahlbar bis Montag"));
        new.extend(words(1, &page("Mit Grüßen")));
        let changes = compare(&old, &new);
        let kinds: Vec<_> = changes.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ChangeKind::Inserted,
                ChangeKind::Replaced,
                ChangeKind::Deleted
            ]
        );
        assert_eq!(changes[0].new_text(), "sofort");
        assert_eq!(
            (changes[1].old_text(), changes[1].new_text()),
            ("Freitag".into(), "Montag".into())
        );
        assert_eq!(changes[2].old_text(), "freundlichen");
        assert_eq!((changes[2].old_page, changes[2].new_page), (1, 1));
        assert!(compare(&old, &old).is_empty());
    }
}
