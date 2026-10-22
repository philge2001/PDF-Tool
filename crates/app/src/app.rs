//! Die Haupt-App: hält Render-Service, geöffnetes Dokument und UI-Zustand.

use crate::viewer::Viewer;
use crate::viewer::thumbnails::Thumbnails;
use eframe::egui;
use foliocraft_core::Document;
use foliocraft_render::{DocId, Event, RenderService};
use std::path::{Path, PathBuf};

/// Ein geöffnetes Dokument samt Ansicht.
pub struct OpenDocument {
    pub doc: Document,
    pub viewer: Viewer,
    pub thumbs: Thumbnails,
    /// Inhalt des Seitenfelds in der Toolbar, während der Nutzer tippt.
    pub page_input: Option<String>,
    /// Inhalt des Zoomfelds in der Toolbar, während der Nutzer tippt.
    pub zoom_input: Option<String>,
}

pub struct FolioApp {
    /// `Err` enthält die Fehlermeldung, wenn PDFium nicht geladen werden konnte.
    pub(crate) render: Result<RenderService, String>,
    next_doc_id: DocId,
    pub(crate) document: Option<OpenDocument>,
    /// Fehlermeldung, die als Dialog angezeigt wird.
    pub(crate) error: Option<String>,
    pub(crate) show_thumbnails: bool,
}

impl FolioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        // egui würde Strg+Plus/Minus sonst für die UI-Vergrößerung nutzen – wir zoomen das Dokument.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        // Der Worker weckt die UI auf, wenn ein Bild fertig ist. `Context` ist intern ein
        // `Arc` – das Klonen ist billig und der Klon darf in den Worker-Thread wandern.
        let ctx = cc.egui_ctx.clone();
        let render = RenderService::start(move || ctx.request_repaint()).map_err(|e| e.to_string());
        if let Err(e) = &render {
            tracing::error!("{e}");
        }
        let mut app = Self {
            render,
            next_doc_id: 1,
            document: None,
            error: None,
            show_thumbnails: true,
        };
        if let Some(path) = initial_file {
            app.open_path(&cc.egui_ctx, &path);
        }
        app
    }

    pub fn open_path(&mut self, ctx: &egui::Context, path: &Path) {
        let render = match &self.render {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e.clone());
                return;
            }
        };
        match Document::open(path) {
            Ok(doc) => {
                let id = self.next_doc_id;
                self.next_doc_id += 1;
                render.open(id, doc.bytes().clone()); // Arc-Klon: nur der Zeiger wird kopiert
                let sizes: Vec<_> = doc.pages().iter().map(|p| p.display_size()).collect();
                self.document = Some(OpenDocument {
                    thumbs: Thumbnails::new(&sizes),
                    viewer: Viewer::new(id, sizes),
                    doc,
                    page_input: None,
                    zoom_input: None,
                });
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{name} – FolioCraft")));
                tracing::info!(path = %path.display(), "Dokument geöffnet");
            }
            Err(e) => {
                self.error = Some(format!(
                    "„{}“ konnte nicht geöffnet werden:\n{e}",
                    path.display()
                ))
            }
        }
    }

    pub(crate) fn open_dialog(&mut self, ctx: &egui::Context) {
        // Nativer Dialog; blockiert den UI-Thread, solange er offen ist (normal für Dateidialoge).
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .pick_file()
        {
            self.open_path(ctx, &path);
        }
    }

    /// Holt fertige Render-Ergebnisse ab.
    fn poll_render(&mut self, ctx: &egui::Context) {
        let Ok(render) = &self.render else { return };
        for event in render.poll() {
            match (&mut self.document, event) {
                (Some(d), Event::OpenFailed { doc, error }) if doc == d.viewer.doc_id => {
                    self.error = Some(format!("PDFium kann das Dokument nicht öffnen: {error}"));
                    self.document = None;
                }
                (Some(d), ev) => d.viewer.on_render_event(ctx, ev),
                (None, _) => {}
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        // `ctx.input` leiht den Eingabezustand kurz aus; wir kopieren den Pfad heraus.
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| !p.as_os_str().is_empty())
        });
        if let Some(path) = dropped {
            self.open_path(ctx, &path);
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| match &self.document {
            Some(d) => {
                let name = d
                    .doc
                    .path()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ui.label(name);
                ui.separator();
                ui.label(format!(
                    "Seite {} / {}",
                    d.viewer.current_page + 1,
                    d.doc.page_count()
                ));
            }
            None => {
                ui.label("Kein Dokument");
            }
        });
    }

    fn empty_state(&self, ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| match &self.render {
            Ok(_) => ui.heading("PDF hierher ziehen oder mit Strg+O öffnen"),
            Err(e) => ui.label(egui::RichText::new(e).color(ui.visuals().error_fg_color)),
        });
    }

    fn error_dialog(&mut self, ctx: &egui::Context) {
        let Some(msg) = &self.error else { return };
        let mut close = false;
        egui::Window::new("Fehler")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(msg);
                close = ui.button("OK").clicked();
            });
        if close {
            self.error = None;
        }
    }
}

impl eframe::App for FolioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_render(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("statusbar").show(ui, |ui| self.status_bar(ui));

        if let Some(d) = &mut self.document {
            if self.show_thumbnails {
                egui::Panel::left("thumbnails")
                    .default_size(160.0)
                    .size_range(100.0..=400.0)
                    .show(ui, |ui| {
                        if let Some(page) = d.viewer.show_thumbnails(ui, &mut d.thumbs) {
                            d.viewer.scroll_to_page(page);
                        }
                    });
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(ui.visuals().extreme_bg_color))
                .show(ui, |ui| {
                    d.viewer.show(ui);
                });
            if let Ok(render) = &self.render {
                d.viewer.flush_requests(render);
            }
        } else {
            egui::CentralPanel::default().show(ui, |ui| self.empty_state(ui));
        }
        self.error_dialog(&ctx);
    }
}
