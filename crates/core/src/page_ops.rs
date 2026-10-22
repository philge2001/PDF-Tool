//! Seitenoperationen als Commands: drehen, löschen, umsortieren.
//!
//! Annotationen verweisen per Seitenindex auf ihre Seite. Ändert sich die Reihenfolge,
//! rechnen die Commands diese Indizes um – so wandert jede Annotation mit ihrer Seite.

use crate::annotation::Annotation;
use crate::command::{Command, Model};
use crate::document::PageInfo;
use crate::error::{CoreError, Result};
use std::any::Any;

/// Sortiert, entfernt Duplikate und prüft, dass alle Indizes gültig sind.
fn normalize(pages: &[usize], count: usize) -> Result<Vec<usize>> {
    let mut v = pages.to_vec();
    v.sort_unstable();
    v.dedup();
    match v.last() {
        Some(&last) if last >= count => Err(CoreError::PageOutOfRange(last)),
        _ => Ok(v),
    }
}

fn page_label(n: usize) -> String {
    if n == 1 {
        "Seite".into()
    } else {
        format!("{n} Seiten")
    }
}

// ---------- Drehen ----------

#[derive(Debug)]
pub struct RotatePages {
    pages: Vec<usize>,
    /// Vierteldrehungen im Uhrzeigersinn (negativ = gegen den Uhrzeigersinn).
    quarter_turns: i32,
}

impl RotatePages {
    pub fn new(pages: &[usize], quarter_turns: i32) -> Self {
        Self {
            pages: pages.to_vec(),
            quarter_turns,
        }
    }

    fn turn(model: &mut Model, pages: &[usize], quarter_turns: i32) -> Result<()> {
        let pages = normalize(pages, model.pages.len())?;
        for i in pages {
            let p = &mut model.pages[i];
            // `rem_euclid` liefert immer ein nicht-negatives Ergebnis (anders als `%`).
            p.rotation = (p.rotation as i32 + 90 * quarter_turns).rem_euclid(360) as u16;
        }
        Ok(())
    }
}

impl Command for RotatePages {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        Self::turn(model, &self.pages, self.quarter_turns)
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        Self::turn(model, &self.pages, -self.quarter_turns)
    }
    fn label(&self) -> String {
        format!("{} drehen", page_label(self.pages.len()))
    }
}

// ---------- Löschen ----------

#[derive(Debug)]
pub struct DeletePages {
    pages: Vec<usize>,
    /// Beim Ausführen gemerkt: gelöschte Seiten (Index, Info) und deren Annotationen.
    removed_pages: Vec<(usize, PageInfo)>,
    removed_annots: Vec<(usize, Annotation)>,
}

impl DeletePages {
    pub fn new(pages: &[usize]) -> Self {
        Self {
            pages: pages.to_vec(),
            removed_pages: Vec::new(),
            removed_annots: Vec::new(),
        }
    }
}

impl Command for DeletePages {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        let pages = normalize(&self.pages, model.pages.len())?;
        if pages.len() >= model.pages.len() {
            return Err(CoreError::Other(
                "Ein Dokument braucht mindestens eine Seite.".into(),
            ));
        }
        self.removed_annots = model.annotations.take_on_pages(&pages);
        // Absteigend entfernen, damit die übrigen Indizes gültig bleiben.
        self.removed_pages = pages
            .iter()
            .rev()
            .map(|&i| (i, model.pages.remove(i)))
            .collect();
        self.removed_pages.reverse();
        // Jede verbleibende Annotation rückt um die Zahl der davor gelöschten Seiten nach vorn.
        model
            .annotations
            .remap_pages(|old| old - pages.iter().filter(|&&d| d < old).count());
        self.pages = pages;
        Ok(())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        let deleted = std::mem::take(&mut self.pages);
        // Umkehrung der Umrechnung: neuer Index -> alter Index.
        let kept_old: Vec<usize> = (0..model.pages.len() + deleted.len())
            .filter(|i| !deleted.contains(i))
            .collect();
        model.annotations.remap_pages(|new| kept_old[new]);
        for (i, p) in std::mem::take(&mut self.removed_pages) {
            model.pages.insert(i, p);
        }
        model
            .annotations
            .restore(std::mem::take(&mut self.removed_annots));
        self.pages = deleted;
        Ok(())
    }
    fn label(&self) -> String {
        format!("{} löschen", page_label(self.pages.len()))
    }
}

// ---------- Umsortieren ----------

/// Neue Reihenfolge: `order[neu] = alt`.
#[derive(Debug)]
pub struct ReorderPages {
    order: Vec<usize>,
}

impl ReorderPages {
    pub fn new(order: Vec<usize>) -> Self {
        Self { order }
    }

    /// Verschiebt die ausgewählten Seiten (in ihrer bisherigen Reihenfolge) so, dass sie
    /// vor der Seite stehen, die vorher den Index `before` hatte (`before == count` = ans Ende).
    pub fn moving(pages: &[usize], before: usize, count: usize) -> Result<Self> {
        let moved = normalize(pages, count)?;
        let before = before.min(count);
        let rest: Vec<usize> = (0..count).filter(|i| !moved.contains(i)).collect();
        // Einfügeposition in der Restliste: alle Restseiten, die vor `before` lagen.
        let at = rest.iter().filter(|&&i| i < before).count();
        let mut order = rest[..at].to_vec();
        order.extend(&moved);
        order.extend(&rest[at..]);
        Ok(Self::new(order))
    }

    /// Ändert sich gar nichts? (z. B. Seite an ihrer eigenen Position fallen gelassen)
    pub fn is_identity(&self) -> bool {
        self.order.iter().enumerate().all(|(i, &o)| i == o)
    }

    fn permute(model: &mut Model, order: &[usize]) -> Result<()> {
        let n = model.pages.len();
        let mut seen = vec![false; n];
        let valid = order.len() == n
            && order
                .iter()
                .all(|&i| i < n && !std::mem::replace(&mut seen[i], true));
        if !valid {
            return Err(CoreError::Other("Ungültige Seitenreihenfolge".into()));
        }
        let old = std::mem::take(&mut model.pages);
        model.pages = order.iter().map(|&i| old[i].clone()).collect();
        // alt -> neu für die Annotationen
        let mut new_of_old = vec![0; n];
        for (new, &old) in order.iter().enumerate() {
            new_of_old[old] = new;
        }
        model.annotations.remap_pages(|old| new_of_old[old]);
        Ok(())
    }

    fn inverse(&self) -> Vec<usize> {
        let mut inv = vec![0; self.order.len()];
        for (new, &old) in self.order.iter().enumerate() {
            inv[old] = new;
        }
        inv
    }
}

impl Command for ReorderPages {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        Self::permute(model, &self.order)
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        let inv = self.inverse();
        Self::permute(model, &inv)
    }
    fn label(&self) -> String {
        "Seiten verschieben".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::AnnotKind;
    use crate::geometry::{Color, Point, Rect};

    /// Modell mit `n` Seiten und je einer Notiz pro Seite (Inhalt = ursprüngliche Seitennummer).
    fn model(n: usize) -> Model {
        let mut m = Model::default();
        for i in 0..n {
            let id = m.allocate_page_id();
            m.pages.push(PageInfo {
                id,
                object_id: (i as u32 + 10, 0),
                crop_box: Rect::new(0.0, 0.0, 600.0, 800.0),
                rotation: 0,
            });
            let mut a = Annotation::new(
                i,
                AnnotKind::Note {
                    pos: Point::new(0.0, 0.0),
                },
                Color::YELLOW,
            );
            a.contents = format!("S{i}");
            m.annotations.push(a);
        }
        m
    }

    fn ids(m: &Model) -> Vec<u32> {
        m.pages.iter().map(|p| p.id.0).collect()
    }

    /// Prüft, dass jede Notiz noch auf "ihrer" Seite liegt.
    fn assert_notes_follow(m: &Model) {
        for a in m.annotations.iter() {
            let original = m.pages[a.page].id.0 - 1; // IDs beginnen bei 1
            assert_eq!(a.contents, format!("S{original}"));
        }
    }

    #[test]
    fn rotate_and_undo() {
        let mut m = model(3);
        let mut c = RotatePages::new(&[0, 2], -1);
        c.apply(&mut m).unwrap();
        assert_eq!(
            m.pages.iter().map(|p| p.rotation).collect::<Vec<_>>(),
            vec![270, 0, 270]
        );
        c.undo(&mut m).unwrap();
        assert!(m.pages.iter().all(|p| p.rotation == 0));
    }

    #[test]
    fn delete_pages_moves_annotations_and_undo_restores() {
        let mut m = model(5);
        let mut c = DeletePages::new(&[3, 1]);
        c.apply(&mut m).unwrap();
        assert_eq!(ids(&m), vec![1, 3, 5]);
        assert_eq!(m.annotations.len(), 3);
        assert_notes_follow(&m);

        c.undo(&mut m).unwrap();
        assert_eq!(ids(&m), vec![1, 2, 3, 4, 5]);
        assert_eq!(m.annotations.len(), 5);
        assert_notes_follow(&m);
        // Reihenfolge im Store ist wiederhergestellt
        let order: Vec<_> = m.annotations.iter().map(|a| a.contents.clone()).collect();
        assert_eq!(order, vec!["S0", "S1", "S2", "S3", "S4"]);
    }

    #[test]
    fn cannot_delete_all_pages() {
        let mut m = model(2);
        assert!(DeletePages::new(&[0, 1]).apply(&mut m).is_err());
        assert_eq!(m.pages.len(), 2);
    }

    #[test]
    fn move_pages_and_undo() {
        let mut m = model(5);
        // Seiten 4 und 1 (0-basiert) vor Seite 0 verschieben
        let mut c = ReorderPages::moving(&[4, 1], 0, 5).unwrap();
        c.apply(&mut m).unwrap();
        assert_eq!(ids(&m), vec![2, 5, 1, 3, 4]);
        assert_notes_follow(&m);
        c.undo(&mut m).unwrap();
        assert_eq!(ids(&m), vec![1, 2, 3, 4, 5]);
        assert_notes_follow(&m);

        // An die eigene Position = keine Änderung
        assert!(ReorderPages::moving(&[2], 2, 5).unwrap().is_identity());
        assert!(ReorderPages::moving(&[2], 3, 5).unwrap().is_identity());

        // Ans Ende verschieben
        let mut c = ReorderPages::moving(&[0], 5, 5).unwrap();
        c.apply(&mut m).unwrap();
        assert_eq!(ids(&m), vec![2, 3, 4, 5, 1]);
    }

    #[test]
    fn invalid_order_is_rejected() {
        let mut m = model(3);
        assert!(ReorderPages::new(vec![0, 0, 1]).apply(&mut m).is_err());
        assert!(ReorderPages::new(vec![0, 1]).apply(&mut m).is_err());
        assert_eq!(ids(&m), vec![1, 2, 3]);
    }
}
