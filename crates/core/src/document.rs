//! Das geöffnete Dokument: Originalbytes, lopdf-Struktur und Seiteninformationen.

use crate::annotation::{self, AnnotId, Annotation, AnnotationStore, Source};
use crate::command::{Command, History, Model};
use crate::error::{CoreError, Result};
use crate::form::{self, FormField, FormStore};
use crate::geometry::{Point, Rect};
use crate::save;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Stabile Kennung einer Seite innerhalb einer Sitzung. Anders als der Seitenindex ändert
/// sie sich beim Umsortieren nicht – die App erkennt daran schon gerenderte Seiten wieder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PageId(pub u32);

/// Infos über eine Seite, ermittelt aus der PDF-Struktur.
#[derive(Debug, Clone, PartialEq)]
pub struct PageInfo {
    pub id: PageId,
    /// Objekt-ID des Seiten-Dictionarys (wird beim Speichern gebraucht).
    pub object_id: ObjectId,
    /// Sichtbarer Bereich (CropBox, sonst MediaBox) im unrotierten Benutzerraum.
    pub crop_box: Rect,
    /// Seitenrotation im Uhrzeigersinn: 0, 90, 180 oder 270.
    pub rotation: u16,
}

impl PageInfo {
    /// Größe so, wie die Seite angezeigt wird (bei 90°/270° sind Breite und Höhe vertauscht).
    pub fn display_size(&self) -> (f32, f32) {
        let (w, h) = (self.crop_box.width(), self.crop_box.height());
        if self.rotation % 180 == 90 {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Wandelt einen Punkt im PDF-Raum in normierte Anzeige-Koordinaten (0..1, Ursprung
    /// OBEN links, wie auf dem Bildschirm) um – inklusive /Rotate.
    pub fn pdf_to_display_unit(&self, p: Point) -> Point {
        let cb = self.crop_box;
        // Zuerst normieren im unrotierten Raum, y nach unten.
        let u = (p.x - cb.x0) / cb.width();
        let v = (cb.y1 - p.y) / cb.height();
        // Dann im Uhrzeigersinn drehen.
        match self.rotation {
            90 => Point::new(1.0 - v, u),
            180 => Point::new(1.0 - u, 1.0 - v),
            270 => Point::new(v, 1.0 - u),
            _ => Point::new(u, v),
        }
    }

    /// Umkehrung von [`Self::pdf_to_display_unit`].
    pub fn display_unit_to_pdf(&self, d: Point) -> Point {
        let (u, v) = match self.rotation {
            90 => (d.y, 1.0 - d.x),
            180 => (1.0 - d.x, 1.0 - d.y),
            270 => (1.0 - d.y, d.x),
            _ => (d.x, d.y),
        };
        let cb = self.crop_box;
        Point::new(cb.x0 + u * cb.width(), cb.y1 - v * cb.height())
    }
}

/// Ein geöffnetes PDF-Dokument samt bearbeitbarem Modell und Undo-Historie.
pub struct Document {
    path: Option<PathBuf>,
    /// Aktuelle Dateibytes. `Arc` = geteilter, unveränderlicher Besitz: Der Render-Thread
    /// bekommt eine billige Kopie des Zeigers statt einer Kopie der Daten.
    bytes: Arc<Vec<u8>>,
    pdf: lopdf::Document,
    model: Model,
    /// Seiten (Objekt-ID, Rotation) so, wie sie in der Datei stehen – zum Erkennen von
    /// Änderungen am Seitenbaum beim Speichern.
    saved_pages: Vec<(ObjectId, u16)>,
    /// Nächste freie Objektnummer für neue Objekte. Zählt nur hoch – so kollidieren
    /// rückgängig gemachte und später wiederhergestellte Einfügungen nie miteinander.
    next_object_id: u32,
    history: History,
    /// Stand der verwalteten Annotationen in der Datei (nach Laden/Speichern),
    /// nach Objektnummer – zum Erkennen von Änderungen beim Speichern.
    imported: HashMap<ObjectId, Annotation>,
    /// Dasselbe für Formularfelder (nach Objektnummer des Feld-Dictionarys).
    imported_fields: HashMap<ObjectId, FormField>,
    /// Stempel der bearbeiteten Seiteninhalte, wie sie in der Datei stehen.
    saved_content: HashMap<ObjectId, u64>,
    /// Passwortschutz (verschlüsselte Datei bzw. neu gesetzter Schutz). `bytes` und `pdf` sind
    /// immer Klartext; verschlüsselt wird erst beim Schreiben der Datei.
    security: Option<crate::security::Security>,
    /// Titel/Sprache und Alternativtexte, wie sie in der Datei stehen (Änderungen erkennen).
    saved_properties: crate::tags::DocProperties,
    saved_alts: std::collections::BTreeMap<ObjectId, Option<String>>,
}

impl Document {
    /// Lädt ein PDF von der Festplatte.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_password(path, None)
    }

    /// Lädt ein PDF, das evtl. ein Passwort braucht. Ohne Passwort und bei Bedarf:
    /// `CoreError::PasswordRequired`, bei falschem Passwort `CoreError::WrongPassword`.
    pub fn open_with_password(path: impl AsRef<Path>, password: Option<&str>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut doc = Self::from_bytes_with_password(bytes, password)?;
        doc.path = Some(path.to_path_buf());
        Ok(doc)
    }

    /// Lädt ein PDF aus dem Speicher (z. B. in Tests).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::from_bytes_with_password(bytes, None)
    }

    /// Wie [`Document::from_bytes`], mit Passwort für verschlüsselte Dateien.
    pub fn from_bytes_with_password(bytes: Vec<u8>, password: Option<&str>) -> Result<Self> {
        let (pdf, security) = crate::security::load(&bytes, password)?;
        // Verschlüsselt: ab jetzt auf einer Klartext-Fassung arbeiten (siehe `security`).
        let (bytes, pdf) = if security.is_some() {
            let plain = crate::security::plain_bytes(&pdf)?;
            let pdf = lopdf::Document::load_mem(&plain)?;
            (plain, pdf)
        } else {
            (bytes, pdf)
        };
        let mut model = Model::default();
        model.pages = read_pages(&pdf)?
            .into_iter()
            .map(|mut p| {
                p.id = model.allocate_page_id();
                p
            })
            .collect();
        let mut doc = Self {
            path: None,
            bytes: Arc::new(bytes),
            pdf,
            saved_pages: Vec::new(),
            next_object_id: 0,
            model,
            history: History::new(),
            imported: HashMap::new(),
            imported_fields: HashMap::new(),
            saved_content: HashMap::new(),
            security,
            saved_properties: Default::default(),
            saved_alts: Default::default(),
        };
        doc.model.properties = crate::tags::read_properties(&doc.pdf);
        doc.saved_properties = doc.model.properties.clone();
        doc.saved_pages = doc.page_structure();
        doc.next_object_id = doc.pdf.max_id + 1;
        doc.import_annotations();
        for f in form::import::read(&doc.pdf, &doc.model.pages) {
            doc.model.forms.push(f);
        }
        doc.imported_fields = snapshot_fields(&doc.model.forms);
        tracing::debug!(
            pages = doc.model.pages.len(),
            annotations = doc.model.annotations.len(),
            fields = doc.model.forms.len(),
            "Dokument geladen"
        );
        Ok(doc)
    }

    fn import_annotations(&mut self) {
        for (page_idx, info) in self.model.pages.iter().enumerate() {
            for entry in annotation::import::page_annots(&self.pdf, info.object_id) {
                let Object::Reference(id) = entry else {
                    continue;
                };
                if let Some(a) = annotation::import::parse(&self.pdf, page_idx, id) {
                    let new_id = self.model.annotations.push(a);
                    if let Some(a) = self.model.annotations.get(new_id) {
                        self.imported.insert(id, a.clone());
                    }
                }
            }
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Löst das Dokument von seiner Datei (z. B. nach dem Zusammenführen): Das nächste
    /// Speichern fragt dann nach einem neuen Namen.
    pub fn forget_path(&mut self) {
        self.path = None;
    }

    /// Erzeugt ein Dokument aus neu geschriebenen Bytes (z. B. nach dem Schwärzen), das zur
    /// bisherigen Datei gehört, aber noch nicht gespeichert ist.
    pub fn replacement(bytes: Vec<u8>, path: Option<PathBuf>) -> Result<Self> {
        let mut doc = Self::from_bytes(bytes)?;
        doc.path = path;
        doc.history.mark_unsaved();
        Ok(doc)
    }

    /// Passwortschutz des Dokuments (`None` = ungeschützt).
    pub fn security(&self) -> Option<&crate::security::Security> {
        self.security.as_ref()
    }

    /// Prüft das Besitzerpasswort eines geschützten Dokuments (volle Rechte).
    pub fn unlock(&mut self, owner_password: &str) -> bool {
        self.security
            .as_mut()
            .is_some_and(|s| s.unlock(owner_password))
    }

    /// Setzt einen neuen Schutz (`Some`) oder entfernt ihn (`None`). Wirkt beim nächsten
    /// Speichern. Ändern darf nur, wer volle Rechte hat (ungeschützt oder Besitzerpasswort).
    pub fn set_protection(
        &mut self,
        protection: Option<&crate::security::Protection>,
    ) -> Result<()> {
        if self.security.as_ref().is_some_and(|s| !s.owner_known) {
            return Err(CoreError::Other(
                "Zum Ändern des Schutzes wird das Besitzerpasswort benötigt.".into(),
            ));
        }
        self.security = protection.map(crate::security::Security::new).transpose()?;
        self.history.mark_unsaved();
        Ok(())
    }

    /// Übernimmt den Schutz eines anderen Dokuments (z. B. für eine neu geschriebene Fassung).
    pub fn inherit_security(&mut self, other: &Document) {
        self.security = other.security.clone();
    }

    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
    }

    pub fn pdf(&self) -> &lopdf::Document {
        &self.pdf
    }

    /// Seiten in der aktuellen Reihenfolge (inkl. noch nicht gespeicherter Änderungen).
    pub fn pages(&self) -> &[PageInfo] {
        &self.model.pages
    }

    pub fn page(&self, index: usize) -> Result<&PageInfo> {
        self.model
            .pages
            .get(index)
            .ok_or(CoreError::PageOutOfRange(index))
    }

    pub fn page_count(&self) -> usize {
        self.model.pages.len()
    }

    /// Stabile IDs aller Seiten in aktueller Reihenfolge.
    pub fn page_ids(&self) -> Vec<PageId> {
        self.model.pages.iter().map(|p| p.id).collect()
    }

    /// (Objekt-ID, Rotation) aller Seiten – vergleicht Modell und Datei.
    fn page_structure(&self) -> Vec<(ObjectId, u16)> {
        self.model
            .pages
            .iter()
            .map(|p| (p.object_id, p.rotation))
            .collect()
    }

    /// Stand der Seiten in der Datei (für `save`).
    pub(crate) fn saved_pages(&self) -> &[(ObjectId, u16)] {
        &self.saved_pages
    }

    // ---------- Modell & Undo/Redo ----------

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn annotations(&self) -> &AnnotationStore {
        &self.model.annotations
    }

    pub fn forms(&self) -> &FormStore {
        &self.model.forms
    }

    /// Seitenindex zur Objektnummer einer Seite (z. B. für Widgets).
    pub fn page_index_of(&self, page: ObjectId) -> Option<usize> {
        self.model.pages.iter().position(|p| p.object_id == page)
    }

    pub(crate) fn allocate_page_id(&mut self) -> PageId {
        self.model.allocate_page_id()
    }

    pub(crate) fn next_free_object_id(&self) -> u32 {
        self.next_object_id
    }

    /// Markiert alle Objektnummern unterhalb von `next` als vergeben.
    pub(crate) fn reserve_object_ids(&mut self, next: u32) {
        self.next_object_id = self.next_object_id.max(next);
    }

    /// Ein Objekt aus der Datei oder (falls noch nicht gespeichert) aus dem Modell.
    pub fn object(&self, id: ObjectId) -> Option<&Object> {
        self.model
            .extra_objects
            .get(&id)
            .or_else(|| self.pdf.get_object(id).ok())
    }

    /// Neue ID für eine Annotation, die gleich per Command hinzugefügt wird.
    pub fn allocate_annotation_id(&mut self) -> AnnotId {
        self.model.annotations.allocate_id()
    }

    /// Führt eine Änderung aus (der einzige Weg, das Modell zu verändern).
    pub fn execute(&mut self, cmd: Box<dyn Command>) -> Result<()> {
        // `history` und `model` sind getrennte Felder – Rust erlaubt, beide
        // gleichzeitig veränderlich auszuleihen.
        self.history.execute(cmd, &mut self.model)
    }

    /// Wie `execute`, fasst aber aufeinanderfolgende zusammenführbare Änderungen zusammen.
    pub fn execute_merging(&mut self, cmd: Box<dyn Command>) -> Result<()> {
        self.history.execute_merging(cmd, &mut self.model)
    }

    pub fn undo(&mut self) -> Result<bool> {
        self.history.undo(&mut self.model)
    }

    pub fn redo(&mut self) -> Result<bool> {
        self.history.redo(&mut self.model)
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// Positionen im /Annots-Array einer Seite, die PDF Tool selbst verwaltet (und
    /// daher als Overlay zeichnet): Annotationen und Widgets ausfüllbarer Formularfelder.
    /// Der Renderer blendet genau diese aus.
    pub fn managed_annot_indices(&self, page: usize) -> Vec<usize> {
        let Some(info) = self.model.pages.get(page) else {
            return Vec::new();
        };
        let widgets: std::collections::HashSet<ObjectId> =
            self.model.forms.managed_widget_ids().collect();
        annotation::import::page_annots(&self.pdf, info.object_id)
            .iter()
            .enumerate()
            .filter(|(_, o)| {
                o.as_reference()
                    .is_ok_and(|id| self.imported.contains_key(&id) || widgets.contains(&id))
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub(crate) fn imported_fields(&self) -> impl Iterator<Item = &FormField> {
        self.imported_fields.values()
    }

    /// Neue ID für ein Formularfeld, das gleich per Command hinzugefügt wird.
    pub fn allocate_field_id(&mut self) -> form::FieldId {
        self.model.forms.allocate_id()
    }

    /// Stand eines Formularfelds in der Datei (für `save`).
    pub(crate) fn imported_field(&self, id: ObjectId) -> Option<&FormField> {
        self.imported_fields.get(&id)
    }

    pub(crate) fn saved_properties(&self) -> crate::tags::DocProperties {
        self.saved_properties.clone()
    }

    /// Alternativtext eines Elements, wie er zuletzt gespeichert wurde (`None` = nie geändert).
    pub(crate) fn saved_alt(&self, elem: ObjectId) -> Option<&Option<String>> {
        self.saved_alts.get(&elem)
    }

    /// Wurde der Inhalt dieser Seite seit dem Laden/Speichern geändert?
    pub(crate) fn content_changed(&self, page: ObjectId) -> bool {
        self.model
            .page_content
            .get(&page)
            .is_some_and(|c| self.saved_content.get(&page) != Some(&c.stamp))
    }

    pub(crate) fn imported(&self) -> impl Iterator<Item = &Annotation> {
        self.imported.values()
    }

    pub(crate) fn is_imported(&self, id: ObjectId) -> bool {
        self.imported.contains_key(&id)
    }

    pub(crate) fn imported_snapshot(&self, id: ObjectId) -> Option<&Annotation> {
        self.imported.get(&id)
    }

    // ---------- Speichern ----------

    /// Erzeugt die Bytes einer inkrementell aktualisierten Datei, ohne etwas zu verändern.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(save::incremental(self)?.bytes)
    }

    /// Momentaufnahme für den Renderer nach strukturellen Änderungen (Seiten gedreht,
    /// gelöscht, umsortiert): Bytes des aktuellen Stands plus die Annotationen, die der
    /// Renderer ausblenden soll (Seitenindex -> Positionen in /Annots).
    pub fn render_snapshot(&self) -> Result<RenderSnapshot> {
        let out = save::incremental(self)?;
        Ok(RenderSnapshot {
            bytes: out.bytes,
            hidden: out.hidden,
        })
    }

    /// Speichert an den bisherigen Pfad.
    pub fn save(&mut self) -> Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| CoreError::Other("Dokument hat noch keinen Dateipfad".into()))?;
        self.save_as(path)
    }

    /// Speichert unter einem (neuen) Pfad. Danach gilt dieser Pfad als Dokumentpfad.
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let out = save::incremental(self)?;
        match &self.security {
            Some(sec) if !sec.can_save() => {
                return Err(CoreError::Other(
                    "Das Dokument erlaubt keine Änderungen. Zum Speichern bitte das \
                     Besitzerpasswort eingeben (Datei → Sicherheit…)."
                        .into(),
                ));
            }
            // Datei verschlüsselt schreiben; im Speicher bleibt der Klartext.
            Some(sec) => write_atomically(path, &sec.encrypt(&out.bytes)?)?,
            None => write_atomically(path, &out.bytes)?,
        }
        self.after_save(out)?;
        self.path = Some(path.to_path_buf());
        tracing::info!(path = %path.display(), "gespeichert");
        Ok(())
    }

    /// Übernimmt den gespeicherten Stand: neue Bytes, neue Objektnummern, "nicht geändert".
    fn after_save(&mut self, out: save::SaveOutput) -> Result<()> {
        self.pdf = lopdf::Document::load_mem(&out.bytes)?;
        self.bytes = Arc::new(out.bytes);
        for (id, object_id, name, modified) in out.written {
            if let Some(a) = self.model.annotations.get(id) {
                let mut a = a.clone();
                let popup = a.source.and_then(|s| s.popup);
                a.source = Some(Source { object_id, popup });
                a.name = name;
                a.modified = modified;
                self.model.annotations.replace(a);
            }
        }
        self.imported = self
            .model
            .annotations
            .iter()
            .filter_map(|a| Some((a.source?.object_id, a.clone())))
            .collect();
        // Neu angelegte Formularobjekte: Objektnummern ins Modell übernehmen.
        for (id, object_id, widgets) in out.written_fields {
            if let Some(f) = self.model.forms.get(id) {
                let mut f = f.clone();
                f.source = Some(object_id);
                for (w, wid) in f.widgets.iter_mut().zip(widgets) {
                    w.object_id = w.object_id.or(wid);
                }
                self.model.forms.replace(f);
            }
        }
        self.imported_fields = snapshot_fields(&self.model.forms);
        self.saved_pages = self.page_structure();
        // Neue Bilder stehen jetzt in der Datei (die Ressourcen der Seite verweisen darauf).
        // Aus dem Modell nehmen, damit sie beim nächsten Speichern nicht erneut angehängt werden.
        for pc in self.model.page_content.values_mut() {
            pc.images.clear();
        }
        self.saved_properties = self.model.properties.clone();
        self.saved_alts = self.model.alt_texts.clone();
        self.saved_content = self
            .model
            .page_content
            .iter()
            .map(|(&id, c)| (id, c.stamp))
            .collect();
        // Die zusätzlichen Objekte stehen jetzt in der Datei.
        self.model.extra_objects.clear();
        self.next_object_id = self.next_object_id.max(self.pdf.max_id + 1);
        self.history.mark_saved();
        Ok(())
    }
}

/// Felder mit Herkunft in der Datei, nach Objektnummer.
fn snapshot_fields(forms: &FormStore) -> HashMap<ObjectId, FormField> {
    forms
        .iter()
        .filter_map(|f| Some((f.source?, f.clone())))
        .collect()
}

/// Siehe [`Document::render_snapshot`].
pub struct RenderSnapshot {
    pub bytes: Vec<u8>,
    pub hidden: HashMap<u32, Vec<usize>>,
}

/// Schreibt erst in eine temporäre Datei und benennt sie dann um. So bleibt bei einem
/// Absturz mitten im Schreiben die alte Datei unversehrt.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let io_err = |source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".pdftool-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(io_err)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(e)
    })
}

fn read_pages(pdf: &lopdf::Document) -> Result<Vec<PageInfo>> {
    // get_pages liefert eine BTreeMap Seitennummer (1-basiert) -> Objekt-ID, sortiert.
    pdf.get_pages()
        .into_values()
        .map(|id| {
            let media = inherited(pdf, id, b"MediaBox")
                .and_then(|o| rect_from_object(pdf, o))
                .unwrap_or(Rect::new(0.0, 0.0, 612.0, 792.0)); // Standard laut Spezifikation: Letter
            let crop = inherited(pdf, id, b"CropBox")
                .and_then(|o| rect_from_object(pdf, o))
                .map(|c| intersect(c, media))
                .unwrap_or(media);
            let rotation = inherited(pdf, id, b"Rotate")
                .and_then(|o| pdf.dereference(o).ok())
                .and_then(|(_, o)| o.as_i64().ok())
                .unwrap_or(0)
                .rem_euclid(360) as u16;
            // Nur Vielfache von 90 sind erlaubt; alles andere behandeln wir als 0.
            let rotation = if rotation % 90 == 0 { rotation } else { 0 };
            Ok(PageInfo {
                id: PageId(0), // wird vom Aufrufer vergeben
                object_id: id,
                crop_box: crop,
                rotation,
            })
        })
        .collect()
}

/// Sucht ein vererbbares Seitenattribut: erst auf der Seite, dann entlang /Parent.
///
/// Die Lifetime `'a` sagt: Das zurückgegebene `&Object` lebt höchstens so lange
/// wie das ausgeliehene `pdf` – es zeigt ja in dessen Daten.
pub(crate) fn inherited<'a>(
    pdf: &'a lopdf::Document,
    page: ObjectId,
    key: &[u8],
) -> Option<&'a Object> {
    let mut current: &Dictionary = pdf.get_dictionary(page).ok()?;
    // Begrenzung schützt vor Endlosschleifen in kaputten PDFs mit zyklischen /Parent-Ketten.
    for _ in 0..64 {
        if let Ok(v) = current.get(key) {
            return Some(v);
        }
        let parent = current.get(b"Parent").ok()?.as_reference().ok()?;
        current = pdf.get_dictionary(parent).ok()?;
    }
    None
}

fn rect_from_object(pdf: &lopdf::Document, obj: &Object) -> Option<Rect> {
    let (_, obj) = pdf.dereference(obj).ok()?;
    let arr = obj.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    let mut v = [0.0f32; 4];
    for (slot, o) in v.iter_mut().zip(arr) {
        let (_, o) = pdf.dereference(o).ok()?;
        *slot = o.as_float().ok()?;
    }
    let r = Rect::new(v[0], v[1], v[2], v[3]).normalized();
    (r.width() > 0.0 && r.height() > 0.0).then_some(r)
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let r = Rect::new(
        a.x0.max(b.x0),
        a.y0.max(b.y0),
        a.x1.min(b.x1),
        a.y1.min(b.y1),
    );
    if r.width() > 0.0 && r.height() > 0.0 {
        r
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn opens_fixture_and_reads_pages() {
        let doc = Document::open(fixture("mixed_sizes.pdf")).unwrap();
        assert_eq!(doc.page_count(), 3);
        assert_eq!(doc.pages()[0].display_size(), (595.0, 842.0));
        assert_eq!(doc.pages()[1].display_size(), (792.0, 612.0));
    }

    #[test]
    fn reads_rotation() {
        let doc = Document::open(fixture("rotated.pdf")).unwrap();
        let rot: Vec<u16> = doc.pages().iter().map(|p| p.rotation).collect();
        assert_eq!(rot, vec![0, 90, 180]);
        // A4 mit 90° wird quer angezeigt.
        assert_eq!(doc.pages()[1].display_size(), (842.0, 595.0));
    }

    #[test]
    fn display_transform_roundtrip_all_rotations() {
        for rotation in [0, 90, 180, 270] {
            let page = PageInfo {
                id: PageId(1),
                object_id: (1, 0),
                crop_box: Rect::new(0.0, 0.0, 600.0, 800.0),
                rotation,
            };
            let p = Point::new(100.0, 700.0);
            let d = page.pdf_to_display_unit(p);
            let back = page.display_unit_to_pdf(d);
            assert!(
                (back.x - p.x).abs() < 1e-3 && (back.y - p.y).abs() < 1e-3,
                "rot {rotation}"
            );
        }
        // Oben links im unrotierten Raum landet bei 90° oben rechts in der Anzeige.
        let page = PageInfo {
            id: PageId(1),
            object_id: (1, 0),
            crop_box: Rect::new(0.0, 0.0, 600.0, 800.0),
            rotation: 90,
        };
        let d = page.pdf_to_display_unit(Point::new(0.0, 800.0));
        assert_eq!((d.x, d.y), (1.0, 0.0));
    }

    #[test]
    fn invalid_pdf_is_an_error() {
        assert!(Document::from_bytes(b"kein pdf".to_vec()).is_err());
    }
}
