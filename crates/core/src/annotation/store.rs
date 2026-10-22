//! Geordnete Sammlung aller verwalteten Annotationen eines Dokuments.

use super::{AnnotId, Annotation};
use crate::geometry::Point;

#[derive(Debug, Clone, Default)]
pub struct AnnotationStore {
    /// Reihenfolge = Zeichenreihenfolge (spätere liegen oben).
    items: Vec<Annotation>,
    next_id: u64,
}

impl AnnotationStore {
    /// Vergibt eine neue ID (für Annotationen, die gleich eingefügt werden).
    pub fn allocate_id(&mut self) -> AnnotId {
        self.next_id += 1;
        AnnotId(self.next_id)
    }

    /// Fügt am Ende ein. Hat die Annotation noch keine ID (0), wird eine vergeben.
    pub fn push(&mut self, mut a: Annotation) -> AnnotId {
        if a.id.0 == 0 {
            a.id = self.allocate_id();
        }
        self.next_id = self.next_id.max(a.id.0);
        let id = a.id;
        self.items.push(a);
        id
    }

    /// Fügt an einer bestimmten Position ein (für Undo von "Löschen").
    pub fn insert(&mut self, index: usize, a: Annotation) {
        self.next_id = self.next_id.max(a.id.0);
        let index = index.min(self.items.len());
        self.items.insert(index, a);
    }

    /// Entfernt und gibt (Position, Annotation) zurück.
    pub fn remove(&mut self, id: AnnotId) -> Option<(usize, Annotation)> {
        let index = self.index_of(id)?;
        Some((index, self.items.remove(index)))
    }

    /// Ersetzt die Annotation mit derselben ID. Gibt die alte Version zurück.
    pub fn replace(&mut self, a: Annotation) -> Option<Annotation> {
        let index = self.index_of(a.id)?;
        Some(std::mem::replace(&mut self.items[index], a))
    }

    pub fn get(&self, id: AnnotId) -> Option<&Annotation> {
        self.items.iter().find(|a| a.id == id)
    }

    pub fn index_of(&self, id: AnnotId) -> Option<usize> {
        self.items.iter().position(|a| a.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Annotation> {
        self.items.iter()
    }

    /// Annotationen einer Seite (in Zeichenreihenfolge).
    pub fn on_page(&self, page: usize) -> impl Iterator<Item = &Annotation> {
        self.items.iter().filter(move |a| a.page == page)
    }

    /// Oberste Annotation an einem Punkt.
    pub fn hit_test(&self, page: usize, p: Point, tol: f32) -> Option<&Annotation> {
        self.items
            .iter()
            .rev()
            .find(|a| a.page == page && a.hit_test(p, tol))
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
