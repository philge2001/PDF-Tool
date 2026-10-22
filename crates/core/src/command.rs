//! Command-Pattern: Jede Änderung am Dokument ist ein Command mit `apply` und `undo`.
//! Die `History` verwaltet Undo- und Redo-Stapel und weiß, ob seit dem Speichern
//! etwas geändert wurde.

use crate::annotation::{AnnotId, Annotation, AnnotationStore};
use crate::document::{PageId, PageInfo};
use crate::error::{CoreError, Result};
use crate::form::{FieldId, FieldValue, FormField, FormStore};
use std::any::Any;

/// Alles, was Commands verändern dürfen.
#[derive(Debug, Default, Clone)]
pub struct Model {
    pub annotations: AnnotationStore,
    /// Formularfelder (AcroForm).
    pub forms: FormStore,
    /// Seiten in Anzeigereihenfolge. Annotationen verweisen per Index darauf; Seiten-Commands
    /// rechnen diese Indizes beim Umsortieren/Löschen mit um.
    pub pages: Vec<PageInfo>,
    /// Neue PDF-Objekte, die noch nicht in der Datei stehen (z. B. eingefügte Seiten
    /// samt Schriften und Bildern). Werden beim Speichern geschrieben.
    pub extra_objects: std::collections::BTreeMap<lopdf::ObjectId, lopdf::Object>,
    /// Bearbeitete Seiteninhalte (Text bearbeiten/hinzufügen), nach Objektnummer der Seite.
    /// Einmal bearbeitet, bleibt eine Seite hier – Undo setzt den vorigen Inhalt ein.
    pub page_content: std::collections::BTreeMap<lopdf::ObjectId, crate::text_edit::PageContent>,
    /// Zählt jede Inhaltsänderung (auch Undo/Redo); liefert die `stamp`-Werte.
    pub content_epoch: u64,
    next_page_id: u32,
}

impl Model {
    pub fn allocate_page_id(&mut self) -> PageId {
        self.next_page_id += 1;
        PageId(self.next_page_id)
    }
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

    /// Für das Zusammenführen: Zugriff auf den konkreten Typ hinter `dyn Command`.
    /// Jede Implementierung schreibt hier einfach `self`.
    fn as_any(&self) -> &dyn Any;

    /// Versucht, ein direkt folgendes Command in dieses einzuarbeiten (z. B. viele kleine
    /// Farbänderungen beim Ziehen im Farbwähler = ein Undo-Schritt). Standard: nein.
    fn merge(&mut self, _next: &dyn Command) -> bool {
        false
    }
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
    fn as_any(&self) -> &dyn Any {
        self
    }
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
    fn as_any(&self) -> &dyn Any {
        self
    }
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
    mergeable: bool,
}

impl UpdateAnnotation {
    pub fn new(before: Annotation, after: Annotation) -> Self {
        debug_assert_eq!(before.id, after.id);
        Self {
            before,
            after,
            mergeable: false,
        }
    }

    /// Darf mit direkt folgenden, ebenfalls zusammenführbaren Änderungen derselben
    /// Annotation zu einem Undo-Schritt verschmelzen (Farbwähler, Tippen).
    pub fn mergeable(mut self) -> Self {
        self.mergeable = true;
        self
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
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        Self::set(model, self.after.clone())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        Self::set(model, self.before.clone())
    }
    fn label(&self) -> String {
        format!("{} ändern", self.after.kind.label())
    }
    fn merge(&mut self, next: &dyn Command) -> bool {
        // `downcast_ref` liefert Some nur, wenn `next` wirklich ein UpdateAnnotation ist.
        match next.as_any().downcast_ref::<UpdateAnnotation>() {
            Some(n) if self.mergeable && n.mergeable && n.after.id == self.after.id => {
                self.after = n.after.clone(); // `before` bleibt der Stand vor der ersten Änderung
                true
            }
            _ => false,
        }
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
    fn as_any(&self) -> &dyn Any {
        self
    }
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

// ---------- Formular-Commands ----------

/// Neues Formularfeld (Formular-Editor).
#[derive(Debug)]
pub struct AddField {
    field: FormField,
}

impl AddField {
    /// Das Feld sollte bereits eine ID haben (`FormStore::allocate_id`).
    pub fn new(field: FormField) -> Self {
        Self { field }
    }
}

impl Command for AddField {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        model.forms.push(self.field.clone());
        Ok(())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        model
            .forms
            .remove(self.field.id)
            .map(|_| ())
            .ok_or_else(|| field_not_found(self.field.id))
    }
    fn label(&self) -> String {
        format!("{} hinzufügen", self.field.kind.label())
    }
}

#[derive(Debug)]
pub struct RemoveField {
    id: FieldId,
    removed: Option<(usize, FormField)>,
}

impl RemoveField {
    pub fn new(id: FieldId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for RemoveField {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        self.removed = Some(
            model
                .forms
                .remove(self.id)
                .ok_or_else(|| field_not_found(self.id))?,
        );
        Ok(())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        let (i, f) = self
            .removed
            .take()
            .ok_or_else(|| field_not_found(self.id))?;
        model.forms.insert(i, f);
        Ok(())
    }
    fn label(&self) -> String {
        match &self.removed {
            Some((_, f)) => format!("{} löschen", f.kind.label()),
            None => "Formularfeld löschen".into(),
        }
    }
}

fn field_not_found(id: FieldId) -> CoreError {
    CoreError::Other(format!("Formularfeld #{} existiert nicht", id.0))
}

/// Ersetzt ein Formularfeld durch eine geänderte Fassung (Wert, später auch Eigenschaften).
#[derive(Debug)]
pub struct UpdateField {
    before: FormField,
    after: FormField,
    mergeable: bool,
}

impl UpdateField {
    pub fn new(before: FormField, after: FormField) -> Self {
        debug_assert_eq!(before.id, after.id);
        Self {
            before,
            after,
            mergeable: false,
        }
    }

    /// Nur den Wert ändern.
    pub fn value(field: &FormField, value: FieldValue) -> Self {
        let mut after = field.clone();
        after.value = value;
        Self::new(field.clone(), after)
    }

    /// Wie bei [`UpdateAnnotation::mergeable`]: Tippen in ein Feld = ein Undo-Schritt.
    pub fn mergeable(mut self) -> Self {
        self.mergeable = true;
        self
    }

    pub fn field_id(&self) -> FieldId {
        self.after.id
    }

    fn set(model: &mut Model, mut f: FormField) -> Result<()> {
        let id = f.id;
        if let Some(current) = model.forms.get(id) {
            f.source = current.source; // Herkunft kann sich durchs Speichern geändert haben
            // Widget-Objektnummern ebenso (neue Widgets bekommen sie beim Speichern).
            for (w, cur) in f.widgets.iter_mut().zip(&current.widgets) {
                if w.object_id.is_none() {
                    w.object_id = cur.object_id;
                }
            }
        }
        model
            .forms
            .replace(f)
            .map(|_| ())
            .ok_or_else(|| field_not_found(id))
    }
}

impl Command for UpdateField {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn apply(&mut self, model: &mut Model) -> Result<()> {
        Self::set(model, self.after.clone())
    }
    fn undo(&mut self, model: &mut Model) -> Result<()> {
        Self::set(model, self.before.clone())
    }
    fn label(&self) -> String {
        let only_value = FormField {
            value: self.before.value.clone(),
            ..self.after.clone()
        } == self.before;
        if only_value {
            format!("„{}“ ausfüllen", self.after.short_name())
        } else {
            format!("Feld „{}“ ändern", self.after.short_name())
        }
    }
    fn merge(&mut self, next: &dyn Command) -> bool {
        match next.as_any().downcast_ref::<UpdateField>() {
            Some(n) if self.mergeable && n.mergeable && n.after.id == self.after.id => {
                self.after = n.after.clone();
                true
            }
            _ => false,
        }
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

    /// Wie `execute`, verschmilzt das Command aber mit dem letzten, wenn beide das erlauben.
    pub fn execute_merging(&mut self, mut cmd: Box<dyn Command>, model: &mut Model) -> Result<()> {
        // Hinweis: Match-Guards (`if` in einem `match`-Arm) dürfen nichts verändern –
        // `merge` braucht `&mut`, deshalb hier ein normales `if`.
        let merged = self.redo.is_empty()
            && self
                .undo
                .last_mut()
                .is_some_and(|last| last.merge(cmd.as_ref()));
        if merged {
            cmd.apply(model)?;
        }
        if !merged {
            return self.execute(cmd, model);
        }
        // Der oberste Schritt hat sich verändert: War genau er gespeichert, ist das jetzt anders.
        if self.saved_at == Some(self.undo.len()) {
            self.saved_at = None;
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

    /// Gilt als "ungespeichert", bis zum nächsten Speichern (z. B. nach dem Schwärzen).
    pub fn mark_unsaved(&mut self) {
        self.saved_at = None;
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
    fn mergeable_updates_become_one_step() {
        let mut m = Model::default();
        let mut h = History::new();
        let a = note(&mut m);
        m.annotations.push(a.clone());
        let mut current = a.clone();
        for c in [Color::RED, Color::GREEN, Color::BLUE] {
            let mut next = current.clone();
            next.color = c;
            let cmd = UpdateAnnotation::new(current.clone(), next.clone()).mergeable();
            h.execute_merging(Box::new(cmd), &mut m).unwrap();
            current = next;
        }
        assert_eq!(m.annotations.get(a.id).unwrap().color, Color::BLUE);
        h.undo(&mut m).unwrap();
        assert_eq!(
            m.annotations.get(a.id).unwrap().color,
            a.color,
            "ein Schritt zurück = Ursprung"
        );
        assert!(h.undo_label().is_none());
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
