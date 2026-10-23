//! Der Render-Worker: ein eigener Thread, der PDFium exklusiv besitzt.
//!
//! Die UI schickt [`Job`]s über einen Channel und bekommt [`Event`]s zurück.
//! Nach jedem Ereignis ruft der Worker den `waker` auf (in der App: `ctx.request_repaint()`),
//! damit die UI das Ergebnis sofort abholt.

use crate::error::{RenderError, Result};
use crate::loader;
use pdfium_render::prelude::*;
use pdftool_core::geometry::Rect;
use pdftool_core::text::{CharBox, PageText};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

/// Kennung eines geöffneten Dokuments. Wird bei jedem Öffnen hochgezählt, damit
/// verspätete Ergebnisse zu einem alten Dokument erkannt und verworfen werden.
pub type DocId = u64;

/// Kantenlänge einer Kachel in Pixeln.
pub const TILE_SIZE: u32 = 1024;

/// Seiten, deren längere Kante bei der gewählten Skalierung größer ist, werden in Kacheln gerendert.
pub const MAX_FULL_PAGE_PX: u32 = 3072;

/// Eindeutiger Schlüssel eines Render-Ergebnisses. `Hash + Eq` erlaubt die Nutzung in HashMaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderKey {
    pub doc: DocId,
    pub page: u32,
    /// Skalierung (Pixel pro PDF-Punkt) mal 1000 – als Ganzzahl, damit sie hashbar ist.
    pub scale_milli: u32,
    /// `None` = ganze Seite, `Some((spalte, zeile))` = eine Kachel.
    pub tile: Option<(u32, u32)>,
}

impl RenderKey {
    pub fn scale(&self) -> f32 {
        self.scale_milli as f32 / 1000.0
    }
}

/// Je kleiner, desto dringender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Visible = 0,
    Prefetch = 1,
    Thumbnail = 2,
}

#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub key: RenderKey,
    /// Anzeigegröße der Seite in Punkt (nach /Rotate) – damit der Worker sie nicht erneut ermitteln muss.
    pub page_size_pt: (f32, f32),
    pub priority: Priority,
}

/// Ein fertig gerendertes Bild, Pixel als RGBA (4 Bytes pro Pixel, Zeile für Zeile).
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Nachrichten vom Worker an die UI.
pub enum Event {
    Opened {
        doc: DocId,
        page_count: usize,
    },
    OpenFailed {
        doc: DocId,
        error: String,
    },
    Rendered {
        key: RenderKey,
        image: RgbaImage,
    },
    RenderFailed {
        key: RenderKey,
        error: String,
    },
    /// Text einer Seite (mit Zeichenpositionen). Bei Fehlern leer.
    Text {
        doc: DocId,
        page: u32,
        text: PageText,
    },
}

/// Nachrichten von der UI an den Worker (intern).
enum Job {
    Open {
        doc: DocId,
        bytes: Arc<Vec<u8>>,
        hidden: HiddenAnnots,
    },
    Close,
    Render(RenderRequest),
    ExtractText {
        doc: DocId,
        page: u32,
    },
    /// Verwirft alle wartenden Aufträge, deren Schlüssel nicht in der Menge ist.
    Retain(HashSet<RenderKey>),
    /// Seite eines beliebigen Dokuments rendern und direkt an `reply` antworten
    /// (für Hintergrund-Jobs wie die Texterkennung).
    Offscreen(OffscreenJob),
}

struct OffscreenJob {
    bytes: Arc<Vec<u8>>,
    page: u32,
    kind: OffscreenKind,
}

enum OffscreenKind {
    Render {
        page_size_pt: (f32, f32),
        scale: f32,
        reply: Sender<Result<RgbaImage>>,
    },
    /// Text der Seite in Lesereihenfolge (wie PDFium ihn liefert, mit Zeilenumbrüchen).
    Text { reply: Sender<Result<String>> },
    /// Zeichen mit Rechtecken (wie `Event::Text`).
    Chars { reply: Sender<Result<PageText>> },
}

/// Rendert Seiten für Hintergrund-Jobs. Klonbar und `Send`: Jeder Job-Thread bekommt eine
/// Kopie. Die Anfrage läuft trotzdem im Render-Thread (PDFium nur dort), der Aufrufer wartet.
#[derive(Clone)]
pub struct OffscreenRenderer {
    jobs: Sender<Job>,
}

impl OffscreenRenderer {
    /// Rendert eine ganze Seite (ohne Annotationen und Formularinhalte) in `scale` Pixeln
    /// pro Punkt. `page_size_pt` = Anzeigegröße (nach /Rotate). Blockiert bis zum Ergebnis.
    /// Dasselbe `bytes`-Arc mehrfach zu übergeben spart das erneute Laden des Dokuments.
    pub fn render_page(
        &self,
        bytes: &Arc<Vec<u8>>,
        page: u32,
        page_size_pt: (f32, f32),
        scale: f32,
    ) -> Result<RgbaImage> {
        let (reply, answer) = mpsc::channel();
        self.send(
            bytes,
            page,
            OffscreenKind::Render {
                page_size_pt,
                scale,
                reply,
            },
        )?;
        answer.recv().map_err(|_| RenderError::WorkerGone)?
    }

    /// Text einer Seite (Lesereihenfolge, Zeilen mit `\n` getrennt). Blockiert.
    pub fn page_text(&self, bytes: &Arc<Vec<u8>>, page: u32) -> Result<String> {
        let (reply, answer) = mpsc::channel();
        self.send(bytes, page, OffscreenKind::Text { reply })?;
        answer.recv().map_err(|_| RenderError::WorkerGone)?
    }

    /// Zeichen einer Seite mit Rechtecken (PDF-Raum). Blockiert.
    pub fn page_chars(&self, bytes: &Arc<Vec<u8>>, page: u32) -> Result<PageText> {
        let (reply, answer) = mpsc::channel();
        self.send(bytes, page, OffscreenKind::Chars { reply })?;
        answer.recv().map_err(|_| RenderError::WorkerGone)?
    }

    fn send(&self, bytes: &Arc<Vec<u8>>, page: u32, kind: OffscreenKind) -> Result<()> {
        self.jobs
            .send(Job::Offscreen(OffscreenJob {
                bytes: Arc::clone(bytes),
                page,
                kind,
            }))
            .map_err(|_| RenderError::WorkerGone)
    }
}

/// Pro Seite: Indizes im /Annots-Array, die beim Rendern ausgeblendet werden
/// (weil die App sie selbst als Overlay zeichnet).
pub type HiddenAnnots = HashMap<u32, Vec<usize>>;

/// Pixelgröße einer Seite bei gegebener Skalierung. App und Worker nutzen dieselbe
/// Funktion, damit Kachelraster und Bildgrößen garantiert übereinstimmen.
pub fn page_pixel_size(page_size_pt: (f32, f32), scale: f32) -> (u32, u32) {
    (
        ((page_size_pt.0 * scale).round() as u32).max(1),
        ((page_size_pt.1 * scale).round() as u32).max(1),
    )
}

/// Muss die Seite bei dieser Pixelgröße gekachelt werden?
pub fn needs_tiles(page_px: (u32, u32)) -> bool {
    page_px.0.max(page_px.1) > MAX_FULL_PAGE_PX
}

/// Anzahl Kacheln (Spalten, Zeilen) für eine Seite.
pub fn tile_grid(page_px: (u32, u32)) -> (u32, u32) {
    (page_px.0.div_ceil(TILE_SIZE), page_px.1.div_ceil(TILE_SIZE))
}

/// Pixelrechteck (x, y, w, h) einer Kachel innerhalb der Seite.
pub fn tile_rect(page_px: (u32, u32), tile: (u32, u32)) -> (u32, u32, u32, u32) {
    let x = tile.0 * TILE_SIZE;
    let y = tile.1 * TILE_SIZE;
    (
        x,
        y,
        TILE_SIZE.min(page_px.0 - x),
        TILE_SIZE.min(page_px.1 - y),
    )
}

/// Handle auf den laufenden Worker. Wird es gedroppt, endet der Thread.
pub struct RenderService {
    // `Option`, damit wir den Sender in `Drop` vorher wegnehmen können (siehe unten).
    jobs: Option<Sender<Job>>,
    events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl RenderService {
    /// Startet den Worker und bindet PDFium. Schlägt fehl, wenn die Bibliothek fehlt.
    ///
    /// `waker` muss `Send + 'static` sein: Er wird in den anderen Thread verschoben
    /// und lebt dort unbegrenzt lange.
    pub fn start(waker: impl Fn() + Send + 'static) -> Result<Self> {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (ev_tx, ev_rx) = mpsc::channel::<Event>();
        // Ein extra Kanal, um das Ergebnis des PDFium-Ladens synchron zurückzumelden.
        let (init_tx, init_rx) = mpsc::channel::<Result<()>>();

        let thread = std::thread::Builder::new()
            .name("pdftool-render".into())
            // `move` verschiebt die Channel-Enden und den Waker in den Thread (Ownership-Übergabe).
            .spawn(move || match loader::bind_pdfium() {
                Ok(pdfium) => {
                    let _ = init_tx.send(Ok(()));
                    run(&pdfium, job_rx, ev_tx, waker);
                }
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                }
            })
            .map_err(|e| RenderError::Pdfium(format!("Thread-Start fehlgeschlagen: {e}")))?;

        init_rx.recv().map_err(|_| RenderError::WorkerGone)??;
        Ok(Self {
            jobs: Some(job_tx),
            events: ev_rx,
            thread: Some(thread),
        })
    }

    fn send(&self, job: Job) {
        if let Some(tx) = &self.jobs
            && tx.send(job).is_err()
        {
            tracing::error!("Render-Worker nicht erreichbar");
        }
    }

    /// Öffnet ein Dokument (ersetzt ein evtl. vorher geöffnetes).
    /// `hidden`: Annotationen, die nicht mitgerendert werden sollen.
    pub fn open(&self, doc: DocId, bytes: Arc<Vec<u8>>, hidden: HiddenAnnots) {
        self.send(Job::Open { doc, bytes, hidden });
    }

    pub fn close(&self) {
        self.send(Job::Close);
    }

    pub fn request(&self, req: RenderRequest) {
        self.send(Job::Render(req));
    }

    /// Fordert den Text einer Seite an (Antwort: [`Event::Text`]).
    pub fn extract_text(&self, doc: DocId, page: u32) {
        self.send(Job::ExtractText { doc, page });
    }

    /// Teilt dem Worker mit, welche Render-Aufträge noch gebraucht werden
    /// (Textaufträge sind davon nicht betroffen).
    pub fn retain(&self, wanted: HashSet<RenderKey>) {
        self.send(Job::Retain(wanted));
    }

    /// Handle für Hintergrund-Jobs (siehe [`OffscreenRenderer`]).
    pub fn offscreen(&self) -> Option<OffscreenRenderer> {
        self.jobs
            .as_ref()
            .map(|jobs| OffscreenRenderer { jobs: jobs.clone() })
    }

    /// Holt alle angekommenen Ereignisse ab, ohne zu blockieren.
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for RenderService {
    fn drop(&mut self) {
        // Sender droppen -> `recv()` im Worker liefert Err -> Schleife endet.
        self.jobs.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Hauptschleife des Workers.
///
/// Die Lifetime `'p` verknüpft das Dokument mit `pdfium`: Ein `PdfDocument<'p>` darf
/// nicht länger leben als die `Pdfium`-Instanz, von der es stammt. Der Compiler prüft das.
fn run<'p>(pdfium: &'p Pdfium, jobs: Receiver<Job>, events: Sender<Event>, waker: impl Fn()) {
    let mut doc: Option<OpenDoc<'p>> = None;
    let mut queues = Queues::default();
    // Zuletzt für Offscreen-Aufträge geladenes Dokument (erkannt am Arc-Zeiger).
    let mut offscreen_doc: Option<(Arc<Vec<u8>>, PdfDocument<'p>)> = None;

    loop {
        // Nichts zu tun? Dann blockierend auf den nächsten Auftrag warten (kostet keine CPU).
        if queues.is_empty() {
            match jobs.recv() {
                Ok(job) => handle_job(job, pdfium, &mut doc, &mut queues, &events, &waker),
                Err(_) => return, // App wurde beendet
            }
        }
        // Alle weiteren wartenden Aufträge einsammeln, damit wir priorisieren können.
        while let Ok(job) = jobs.try_recv() {
            handle_job(job, pdfium, &mut doc, &mut queues, &events, &waker);
        }
        // Offscreen-Aufträge brauchen kein geöffnetes Dokument der Ansicht.
        if queues.offscreen_is_next()
            && let Some(job) = queues.offscreen.pop_front()
        {
            run_offscreen(pdfium, &mut offscreen_doc, job);
            continue;
        }
        if queues.offscreen.is_empty() {
            offscreen_doc = None; // Speicher freigeben, wenn kein Job mehr rendert
        }
        let Some(d) = &mut doc else {
            // ohne Dokument gibt es nichts zu tun (Offscreen-Aufträge laufen oben weiter)
            queues.renders.clear();
            queues.texts.clear();
            continue;
        };
        let doc_id = d.id;

        let event = match queues.next() {
            Some(Work::Render(req)) if req.key.doc == doc_id => match d
                .hide_annotations(req.key.page)
                .and_then(|_| render(&d.pdf, &req))
            {
                Ok(image) => Event::Rendered {
                    key: req.key,
                    image,
                },
                Err(e) => Event::RenderFailed {
                    key: req.key,
                    error: e.to_string(),
                },
            },
            Some(Work::Text(id, page)) if id == doc_id => Event::Text {
                doc: id,
                page,
                text: extract_text(&d.pdf, page).unwrap_or_else(|e| {
                    tracing::warn!(page, error = %e, "Textextraktion fehlgeschlagen");
                    PageText::default()
                }),
            },
            _ => continue, // leer oder gehört zu einem alten Dokument
        };
        if events.send(event).is_err() {
            return;
        }
        waker();
    }
}

/// Das im Worker geöffnete Dokument.
struct OpenDoc<'p> {
    id: DocId,
    pdf: PdfDocument<'p>,
    hidden: HiddenAnnots,
}

impl OpenDoc<'_> {
    /// Entfernt die auszublendenden Annotationen einer Seite aus der PDFium-Kopie im
    /// Speicher (einmal pro Seite; die Datei selbst bleibt unberührt).
    fn hide_annotations(&mut self, page: u32) -> Result<()> {
        // `remove` liefert die Liste nur beim ersten Mal – danach ist die Seite erledigt.
        let Some(mut indices) = self.hidden.remove(&page) else {
            return Ok(());
        };
        let mut p = self.pdf.pages().get(page as PdfPageIndex)?;
        // Absteigend löschen, damit sich die übrigen Indizes nicht verschieben.
        indices.sort_unstable_by(|a, b| b.cmp(a));
        for i in indices {
            let annots = p.annotations_mut();
            match annots.get(i as PdfPageAnnotationIndex) {
                Ok(a) => annots.delete_annotation(a)?,
                Err(e) => tracing::warn!(page, index = i, error = %e, "Annotation nicht gefunden"),
            }
        }
        Ok(())
    }
}

/// Wartende Arbeit des Workers.
#[derive(Default)]
struct Queues {
    renders: Vec<RenderRequest>,
    texts: VecDeque<(DocId, u32)>,
    offscreen: VecDeque<OffscreenJob>,
}

enum Work {
    Render(RenderRequest),
    Text(DocId, u32),
}

impl Queues {
    fn is_empty(&self) -> bool {
        self.renders.is_empty() && self.texts.is_empty() && self.offscreen.is_empty()
    }

    /// Ansichts-Aufträge verwerfen (Offscreen-Aufträge warten auf Antwort und bleiben).
    fn clear(&mut self) {
        self.renders.clear();
        self.texts.clear();
    }

    /// Offscreen kommt nach sichtbaren Seiten und Text, aber vor Vorab-Rendern und Miniaturen.
    fn offscreen_is_next(&self) -> bool {
        !self.offscreen.is_empty()
            && self.texts.is_empty()
            && !self.renders.iter().any(|r| r.priority == Priority::Visible)
    }

    /// Reihenfolge: sichtbare Seiten > Text (Suche) > Vorab-Rendern > Miniaturen.
    /// Bei gleicher Priorität gewinnt der zuletzt angefragte Render-Auftrag
    /// (der Nutzer schaut vermutlich gerade dorthin).
    fn next(&mut self) -> Option<Work> {
        let best = self
            .renders
            .iter()
            .enumerate()
            .min_by_key(|(i, r)| (r.priority, std::cmp::Reverse(*i)))
            .map(|(i, r)| (i, r.priority));
        match best {
            Some((i, Priority::Visible)) => Some(Work::Render(self.renders.remove(i))),
            _ if !self.texts.is_empty() => self.texts.pop_front().map(|(d, p)| Work::Text(d, p)),
            Some((i, _)) => Some(Work::Render(self.renders.remove(i))),
            None => None,
        }
    }
}

fn handle_job<'p>(
    job: Job,
    pdfium: &'p Pdfium,
    doc: &mut Option<OpenDoc<'p>>,
    queues: &mut Queues,
    events: &Sender<Event>,
    waker: &impl Fn(),
) {
    match job {
        Job::Open {
            doc: id,
            bytes,
            hidden,
        } => {
            queues.clear();
            *doc = None; // altes Dokument zuerst freigeben
            // PDFium will einen eigenen Vec; wir kopieren die Bytes einmalig.
            let event = match pdfium.load_pdf_from_byte_vec(bytes.as_ref().clone(), None) {
                Ok(d) => {
                    let page_count = d.pages().len() as usize;
                    *doc = Some(OpenDoc { id, pdf: d, hidden });
                    Event::Opened {
                        doc: id,
                        page_count,
                    }
                }
                Err(e) => Event::OpenFailed {
                    doc: id,
                    error: e.to_string(),
                },
            };
            let _ = events.send(event);
            waker();
        }
        Job::Close => {
            queues.clear();
            *doc = None;
        }
        Job::Render(req) => {
            // Doppelte Aufträge zusammenführen, dabei die höhere Priorität behalten.
            if let Some(existing) = queues.renders.iter_mut().find(|r| r.key == req.key) {
                existing.priority = existing.priority.min(req.priority);
            } else {
                queues.renders.push(req);
            }
        }
        Job::ExtractText { doc, page } => {
            if !queues.texts.contains(&(doc, page)) {
                queues.texts.push_back((doc, page));
            }
        }
        Job::Retain(wanted) => queues.renders.retain(|r| wanted.contains(&r.key)),
        Job::Offscreen(job) => queues.offscreen.push_back(job),
    }
}

/// Liest alle Zeichen einer Seite mit ihren Rechtecken (PDF-Raum, unrotiert).
fn extract_text(doc: &PdfDocument, page: u32) -> Result<PageText> {
    let page = doc.pages().get(page as PdfPageIndex)?;
    let text = page.text()?;
    let chars = text
        .chars()
        .iter()
        .map(|c| {
            let ch = c.unicode_char().unwrap_or('\u{FFFD}');
            // Generierte Zeichen (von PDFium eingefügte Leerzeichen/Umbrüche) haben keine Box.
            let rect = c
                .loose_bounds()
                .ok()
                .filter(|_| !c.is_generated().unwrap_or(false))
                .map(|b| {
                    Rect::new(
                        b.left().value,
                        b.bottom().value,
                        b.right().value,
                        b.top().value,
                    )
                    .normalized()
                })
                .unwrap_or_default();
            CharBox { ch, rect }
        })
        .collect();
    Ok(PageText::new(chars))
}

/// Offscreen-Auftrag ausführen und direkt beantworten.
fn run_offscreen<'p>(
    pdfium: &'p Pdfium,
    cache: &mut Option<(Arc<Vec<u8>>, PdfDocument<'p>)>,
    job: OffscreenJob,
) {
    let pdf = offscreen_doc(pdfium, cache, &job.bytes);
    match job.kind {
        OffscreenKind::Render {
            page_size_pt,
            scale,
            reply,
        } => {
            let result = pdf.and_then(|pdf| render_offscreen(pdf, job.page, page_size_pt, scale));
            let _ = reply.send(result);
        }
        OffscreenKind::Chars { reply } => {
            let _ = reply.send(pdf.and_then(|pdf| extract_text(pdf, job.page)));
        }
        OffscreenKind::Text { reply } => {
            let result = pdf.and_then(|pdf| {
                let page = pdf.pages().get(job.page as PdfPageIndex)?;
                Ok(page.text()?.all())
            });
            let _ = reply.send(result);
        }
    }
}

/// Lädt das Dokument eines Offscreen-Auftrags (oder nimmt das zuletzt geladene).
fn offscreen_doc<'a, 'p>(
    pdfium: &'p Pdfium,
    cache: &'a mut Option<(Arc<Vec<u8>>, PdfDocument<'p>)>,
    bytes: &Arc<Vec<u8>>,
) -> Result<&'a PdfDocument<'p>> {
    let cached = cache.as_ref().is_some_and(|(b, _)| Arc::ptr_eq(b, bytes));
    if !cached {
        *cache = None;
        let pdf = pdfium.load_pdf_from_byte_vec(bytes.as_ref().clone(), None)?;
        *cache = Some((Arc::clone(bytes), pdf));
    }
    cache
        .as_ref()
        .map(|(_, pdf)| pdf)
        .ok_or(RenderError::WorkerGone)
}

/// Ganze Seite rendern (ohne Annotationen).
fn render_offscreen(
    pdf: &PdfDocument,
    page: u32,
    page_size_pt: (f32, f32),
    scale: f32,
) -> Result<RgbaImage> {
    let page = pdf.pages().get(page as PdfPageIndex)?;
    let (w, h) = page_pixel_size(page_size_pt, scale);
    let mut bitmap = PdfBitmap::empty(w as Pixels, h as Pixels, PdfBitmapFormat::BGRA)?;
    let config = PdfRenderConfig::new()
        .set_fixed_size(w as Pixels, h as Pixels)
        // Achtung: Ohne Formulardaten nimmt pdfium-render den Weg über eine Matrix und
        // ignoriert dann `set_fixed_size` (die Seite käme im Maßstab 1). Formularfelder
        // stören die Texterkennung nicht – ihr Text soll ruhig mit erkannt werden.
        .render_form_data(true)
        .render_annotations(false)
        .set_clear_color(PdfColor::WHITE)
        .set_reverse_byte_order(true);
    page.render_into_bitmap_with_config(&mut bitmap, &config)?;
    Ok(RgbaImage {
        width: w,
        height: h,
        pixels: bitmap.as_rgba_bytes(),
    })
}

fn render(doc: &PdfDocument, req: &RenderRequest) -> Result<RgbaImage> {
    let page = doc.pages().get(req.key.page as PdfPageIndex)?;
    let full = page_pixel_size(req.page_size_pt, req.key.scale());
    let (x, y, w, h) = match req.key.tile {
        None => (0, 0, full.0, full.1),
        Some(t) => tile_rect(full, t),
    };

    let mut bitmap = PdfBitmap::empty(w as Pixels, h as Pixels, PdfBitmapFormat::BGRA)?;
    let config = PdfRenderConfig::new()
        .set_fixed_size(full.0 as Pixels, full.1 as Pixels)
        // Negative Verschiebung: Die Seite wird so platziert, dass nur der Kachelausschnitt
        // in die (kleinere) Bitmap fällt. PDFium schneidet den Rest ab.
        .set_origin(-(x as Pixels), -(y as Pixels))
        .render_form_data(true)
        .render_annotations(true)
        .set_clear_color(PdfColor::WHITE)
        // Lässt PDFium direkt RGBA statt BGRA schreiben -> keine Umrechnung nötig.
        .set_reverse_byte_order(true);
    page.render_into_bitmap_with_config(&mut bitmap, &config)?;

    Ok(RgbaImage {
        width: w,
        height: h,
        pixels: bitmap.as_rgba_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_math() {
        let px = page_pixel_size((595.0, 842.0), 5.0); // A4 bei 500 %
        assert_eq!(px, (2975, 4210));
        assert!(needs_tiles(px));
        assert_eq!(tile_grid(px), (3, 5));
        assert_eq!(tile_rect(px, (2, 4)), (2048, 4096, 927, 114));
    }
}
