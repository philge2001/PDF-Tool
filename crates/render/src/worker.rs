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
    // Im Browser laufen Offscreen-Aufträge direkt, ohne Warteschlange.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct OffscreenRenderer {
    jobs: Sender<Job>,
}

#[cfg(not(target_arch = "wasm32"))]
impl OffscreenRenderer {
    fn send(&self, job: OffscreenJob) -> Result<()> {
        self.jobs
            .send(Job::Offscreen(job))
            .map_err(|_| RenderError::WorkerGone)
    }
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
        self.submit(
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
        self.submit(bytes, page, OffscreenKind::Text { reply })?;
        answer.recv().map_err(|_| RenderError::WorkerGone)?
    }

    /// Zeichen einer Seite mit Rechtecken (PDF-Raum). Blockiert.
    pub fn page_chars(&self, bytes: &Arc<Vec<u8>>, page: u32) -> Result<PageText> {
        let (reply, answer) = mpsc::channel();
        self.submit(bytes, page, OffscreenKind::Chars { reply })?;
        answer.recv().map_err(|_| RenderError::WorkerGone)?
    }

    fn submit(&self, bytes: &Arc<Vec<u8>>, page: u32, kind: OffscreenKind) -> Result<()> {
        self.send(OffscreenJob {
            bytes: Arc::clone(bytes),
            page,
            kind,
        })
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
#[cfg(not(target_arch = "wasm32"))]
pub struct RenderService {
    // `Option`, damit wir den Sender in `Drop` vorher wegnehmen können (siehe unten).
    jobs: Option<Sender<Job>>,
    events: Receiver<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
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
                    run(Worker::new(&pdfium, ev_tx, Box::new(waker)), job_rx);
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

/// Gemeinsame Aufträge beider Varianten (Desktop: Thread, Browser: siehe [`web`]).
impl RenderService {
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
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for RenderService {
    fn drop(&mut self) {
        // Sender droppen -> `recv()` im Worker liefert Err -> Schleife endet.
        self.jobs.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Hauptschleife des Render-Threads (Desktop).
#[cfg(not(target_arch = "wasm32"))]
fn run(mut worker: Worker<'_>, jobs: Receiver<Job>) {
    loop {
        // Nichts zu tun? Dann blockierend auf den nächsten Auftrag warten (kostet keine CPU).
        if !worker.has_work() {
            match jobs.recv() {
                Ok(job) => worker.handle_job(job),
                Err(_) => return, // App wurde beendet
            }
        }
        // Alle weiteren wartenden Aufträge einsammeln, damit wir priorisieren können.
        while let Ok(job) = jobs.try_recv() {
            worker.handle_job(job);
        }
        if !worker.step() {
            return; // die App holt keine Ergebnisse mehr ab
        }
    }
}

/// Browser-Variante: Es gibt keinen zweiten Thread. PDFium läuft im UI-Thread, und zwar
/// in Zeitscheiben: Aufträge werden nur gesammelt, [`RenderService::poll`] arbeitet sie pro
/// Frame höchstens [`web::FRAME_BUDGET_MS`] lang ab und fordert bei Restarbeit einen neuen
/// Frame an. Offscreen-Aufträge (Hintergrund-Jobs) laufen sofort und synchron.
#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use std::cell::RefCell;

    /// Höchstens so viele Millisekunden PDFium-Arbeit pro Frame (die Oberfläche bleibt flüssig).
    pub(super) const FRAME_BUDGET_MS: u128 = 12;

    thread_local! {
        // Im Browser gibt es nur einen Thread; der Worker lebt hier, damit der klonbare
        // `OffscreenRenderer` (der `Send` sein muss) ohne eigenen Zeiger an ihn herankommt.
        static WORKER: RefCell<Option<Worker<'static>>> = const { RefCell::new(None) };
    }

    pub struct RenderService {
        events: Receiver<Event>,
    }

    impl RenderService {
        /// Bindet pdfium.wasm (vorher von der Webseite geladen, siehe PDF Webeditor).
        pub fn start(waker: impl Fn() + Send + 'static) -> Result<Self> {
            let pdfium = loader::bind_pdfium()?;
            // PDFium lebt bis zum Schließen der Seite: `Box::leak` liefert `&'static Pdfium`,
            // damit der Worker (und seine `PdfDocument<'static>`) ohne Lifetime auskommt.
            let pdfium: &'static Pdfium = Box::leak(Box::new(pdfium));
            let (ev_tx, ev_rx) = mpsc::channel::<Event>();
            WORKER.with(|w| *w.borrow_mut() = Some(Worker::new(pdfium, ev_tx, Box::new(waker))));
            Ok(Self { events: ev_rx })
        }

        pub(super) fn send(&self, job: Job) {
            with_worker(|w| {
                w.handle_job(job);
                (w.waker)(); // nächster Frame arbeitet die Aufträge in `poll` ab
            });
        }

        pub fn offscreen(&self) -> Option<OffscreenRenderer> {
            Some(OffscreenRenderer {})
        }

        /// Arbeitet Aufträge ab (höchstens eine Zeitscheibe) und liefert die Ergebnisse.
        pub fn poll(&self) -> Vec<Event> {
            let start = web_time::Instant::now();
            with_worker(|w| {
                while w.has_work() && start.elapsed().as_millis() < FRAME_BUDGET_MS {
                    w.step();
                }
                if w.has_work() {
                    (w.waker)();
                }
            });
            self.events.try_iter().collect()
        }
    }

    pub(super) fn with_worker(f: impl FnOnce(&mut Worker<'static>)) {
        WORKER.with(|w| match w.try_borrow_mut() {
            Ok(mut guard) => {
                if let Some(worker) = guard.as_mut() {
                    f(worker);
                }
            }
            Err(_) => {
                tracing::error!("Render-Worker ist gerade beschäftigt (verschachtelter Aufruf)")
            }
        });
    }

    /// Im Browser rendern Hintergrund-Jobs direkt (synchron) im UI-Thread.
    #[derive(Clone)]
    pub struct OffscreenRenderer {}

    impl OffscreenRenderer {
        pub(super) fn send(&self, job: OffscreenJob) -> Result<()> {
            let mut job = Some(job);
            with_worker(|w| {
                if let Some(job) = job.take() {
                    w.run_offscreen(job);
                }
            });
            // Nicht ausgeführt (kein Worker) → der Aufrufer bekommt beim Warten `WorkerGone`.
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{OffscreenRenderer, RenderService};

/// Der Zustand des Workers: geöffnetes Dokument, Warteschlangen, Offscreen-Dokument.
///
/// Die Lifetime `'p` verknüpft die Dokumente mit `pdfium`: Ein `PdfDocument<'p>` darf
/// nicht länger leben als die `Pdfium`-Instanz, von der es stammt. Der Compiler prüft das.
struct Worker<'p> {
    pdfium: &'p Pdfium,
    doc: Option<OpenDoc<'p>>,
    queues: Queues,
    /// Zuletzt für Offscreen-Aufträge geladenes Dokument (erkannt am Arc-Zeiger).
    offscreen_doc: Option<(Arc<Vec<u8>>, PdfDocument<'p>)>,
    events: Sender<Event>,
    /// In der App: `ctx.request_repaint()`. `Box<dyn …>`, damit `Worker` keinen Typparameter braucht.
    waker: Box<dyn Fn()>,
}

impl<'p> Worker<'p> {
    fn new(pdfium: &'p Pdfium, events: Sender<Event>, waker: Box<dyn Fn()>) -> Self {
        Self {
            pdfium,
            doc: None,
            queues: Queues::default(),
            offscreen_doc: None,
            events,
            waker,
        }
    }

    fn has_work(&self) -> bool {
        !self.queues.is_empty()
    }

    /// Erledigt eine Arbeitseinheit. `false`, wenn niemand mehr Ergebnisse abholt.
    fn step(&mut self) -> bool {
        // Offscreen-Aufträge brauchen kein geöffnetes Dokument der Ansicht.
        if self.queues.offscreen_is_next()
            && let Some(job) = self.queues.offscreen.pop_front()
        {
            self.run_offscreen(job);
            return true;
        }
        if self.queues.offscreen.is_empty() {
            self.offscreen_doc = None; // Speicher freigeben, wenn kein Job mehr rendert
        }
        let Some(d) = &mut self.doc else {
            // ohne Dokument gibt es nichts zu tun (Offscreen-Aufträge laufen oben weiter)
            self.queues.renders.clear();
            self.queues.texts.clear();
            return true;
        };
        let doc_id = d.id;

        let event = match self.queues.next() {
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
            _ => return true, // leer oder gehört zu einem alten Dokument
        };
        if self.events.send(event).is_err() {
            return false;
        }
        (self.waker)();
        true
    }

    fn handle_job(&mut self, job: Job) {
        let queues = &mut self.queues;
        match job {
            Job::Open {
                doc: id,
                bytes,
                hidden,
            } => {
                queues.clear();
                self.doc = None; // altes Dokument zuerst freigeben
                // PDFium will einen eigenen Vec; wir kopieren die Bytes einmalig.
                let event = match self
                    .pdfium
                    .load_pdf_from_byte_vec(bytes.as_ref().clone(), None)
                {
                    Ok(d) => {
                        let page_count = d.pages().len() as usize;
                        self.doc = Some(OpenDoc { id, pdf: d, hidden });
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
                let _ = self.events.send(event);
                (self.waker)();
            }
            Job::Close => {
                queues.clear();
                self.doc = None;
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

    /// Offscreen-Auftrag ausführen und direkt beantworten.
    fn run_offscreen(&mut self, job: OffscreenJob) {
        let pdf = offscreen_doc(self.pdfium, &mut self.offscreen_doc, &job.bytes);
        match job.kind {
            OffscreenKind::Render {
                page_size_pt,
                scale,
                reply,
            } => {
                let result =
                    pdf.and_then(|pdf| render_offscreen(pdf, job.page, page_size_pt, scale));
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
