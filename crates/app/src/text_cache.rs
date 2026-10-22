//! Cache für extrahierten Seitentext (wird für Suche und Textauswahl gebraucht).

use foliocraft_core::PageText;
use foliocraft_render::{DocId, RenderService};
use std::sync::Arc;

pub struct TextCache {
    doc_id: DocId,
    /// `Some` = Text liegt vor. `Arc`, damit Suche/Auswahl ihn billig teilen können.
    pages: Vec<Option<Arc<PageText>>>,
    requested: Vec<bool>,
}

impl TextCache {
    pub fn new(doc_id: DocId, page_count: usize) -> Self {
        Self {
            doc_id,
            pages: vec![None; page_count],
            requested: vec![false; page_count],
        }
    }

    pub fn get(&self, page: usize) -> Option<&Arc<PageText>> {
        self.pages.get(page)?.as_ref()
    }

    /// Fordert den Text an, falls er weder vorliegt noch unterwegs ist.
    pub fn ensure(&mut self, render: &RenderService, page: usize) {
        if page < self.pages.len() && self.pages[page].is_none() && !self.requested[page] {
            self.requested[page] = true;
            render.extract_text(self.doc_id, page as u32);
        }
    }

    pub fn insert(&mut self, doc: DocId, page: u32, text: PageText) {
        if doc == self.doc_id
            && let Some(slot) = self.pages.get_mut(page as usize)
        {
            *slot = Some(Arc::new(text));
        }
    }

    /// Nach Neuladen im Worker: noch nicht gelieferte Texte erneut anfordern lassen.
    pub fn reset_requested(&mut self) {
        for (req, page) in self.requested.iter_mut().zip(&self.pages) {
            *req = page.is_some();
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
}
