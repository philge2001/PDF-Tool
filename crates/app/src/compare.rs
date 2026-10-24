//! Dokumentvergleich: aktuelles Dokument (alt) gegen eine andere Datei (neu).
//!
//! Ein Job holt von beiden Dokumenten die Zeichen mit Rechtecken (PDFium, offscreen) und
//! vergleicht die Wortfolgen (`core::compare`). Das Fenster zeigt die Änderungen als Liste
//! und die betroffenen Seiten nebeneinander: Gelöschtes links rot, Eingefügtes rechts grün.
//! Die Seitenbilder rendert bei Bedarf ein Hilfsthread (ebenfalls offscreen).

use crate::app::PdfToolApp;
use crate::viewer::transform;
use eframe::egui::{self, Color32, Rect, Stroke, StrokeKind, vec2};
use pdftool_core::compare::{self, Change, ChangeKind};
use pdftool_core::{Document, PageInfo};
use pdftool_render::{OffscreenRenderer, RgbaImage};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Auflösung der Vorschaubilder (Pixel pro Punkt).
const PREVIEW_SCALE: f32 = 1.5;

const DELETED: Color32 = Color32::from_rgb(220, 50, 50);
const INSERTED: Color32 = Color32::from_rgb(30, 160, 70);

/// Ein Dokument im Vergleich.
pub struct Side {
    bytes: Arc<Vec<u8>>,
    pages: Vec<PageInfo>,
    name: String,
}

/// Ergebnis des Vergleichs-Jobs.
pub struct CompareResult {
    old: Side,
    new: Side,
    changes: Vec<Change>,
}

pub type CompareSlot = Arc<Mutex<Option<CompareResult>>>;

/// Welche Seite welchen Dokuments (false = alt, true = neu).
type PageKey = (bool, usize);

pub struct CompareView {
    result: CompareResult,
    renderer: OffscreenRenderer,
    selected: Option<usize>,
    /// Angezeigte Seiten (alt, neu).
    pages: (usize, usize),
    textures: HashMap<PageKey, egui::TextureHandle>,
    requested: HashSet<PageKey>,
    tx: Sender<(PageKey, Result<RgbaImage, String>)>,
    rx: Receiver<(PageKey, Result<RgbaImage, String>)>,
}

/// Wörter aller Seiten eines Dokuments.
fn document_words(
    renderer: &OffscreenRenderer,
    bytes: &Arc<Vec<u8>>,
    pages: usize,
    progress: &dyn Fn(usize) -> bool,
) -> Result<Vec<compare::Word>, String> {
    let mut words = Vec::new();
    for page in 0..pages {
        if !progress(page) {
            return Err("Vergleich abgebrochen.".into());
        }
        let text = renderer
            .page_chars(bytes, page as u32)
            .map_err(|e| format!("Seite {}: {e}", page + 1))?;
        words.extend(compare::words(page, &text));
    }
    Ok(words)
}

impl PdfToolApp {
    /// "Vergleichen mit…": andere Datei wählen und den Vergleich starten.
    /// Das Ergebnis kommt über `handle_picked` (`Pick::Compare`) → `start_compare_with`.
    pub(crate) fn start_compare(&mut self, ctx: &egui::Context) {
        crate::platform::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_title("Vergleichen mit…")
            .pick(ctx, crate::platform::Pick::Compare, false);
    }

    pub(crate) fn start_compare_with(&mut self, ctx: &egui::Context, path: PathBuf) {
        let Some(d) = &self.document else { return };
        if self.job.is_some() {
            return;
        }
        let renderer = match &self.render {
            Ok(r) => r.offscreen(),
            Err(e) => {
                self.error = Some(e.clone());
                return;
            }
        };
        let Some(renderer) = renderer else { return };
        let old = match d.doc.render_snapshot() {
            Ok(s) => Side {
                bytes: Arc::new(s.bytes),
                pages: d.doc.pages().to_vec(),
                name: d.doc.path().and_then(|p| p.file_name()).map_or_else(
                    || "Dieses Dokument".into(),
                    |n| n.to_string_lossy().into_owned(),
                ),
            },
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let slot: CompareSlot = Arc::new(Mutex::new(None));
        let out = slot.clone();
        let worker = renderer.clone();
        let job = crate::jobs::Job::spawn("Vergleichen", ctx, move |c| {
            c.progress(0.0, "Datei wird gelesen …");
            let bytes =
                crate::platform::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let doc = Document::from_bytes(bytes.clone()).map_err(|e| e.to_string())?;
            let new = Side {
                bytes: Arc::new(bytes),
                pages: doc.pages().to_vec(),
                name: path.file_name().map_or_else(
                    || "Andere Datei".into(),
                    |n| n.to_string_lossy().into_owned(),
                ),
            };
            let total = (old.pages.len() + new.pages.len()).max(1) as f32;
            let old_words = document_words(&worker, &old.bytes, old.pages.len(), &|p| {
                c.progress(
                    p as f32 / total,
                    format!("Text lesen: {} Seite {}", old.name, p + 1),
                );
                !c.cancelled()
            })?;
            let offset = old.pages.len();
            let new_words = document_words(&worker, &new.bytes, new.pages.len(), &|p| {
                c.progress(
                    (offset + p) as f32 / total,
                    format!("Text lesen: {} Seite {}", new.name, p + 1),
                );
                !c.cancelled()
            })?;
            c.progress(1.0, "Vergleichen …");
            let changes = compare::compare(&old_words, &new_words);
            let summary = if changes.is_empty() {
                "Keine Textunterschiede gefunden.".to_string()
            } else {
                format!("{} Unterschied(e) gefunden.", changes.len())
            };
            if let Ok(mut s) = out.lock() {
                *s = Some(CompareResult { old, new, changes });
            }
            Ok(summary)
        });
        self.pending_compare = Some((slot, renderer));
        self.job = Some(job);
    }

    /// Nach dem Job: Vergleichsfenster öffnen.
    pub(crate) fn take_pending_compare(&mut self) {
        let Some((slot, renderer)) = self.pending_compare.take() else {
            return;
        };
        let Some(result) = slot.lock().ok().and_then(|mut s| s.take()) else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut view = CompareView {
            result,
            renderer,
            selected: None,
            pages: (0, 0),
            textures: HashMap::new(),
            requested: HashSet::new(),
            tx,
            rx,
        };
        if !view.result.changes.is_empty() {
            view.select(0);
        }
        self.compare = Some(view);
    }

    pub(crate) fn compare_ui(&mut self, ctx: &egui::Context) {
        let Some(view) = &mut self.compare else {
            return;
        };
        let mut open = true;
        egui::Window::new("Vergleich")
            .open(&mut open)
            .default_size(vec2(1100.0, 720.0))
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| view.show(ui));
        if !open {
            self.compare = None;
        }
    }
}

impl CompareView {
    fn select(&mut self, i: usize) {
        if let Some(c) = self.result.changes.get(i) {
            self.selected = Some(i);
            self.pages = (c.old_page, c.new_page);
        }
    }

    /// Seitenbild holen (oder im Hintergrund anfordern).
    fn texture(&mut self, ctx: &egui::Context, key: PageKey) -> Option<egui::TextureHandle> {
        // Fertige Bilder abholen.
        while let Ok((k, image)) = self.rx.try_recv() {
            match image {
                Ok(img) => {
                    let color = egui::ColorImage::from_rgba_unmultiplied(
                        [img.width as usize, img.height as usize],
                        &img.pixels,
                    );
                    let name = format!("compare-{}-{}", k.0, k.1);
                    self.textures.insert(
                        k,
                        ctx.load_texture(name, color, egui::TextureOptions::LINEAR),
                    );
                }
                Err(e) => tracing::warn!(error = %e, "Vergleichsseite nicht gerendert"),
            }
        }
        if let Some(t) = self.textures.get(&key) {
            return Some(t.clone());
        }
        if self.requested.insert(key) {
            let side = if key.0 {
                &self.result.new
            } else {
                &self.result.old
            };
            let size = side.pages.get(key.1)?.display_size();
            let (bytes, renderer, tx, ctx) = (
                side.bytes.clone(),
                self.renderer.clone(),
                self.tx.clone(),
                ctx.clone(),
            );
            // Eigener Thread: `render_page` blockiert, die UI soll weiterlaufen.
            crate::platform::spawn(move || {
                let img = renderer
                    .render_page(&bytes, key.1 as u32, size, PREVIEW_SCALE)
                    .map_err(|e| e.to_string());
                let _ = tx.send((key, img));
                ctx.request_repaint();
            });
        }
        None
    }

    fn show(&mut self, ui: &mut egui::Ui) {
        let n = self.result.changes.len();
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} (alt) gegen {} (neu): {n} Unterschied(e)",
                self.result.old.name, self.result.new.name
            ));
            ui.separator();
            if ui
                .add_enabled(n > 0, egui::Button::new("‹ Vorheriger"))
                .clicked()
            {
                let i = self.selected.map_or(0, |i| (i + n - 1) % n);
                self.select(i);
            }
            if ui
                .add_enabled(n > 0, egui::Button::new("Nächster ›"))
                .clicked()
            {
                let i = self.selected.map_or(0, |i| (i + 1) % n);
                self.select(i);
            }
        });
        ui.separator();

        let mut clicked = None;
        egui::Panel::left("compare_list")
            .default_size(260.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if n == 0 {
                        ui.label("Keine Textunterschiede.");
                    }
                    for (i, c) in self.result.changes.iter().enumerate() {
                        let selected = self.selected == Some(i);
                        let text = match c.kind {
                            ChangeKind::Deleted => {
                                format!("S. {}: − {}", c.old_page + 1, short(&c.old_text()))
                            }
                            ChangeKind::Inserted => {
                                format!("S. {}: + {}", c.new_page + 1, short(&c.new_text()))
                            }
                            ChangeKind::Replaced => format!(
                                "S. {}: {} statt {}",
                                c.new_page + 1,
                                short(&c.new_text()),
                                short(&c.old_text())
                            ),
                        };
                        let color = match c.kind {
                            ChangeKind::Deleted => DELETED,
                            ChangeKind::Inserted => INSERTED,
                            ChangeKind::Replaced => ui.visuals().text_color(),
                        };
                        if ui
                            .selectable_label(selected, egui::RichText::new(text).color(color))
                            .clicked()
                        {
                            clicked = Some(i);
                        }
                    }
                });
            });
        if let Some(i) = clicked {
            self.select(i);
        }

        // Beide Seiten nebeneinander, so groß wie Platz ist.
        let avail = ui.available_rect_before_wrap();
        let half = (avail.width() - 12.0) / 2.0;
        for (side_new, x) in [(false, avail.min.x), (true, avail.min.x + half + 12.0)] {
            let page = if side_new { self.pages.1 } else { self.pages.0 };
            let side = if side_new {
                &self.result.new
            } else {
                &self.result.old
            };
            let Some(info) = side.pages.get(page).cloned() else {
                continue;
            };
            let (w, h) = info.display_size();
            let scale = (half / w).min((avail.height() - 24.0) / h);
            let rect = Rect::from_min_size(
                egui::pos2(x, avail.min.y + 20.0),
                vec2(w * scale, h * scale),
            );
            ui.painter().text(
                egui::pos2(x, avail.min.y),
                egui::Align2::LEFT_TOP,
                format!(
                    "{} – Seite {}",
                    if side_new { "neu" } else { "alt" },
                    page + 1
                ),
                egui::FontId::proportional(13.0),
                ui.visuals().text_color(),
            );
            match self.texture(ui.ctx(), (side_new, page)) {
                Some(t) => {
                    ui.painter().image(
                        t.id(),
                        rect,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().rect_filled(rect, 0.0, Color32::from_gray(235));
                }
            }
            // Änderungen auf dieser Seite markieren (ausgewählte kräftiger).
            for (i, c) in self.result.changes.iter().enumerate() {
                let (words, color) = if side_new {
                    (&c.new, INSERTED)
                } else {
                    (&c.old, DELETED)
                };
                let strong = self.selected == Some(i);
                for wd in words.iter().filter(|wd| wd.page == page) {
                    let r = transform::pdf_rect_to_screen(&info, rect, wd.rect).expand(1.0);
                    ui.painter().rect_filled(
                        r,
                        1.0,
                        color.gamma_multiply(if strong { 0.45 } else { 0.2 }),
                    );
                    if strong {
                        ui.painter().rect_stroke(
                            r,
                            1.0,
                            Stroke::new(1.5, color),
                            StrokeKind::Outside,
                        );
                    }
                }
            }
        }
        ui.allocate_rect(avail, egui::Sense::hover());
    }
}

/// Kürzt lange Texte für die Liste.
fn short(s: &str) -> String {
    const MAX: usize = 40;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(MAX).collect::<String>())
    }
}
