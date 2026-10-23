//! Seitenorganisation in der App: Seitenaktionen ausführen und nach jeder
//! Strukturänderung Ansicht, Caches und Render-Worker nachziehen.

use crate::app::OpenDocument;
use pdftool_core::{DeletePages, PageId, ReorderPages, RotatePages};
use pdftool_render::{DocId, RenderService};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Jede Strukturänderung bekommt eine neue Dokument-ID für den Render-Worker. So werden
/// Bilder verworfen, die er noch nach dem alten Stand gerendert hat und die erst danach eintreffen.
static NEXT_DOC_ID: AtomicU64 = AtomicU64::new(1);

pub fn new_doc_id() -> DocId {
    // `Relaxed` reicht: Wir brauchen nur eindeutige Zahlen, keine Synchronisation mit anderen Daten.
    NEXT_DOC_ID.fetch_add(1, Ordering::Relaxed)
}

/// Seitenaktion aus der Oberfläche (Seitenleiste, Menü).
#[derive(Debug, Clone)]
pub enum PageAction {
    /// Vierteldrehungen im Uhrzeigersinn (negativ = gegen den Uhrzeigersinn).
    Rotate(Vec<usize>, i32),
    Delete(Vec<usize>),
    /// Seiten vor die Seite mit diesem (alten) Index verschieben.
    Move(Vec<usize>, usize),
}

impl OpenDocument {
    /// Seiten, auf die eine Menüaktion wirkt: die Auswahl in der Seitenleiste,
    /// sonst die aktuelle Seite.
    pub fn target_pages(&self) -> Vec<usize> {
        let sel = self.thumbs.selected_indices(&self.doc.page_ids());
        if sel.is_empty() {
            vec![self.viewer.current_page]
        } else {
            sel
        }
    }

    /// (PageId, Rotation, Inhaltsstempel) aller Seiten – daran erkennen wir Struktur-
    /// und Inhaltsänderungen.
    pub fn page_structure(&self) -> Vec<(PageId, u16, u64)> {
        self.doc
            .pages()
            .iter()
            .map(|p| (p.id, p.rotation, self.doc.content_stamp(p.object_id)))
            .collect()
    }

    pub fn apply_page_action(&mut self, action: PageAction) {
        let n = self.doc.page_count();
        let cmd: Box<dyn pdftool_core::Command> = match action {
            PageAction::Rotate(pages, turns) => Box::new(RotatePages::new(&pages, turns)),
            PageAction::Delete(pages) => {
                if pages.len() >= n {
                    tracing::warn!("Alle Seiten löschen ist nicht möglich");
                    return;
                }
                Box::new(DeletePages::new(&pages))
            }
            PageAction::Move(pages, before) => match ReorderPages::moving(&pages, before, n) {
                // An derselben Stelle losgelassen: kein (leerer) Undo-Schritt.
                Ok(c) if c.is_identity() => return,
                Ok(c) => Box::new(c),
                Err(e) => {
                    tracing::error!(error = %e, "Verschieben nicht möglich");
                    return;
                }
            },
        };
        self.execute(cmd);
    }

    /// Prüft, ob sich die Seitenstruktur geändert hat (durch ein Command, Undo oder Redo),
    /// und zieht dann alles nach. Billig genug für jeden Frame.
    pub fn sync_structure(&mut self, render: &RenderService) {
        let now = self.page_structure();
        if now == self.structure {
            return;
        }
        let before = std::mem::replace(&mut self.structure, now.clone());

        // Alter Index pro PageId, Rotation und Inhaltsstempel.
        let old_index: HashMap<PageId, (usize, u16, u64)> = before
            .iter()
            .enumerate()
            .map(|(i, &(id, rot, stamp))| (id, (i, rot, stamp)))
            .collect();
        // Text bleibt gültig, solange der Inhalt gleich ist.
        let old_of_new: Vec<Option<usize>> = now
            .iter()
            .map(|(id, _, stamp)| {
                old_index
                    .get(id)
                    .filter(|o| o.2 == *stamp)
                    .map(|&(i, _, _)| i)
            })
            .collect();
        // Bilder sind nur gültig, wenn die Seite weder gedreht noch inhaltlich geändert wurde.
        let mut new_of_old_image: HashMap<u32, u32> = HashMap::new();
        for (new, (id, rot, stamp)) in now.iter().enumerate() {
            if let Some(&(old, old_rot, old_stamp)) = old_index.get(id)
                && old_rot == *rot
                && old_stamp == *stamp
            {
                new_of_old_image.insert(old as u32, new as u32);
            }
        }

        let doc_id = new_doc_id();
        let sizes: Vec<_> = self.doc.pages().iter().map(|p| p.display_size()).collect();
        self.viewer.restructure(doc_id, sizes.clone(), |old| {
            new_of_old_image.get(&old).copied()
        });
        self.thumbs.restructure(&sizes);
        self.texts.remap(doc_id, &old_of_new);
        self.search.invalidate();
        self.selection = None;
        self.validate_selection();

        match self.doc.render_snapshot() {
            Ok(snap) => {
                render.open(doc_id, std::sync::Arc::new(snap.bytes), snap.hidden);
                tracing::debug!(
                    pages = now.len(),
                    "Seitenstruktur geändert, Renderer neu geladen"
                );
            }
            Err(e) => tracing::error!(error = %e, "Momentaufnahme für den Renderer fehlgeschlagen"),
        }
    }
}
