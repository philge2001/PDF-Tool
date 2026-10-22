//! Der Render-Worker: ein eigener Thread, der PDFium exklusiv besitzt.
//!
//! Die UI schickt [`Job`]s über einen Channel und bekommt [`Event`]s zurück.
//! Nach jedem Ereignis ruft der Worker den `waker` auf (in der App: `ctx.request_repaint()`),
//! damit die UI das Ergebnis sofort abholt.

use crate::error::{RenderError, Result};
use crate::loader;
use pdfium_render::prelude::*;
use std::collections::HashSet;
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
    Opened { doc: DocId, page_count: usize },
    OpenFailed { doc: DocId, error: String },
    Rendered { key: RenderKey, image: RgbaImage },
    RenderFailed { key: RenderKey, error: String },
}

/// Nachrichten von der UI an den Worker (intern).
enum Job {
    Open {
        doc: DocId,
        bytes: Arc<Vec<u8>>,
    },
    Close,
    Render(RenderRequest),
    /// Verwirft alle wartenden Aufträge, deren Schlüssel nicht in der Menge ist.
    Retain(HashSet<RenderKey>),
}

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
            .name("foliocraft-render".into())
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
    pub fn open(&self, doc: DocId, bytes: Arc<Vec<u8>>) {
        self.send(Job::Open { doc, bytes });
    }

    pub fn close(&self) {
        self.send(Job::Close);
    }

    pub fn request(&self, req: RenderRequest) {
        self.send(Job::Render(req));
    }

    /// Teilt dem Worker mit, welche Aufträge noch gebraucht werden.
    pub fn retain(&self, wanted: HashSet<RenderKey>) {
        self.send(Job::Retain(wanted));
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
    let mut doc: Option<(DocId, PdfDocument<'p>)> = None;
    let mut queue: Vec<RenderRequest> = Vec::new();

    loop {
        // Nichts zu tun? Dann blockierend auf den nächsten Auftrag warten (kostet keine CPU).
        if queue.is_empty() {
            match jobs.recv() {
                Ok(job) => handle_job(job, pdfium, &mut doc, &mut queue, &events, &waker),
                Err(_) => return, // App wurde beendet
            }
        }
        // Alle weiteren wartenden Aufträge einsammeln, damit wir priorisieren können.
        while let Ok(job) = jobs.try_recv() {
            handle_job(job, pdfium, &mut doc, &mut queue, &events, &waker);
        }

        // Dringendsten Auftrag wählen; bei Gleichstand den zuletzt angefragten
        // (der Nutzer schaut vermutlich gerade dorthin).
        let Some(best) = queue
            .iter()
            .enumerate()
            .min_by_key(|(i, r)| (r.priority, std::cmp::Reverse(*i)))
            .map(|(i, _)| i)
        else {
            continue;
        };
        let req = queue.remove(best);

        let event = match &doc {
            Some((id, d)) if *id == req.key.doc => match render(d, &req) {
                Ok(image) => Event::Rendered {
                    key: req.key,
                    image,
                },
                Err(e) => Event::RenderFailed {
                    key: req.key,
                    error: e.to_string(),
                },
            },
            _ => continue, // gehört zu einem alten Dokument
        };
        if events.send(event).is_err() {
            return;
        }
        waker();
    }
}

fn handle_job<'p>(
    job: Job,
    pdfium: &'p Pdfium,
    doc: &mut Option<(DocId, PdfDocument<'p>)>,
    queue: &mut Vec<RenderRequest>,
    events: &Sender<Event>,
    waker: &impl Fn(),
) {
    match job {
        Job::Open { doc: id, bytes } => {
            queue.clear();
            *doc = None; // altes Dokument zuerst freigeben
            // PDFium will einen eigenen Vec; wir kopieren die Bytes einmalig.
            let event = match pdfium.load_pdf_from_byte_vec(bytes.as_ref().clone(), None) {
                Ok(d) => {
                    let page_count = d.pages().len() as usize;
                    *doc = Some((id, d));
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
            queue.clear();
            *doc = None;
        }
        Job::Render(req) => {
            // Doppelte Aufträge zusammenführen, dabei die höhere Priorität behalten.
            if let Some(existing) = queue.iter_mut().find(|r| r.key == req.key) {
                existing.priority = existing.priority.min(req.priority);
            } else {
                queue.push(req);
            }
        }
        Job::Retain(wanted) => queue.retain(|r| wanted.contains(&r.key)),
    }
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
