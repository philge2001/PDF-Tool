//! Mehrere Dokumente als Tabs.
//!
//! Das aktive Dokument steht wie bisher in `PdfToolApp::document` – so bleibt der übrige Code
//! unverändert. Die anderen Tabs liegen in `others`; `active_pos` ist die Position des aktiven
//! Tabs in der Reihenfolge `others[..pos] + aktiv + others[pos..]`.
//!
//! Der Render-Worker hält immer nur ein Dokument. Beim Wechsel bekommt er das neue (unter
//! dessen alter `DocId`), deshalb bleiben bereits gerenderte Seitenbilder gültig.

use crate::app::{OpenDocument, PdfToolApp, hidden_annots};
use crate::tools::Tool;
use pdftool_core::Document;
use std::path::Path;

impl PdfToolApp {
    /// Anzahl offener Dokumente.
    pub(crate) fn tab_count(&self) -> usize {
        self.others.len() + usize::from(self.document.is_some())
    }

    /// Dokument an Tab-Position `i` (nur lesend, z. B. für Titel).
    pub(crate) fn tab(&self, i: usize) -> Option<&OpenDocument> {
        let p = self.active_pos;
        match i.cmp(&p) {
            std::cmp::Ordering::Less => self.others.get(i),
            std::cmp::Ordering::Equal => self.document.as_ref(),
            std::cmp::Ordering::Greater => self.others.get(i - 1),
        }
    }

    /// Wechselt zum Tab `i`.
    pub(crate) fn switch_tab(&mut self, i: usize) {
        if i == self.active_pos && self.document.is_some() {
            self.show_home = false;
            return;
        }
        // Alle Tabs in Reihenfolge zusammenlegen, Ziel herausnehmen, Rest zurücklegen.
        let mut all = std::mem::take(&mut self.others);
        if let Some(cur) = self.document.take() {
            all.insert(self.active_pos.min(all.len()), cur);
        }
        if i >= all.len() {
            self.others = all;
            return;
        }
        let next = all.remove(i);
        self.others = all;
        self.active_pos = i;
        self.document = Some(next);
        self.show_home = false;
        self.on_tab_changed();
    }

    /// Neues Dokument als weiteren Tab rechts neben dem aktiven öffnen.
    pub(crate) fn open_in_new_tab(&mut self, doc: Document) {
        let previous = self.document.take();
        let had_previous = previous.is_some();
        if let Some(cur) = previous {
            self.others
                .insert(self.active_pos.min(self.others.len()), cur);
            self.active_pos += 1;
        }
        self.adopt_document(doc);
        if self.document.is_none() && had_previous {
            // Anzeigen fehlgeschlagen: zurück zum vorherigen Tab.
            self.active_pos -= 1;
            self.document = Some(self.others.remove(self.active_pos));
            self.on_tab_changed();
            return;
        }
        self.tool_reset_for_tab();
    }

    /// Ist die Datei schon in einem Tab offen? Dann dorthin wechseln.
    pub(crate) fn focus_open_file(&mut self, path: &Path) -> bool {
        let found = (0..self.tab_count()).find(|&i| {
            self.tab(i)
                .and_then(|d| d.doc.path())
                .is_some_and(|p| same_file(p, path))
        });
        match found {
            Some(i) => {
                self.switch_tab(i);
                true
            }
            None => false,
        }
    }

    /// Aktives Dokument schließen; ein benachbarter Tab wird aktiv.
    pub(crate) fn close_active_tab(&mut self) {
        self.document = None;
        if self.others.is_empty() {
            self.active_pos = 0;
            self.show_home = true;
        } else {
            let i = self.active_pos.min(self.others.len() - 1);
            self.document = Some(self.others.remove(i));
            self.active_pos = i;
            self.on_tab_changed();
        }
        self.tool_reset_for_tab();
    }

    /// Erster Tab mit ungespeicherten Änderungen.
    pub(crate) fn first_dirty_tab(&self) -> Option<usize> {
        (0..self.tab_count()).find(|&i| self.tab(i).is_some_and(|d| d.doc.is_dirty()))
    }

    /// Nach dem Wechsel: Worker auf das neue aktive Dokument umstellen.
    fn on_tab_changed(&mut self) {
        self.tool_reset_for_tab();
        let (Ok(render), Some(d)) = (&self.render, &mut self.document) else {
            return;
        };
        if d.doc.is_dirty() {
            // Ungespeicherte Änderungen: Momentaufnahme des aktuellen Stands.
            match d.doc.render_snapshot() {
                Ok(s) => render.open(d.viewer.doc_id, std::sync::Arc::new(s.bytes), s.hidden),
                Err(e) => {
                    self.error = Some(format!("Dokument kann nicht angezeigt werden:\n{e}"));
                    return;
                }
            }
        } else {
            render.open(
                d.viewer.doc_id,
                d.doc.bytes().clone(),
                hidden_annots(&d.doc),
            );
        }
        // Aufträge an den Worker für dieses Dokument sind beim Wechsel verfallen.
        d.viewer.reset_pending();
        d.texts.reset_requested();
    }

    /// Werkzeuge gelten pro Dokument: Bearbeiten-/Formularmodus beim Wechsel verlassen.
    fn tool_reset_for_tab(&mut self) {
        if self.tool.is_form_tool() || self.tool.is_content_tool() {
            self.tool = Tool::TextSelect;
        }
    }
}

/// Gleiche Datei? (Pfade können unterschiedlich geschrieben sein.)
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
