//! Schwärzen in der App: Markierungen (/Redact-Annotationen) anlegen und anwenden.
//!
//! Markieren ist eine normale, rückgängig machbare Annotation. Erst "Schwärzungen anwenden"
//! entfernt die Inhalte darunter endgültig (siehe `core::redact`).

use crate::app::OpenDocument;
use crate::tools::{Tool, ToolSettings};
use foliocraft_core::{AddAnnotation, AnnotKind, Batch, Command, Quad};

impl OpenDocument {
    /// Jeden Suchtreffer zum Schwärzen markieren (ein Undo-Schritt).
    pub fn redact_search_hits(&mut self, settings: &ToolSettings) {
        // Erst die Daten herauskopieren: `new_annotation` braucht `self` veränderlich.
        let hits: Vec<(usize, Vec<Quad>)> = self
            .search
            .hits
            .iter()
            .map(|h| {
                (
                    h.page,
                    h.rects.iter().copied().map(Quad::from_rect).collect(),
                )
            })
            .collect();
        let query = self.search.query.clone();
        let cmds: Vec<Box<dyn Command>> = hits
            .into_iter()
            .map(|(page, quads)| {
                let mut a = self.new_annotation(
                    page,
                    AnnotKind::Redact { quads },
                    Tool::RedactText,
                    settings,
                );
                a.contents = query.clone();
                Box::new(AddAnnotation::new(a)) as Box<dyn Command>
            })
            .collect();
        if !cmds.is_empty() {
            let label = format!("{} Treffer schwärzen", cmds.len());
            self.execute(Box::new(Batch::new(label, cmds)));
        }
    }

    /// Anzahl der markierten, noch nicht angewendeten Schwärzungen.
    pub fn pending_redactions(&self) -> usize {
        self.doc
            .annotations()
            .iter()
            .filter(|a| matches!(a.kind, AnnotKind::Redact { .. }))
            .count()
    }
}
