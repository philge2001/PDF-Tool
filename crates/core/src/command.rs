//! Command-Pattern: Jede Änderung am Dokument ist ein Command mit `apply` und `undo`.
//! Die `History` verwaltet Undo- und Redo-Stapel und weiß, ob seit dem Speichern
//! etwas geändert wurde.

use crate::annotation::{AnnotId, Annotation, AnnotationStore};
use crate::error::{CoreError, Result};

/// Alles, was Commands verändern dürfen. Spätere Phasen ergänzen hier z. B.
/// Seitenreihenfolge (Phase 2) oder Formularwerte (Phase 3).
#[derive(Debug, Default, Clone)]
pub struct Model {
    pub annotations: AnnotationStore,
}

/// Eine rückgängig machbare Änderung.
///
/// `Send` erlaubt, Commands später auch zwischen Threads zu verschieben;
/// `Debug` hilft beim Logging.
pub trait Command: std::fmt::Debug + Send {
    fn apply(&mut self, model: &mut Model) -> Result<()>;
    fn undo(&mut self, model: &mut Model) -> Result<()>;
    /// Kurzbeschreibung für das Menü ("Rückgängig: Hervorhebung hinzufügen").
    fn label(&self) -> String;
}

// ---------- Annotations-Commands ----------

#[derive(Debug)]
pub struct AddAnnotation {
    annotation: Annotation,
}

impl AddAnnotation {
    /// Die Annotation sollte bereits eine ID haben (`AnnotationStore::allocate_id`),
    /// damit die UI sie direkt nach dem Hinzufügen auswählen kann.
    pub fn new(annotation: Annotation) -> Self {
        Self { annotation }
    }
}

impl Command for AddAnnotation {
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        model.annotations.push(self.annotation.clone());
        Ok(())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        model
            .annotations
            .remove(self.annotation.id)
            .map(|_| ())
            .ok_or_else(|| not_found(self.annotation.id))
    }
    fn label(&self) -> String {
        format!("{} hinzufügen", self.annotation.kind.label())
    }
}

#[derive(Debug)]
pub struct RemoveAnnotation {
    id: AnnotId,
    /// Beim Ausführen gemerkt: Position und Inhalt, für das Wiederherstellen.
    removed: Option<(usize, Annotation)>,
}

impl RemoveAnnotation {
    pub fn new(id: AnnotId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for RemoveAnnotation {
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        self.removed = Some(
            model
                .annotations
                .remove(self.id)
                .ok_or_else(|| not_found(self.id))?,
        );
        Ok(())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        // `take` holt den Wert aus der Option heraus (danach ist sie None).
        let (index, a) = self.removed.take().ok_or_else(|| not_found(self.id))?;
        model.annotations.insert(index, a);
        Ok(())
    }
    fn label(&self) -> String {
        match &self.removed {
            Some((_, a)) => format!("{} löschen", a.kind.label()),
            None => "Annotation löschen".into(),
        }
    }
}

/// Ersetzt eine Annotation durch eine geänderte Fassung (Verschieben, Farbe, Text …).
#[derive(Debug)]
pub struct UpdateAnnotation {
    before: Annotation,
    after: Annotation,
}

impl UpdateAnnotation {
    pub fn new(before: Annotation, after: Annotation) -> Self {
        debug_assert_eq!(before.id, after.id);
        Self { before, after }
    }

    fn set(model: &mut Model, mut a: Annotation) -> Result<()> {
        let id = a.id;
        // Die Herkunft in der Datei kann sich seit Erzeugung des Commands geändert haben
        // (z. B. nach dem Speichern) – die aktuelle gilt.
        if let Some(current) = model.annotations.get(id) {
            a.source = current.source;
            if a.name.is_empty() {
                a.name = current.name.clone(); // beim Speichern vergebener /NM bleibt erhalten
            }
        }
        model
            .annotations
            .replace(a)
            .map(|_| ())
            .ok_or_else(|| not_found(id))
    }
}

impl Command for UpdateAnnotation {
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        Self::set(model, self.after.clone())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        Self::set(model, self.before.clone())
    }
    fn label(&self) -> String {
        format!("{} ändern", self.after.kind.label())
    }
}

/// Mehrere Commands als EIN Undo-Schritt (z. B. Hervorhebung über mehrere Seiten).
#[derive(Debug)]
pub struct Batch {
    label: String,
    commands: Vec<Box<dyn Command>>,
}

impl Batch {
    pub fn new(label: impl Into<String>, commands: Vec<Box<dyn Command>>) -> Self {
        Self {
            label: label.into(),
            commands,
        }
    }
}

impl Command for Batch {
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        for (i, cmd) in self.commands.iter_mut().enumerate() {
            if let Err(e) = cmd.apply(model) {
                // Bereits ausgeführte Teile zurückrollen, damit nichts halb angewendet bleibt.
                for done in self.commands[..i].iter_mut().rev() {
                    let _ = done.undo(model);
                }
                return Err(e);
            }
        }
        Ok(())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        for cmd in self.commands.iter_mut().rev() {
            cmd.undo(model)?;
        }
        Ok(())
    }
    fn label(&self) -> String {
        self.label.clone()
    }
}

fn not_found(id: AnnotId) -> CoreError {
    CoreError::AnnotationNotFound(format!("#{}", id.0))
}

// ---------- History ----------

/// Maximal gemerkte Schritte (ältere fallen weg).
const MAX_HISTORY: usize = 500;

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Box<dyn Command>>,
    redo: Vec<Box<dyn Command>>,
    /// Länge des Undo-Stapels beim letzten Speichern; `None` = Zustand nicht mehr erreichbar.
    saved_at: Option<usize>,
}

impl History {
    pub fn new() -> Self {
        Self {
            saved_at: Some(0),
            ..Default::default()
        }
    }

    /// Führt ein Command aus und legt es auf den Undo-Stapel.
    pub fn execute(&mut self, mut cmd: Box<dyn Command>, model: &mut Model) -> Result<()> {
        cmd.apply(model)?;
        self.undo.push(cmd);
        // Neuer Zweig: Der gespeicherte Zustand liegt evtl. im verworfenen Redo-Teil.
        if self.saved_at.is_some_and(|s| s >= self.undo.len()) {
            self.saved_at = None;
        }
        self.redo.clear();
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
            self.saved_at = self.saved_at.and_then(|s| s.checked_sub(1));
        }
        Ok(())
    }

    /// Macht den letzten Schritt rückgängig. `Ok(false)`, wenn es nichts gab.
    pub fn undo(&mut self, model: &mut Model) -> Result<bool> {
        let Some(mut cmd) = self.undo.pop() else {
            return Ok(false);
        };
        if let Err(e) = cmd.undo(model) {
            self.undo.push(cmd); // Zustand unverändert lassen
            return Err(e);
        }
        self.redo.push(cmd);
        Ok(true)
    }

    pub fn redo(&mut self, model: &mut Model) -> Result<bool> {
        let Some(mut cmd) = self.redo.pop() else {
            return Ok(false);
        };
        if let Err(e) = cmd.apply(model) {
            self.redo.push(cmd);
            return Err(e);
        }
        self.undo.push(cmd);
        Ok(true)
    }

    pub fn undo_label(&self) -> Option<String> {
        self.undo.last().map(|c| c.label())
    }

    pub fn redo_label(&self) -> Option<String> {
        self.redo.last().map(|c| c.label())
    }

    /// Gibt es Änderungen seit dem letzten Speichern?
    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo.len())
    }

    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.undo.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotation::AnnotKind;
    use crate::geometry::{Color, Point};

    fn note(model: &mut Model) -> Annotation {
        let mut a = Annotation::new(
            0,
            AnnotKind::Note {
                pos: Point::new(10.0, 10.0),
            },
            Color::YELLOW,
        );
        a.id = model.annotations.allocate_id();
        a
    }

    #[test]
    fn add_undo_redo() {
        let mut m = Model::default();
        let mut h = History::new();
        let a = note(&mut m);
        h.execute(Box::new(AddAnnotation::new(a.clone())), &mut m)
            .unwrap();
        assert_eq!(m.annotations.len(), 1);
        assert!(h.is_dirty());
        assert_eq!(h.undo_label().unwrap(), "Notiz hinzufügen");

        assert!(h.undo(&mut m).unwrap());
        assert!(m.annotations.is_empty());
        assert!(!h.is_dirty());
        assert!(h.redo(&mut m).unwrap());
        assert_eq!(m.annotations.get(a.id), Some(&a));
        assert!(!h.redo(&mut m).unwrap());
    }

    #[test]
    fn remove_restores_position() {
        let mut m = Model::default();
        let mut h = History::new();
        let ids: Vec<_> = (0..3)
            .map(|_| {
                let a = note(&mut m);
                m.annotations.push(a)
            })
            .collect();
        h.execute(Box::new(RemoveAnnotation::new(ids[1])), &mut m)
            .unwrap();
        assert_eq!(m.annotations.len(), 2);
        h.undo(&mut m).unwrap();
        assert_eq!(m.annotations.index_of(ids[1]), Some(1));
    }

    #[test]
    fn batch_is_one_undo_step() {
        let mut m = Model::default();
        let mut h = History::new();
        let cmds: Vec<Box<dyn Command>> = (0..3)
            .map(|_| Box::new(AddAnnotation::new(note(&mut m))) as Box<dyn Command>)
            .collect();
        h.execute(Box::new(Batch::new("3 Notizen", cmds)), &mut m)
            .unwrap();
        assert_eq!(m.annotations.len(), 3);
        h.undo(&mut m).unwrap();
        assert!(m.annotations.is_empty());
    }

    #[test]
    fn update_and_dirty_tracking() {
        let mut m = Model::default();
        let mut h = History::new();
        let a = note(&mut m);
        m.annotations.push(a.clone());
        h.mark_saved();

        let mut b = a.clone();
        b.translate(5.0, 0.0);
        b.color = Color::RED;
        h.execute(
            Box::new(UpdateAnnotation::new(a.clone(), b.clone())),
            &mut m,
        )
        .unwrap();
        assert_eq!(m.annotations.get(a.id).unwrap().color, Color::RED);
        h.mark_saved();
        assert!(!h.is_dirty());
        h.undo(&mut m).unwrap();
        assert!(h.is_dirty());
        assert_eq!(m.annotations.get(a.id), Some(&a));

        // Neuer Zweig nach Undo: gespeicherter Zustand ist nicht mehr erreichbar.
        h.execute(Box::new(RemoveAnnotation::new(a.id)), &mut m)
            .unwrap();
        assert!(h.is_dirty());
        h.undo(&mut m).unwrap();
        assert!(h.is_dirty());
    }
}
