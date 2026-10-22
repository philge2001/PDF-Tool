//! Das geöffnete Dokument: Originalbytes, lopdf-Struktur und Seiteninformationen.

use crate::annotation::{self, AnnotId, Annotation, AnnotationStore, Source};
use crate::command::{Command, History, Model};
use crate::error::{CoreError, Result};
use crate::geometry::{Point, Rect};
use crate::save;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Infos über eine Seite, ermittelt aus der PDF-Struktur.
#[derive(Debug, Clone, PartialEq)]
pub struct PageInfo {
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
    pages: Vec<PageInfo>,
    model: Model,
    history: History,
    /// Stand der verwalteten Annotationen in der Datei (nach Laden/Speichern),
    /// nach Objektnummer – zum Erkennen von Änderungen beim Speichern.
    imported: HashMap<ObjectId, Annotation>,
}

impl Document {
    /// Lädt ein PDF von der Festplatte.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut doc = Self::from_bytes(bytes)?;
        doc.path = Some(path.to_path_buf());
        Ok(doc)
    }

    /// Lädt ein PDF aus dem Speicher (z. B. in Tests).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let pdf = lopdf::Document::load_mem(&bytes)?;
        let pages = read_pages(&pdf)?;
        let mut doc = Self {
            path: None,
            bytes: Arc::new(bytes),
            pdf,
            pages,
            model: Model::default(),
            history: History::new(),
            imported: HashMap::new(),
        };
        doc.import_annotations();
        tracing::debug!(
            pages = doc.pages.len(),
            annotations = doc.model.annotations.len(),
            "Dokument geladen"
        );
        Ok(doc)
    }

    fn import_annotations(&mut self) {
        for (page_idx, info) in self.pages.iter().enumerate() {
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

    pub fn bytes(&self) -> &Arc<Vec<u8>> {
        &self.bytes
    }

    pub fn pdf(&self) -> &lopdf::Document {
        &self.pdf
    }

    pub fn pages(&self) -> &[PageInfo] {
        &self.pages
    }

    pub fn page(&self, index: usize) -> Result<&PageInfo> {
        self.pages
            .get(index)
            .ok_or(CoreError::PageOutOfRange(index))
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    // ---------- Modell & Undo/Redo ----------

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn annotations(&self) -> &AnnotationStore {
        &self.model.annotations
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
        write_atomically(path, &out.bytes)?;
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
        self.history.mark_saved();
        Ok(())
    }
}

/// Schreibt erst in eine temporäre Datei und benennt sie dann um. So bleibt bei einem
/// Absturz mitten im Schreiben die alte Datei unversehrt.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let io_err = |source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    };
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".foliocraft-tmp");
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
fn inherited<'a>(pdf: &'a lopdf::Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
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
