//! Die Haupt-App: hält Render-Service, geöffnetes Dokument und UI-Zustand.

use crate::annot_draw;
use crate::file_ops::PendingAction;
use crate::search::SearchState;
use crate::settings::{STORAGE_KEY, Settings};
use crate::text_cache::TextCache;
use crate::tools::create::Gesture;
use crate::tools::editor::TextEditor;
use crate::tools::select::AnnotDrag;
use crate::tools::text_select::TextSelection;
use crate::tools::{Tool, ToolSettings};
use crate::viewer::ViewOutput;
use crate::viewer::Viewer;
use crate::viewer::thumbnails::ThumbAction;
use crate::viewer::thumbnails::Thumbnails;
use eframe::egui;
use foliocraft_core::{AnnotId, Document};
use foliocraft_render::{Event, HiddenAnnots, RenderService};
use std::path::{Path, PathBuf};

/// Ein geöffnetes Dokument samt Ansicht.
pub struct OpenDocument {
    pub doc: Document,
    pub viewer: Viewer,
    pub thumbs: Thumbnails,
    pub texts: TextCache,
    /// Letzter bekannter Stand der Seitenstruktur (siehe `sync_structure`).
    pub structure: Vec<(foliocraft_core::PageId, u16)>,
    pub search: SearchState,
    pub selection: Option<TextSelection>,
    /// Laufender Mauszug eines Zeichenwerkzeugs.
    pub gesture: Option<Gesture>,
    /// Offenes Eingabefenster für Notiz-/Freitext.
    pub editor: Option<TextEditor>,
    /// Ausgewählte Annotation.
    pub selected: Option<AnnotId>,
    /// Laufendes Verschieben einer Annotation.
    pub annot_drag: Option<AnnotDrag>,
    /// Inhalt des Seitenfelds in der Toolbar, während der Nutzer tippt.
    pub page_input: Option<String>,
    /// Inhalt des Zoomfelds in der Toolbar, während der Nutzer tippt.
    pub zoom_input: Option<String>,
    /// Formular ausfüllen: Fokus, Aufklappliste.
    pub form: crate::forms::FormUi,
    /// Formular bearbeiten: Auswahl, laufender Zug.
    pub form_edit: crate::form_edit::FormEdit,
    /// Noch nicht übernommene Eingaben im Eigenschaften-Panel.
    pub prop_inputs: crate::form_props::PropInputs,
    /// Geprüfte Signaturen.
    pub signatures: crate::signatures::SigState,
}

pub struct FolioApp {
    /// `Err` enthält die Fehlermeldung, wenn PDFium nicht geladen werden konnte.
    pub(crate) render: Result<RenderService, String>,
    pub(crate) document: Option<OpenDocument>,
    /// Fehlermeldung, die als Dialog angezeigt wird.
    pub(crate) error: Option<String>,
    pub(crate) show_thumbnails: bool,
    pub(crate) show_comments: bool,
    pub(crate) tool: Tool,
    pub(crate) tool_settings: ToolSettings,
    /// Aktion, die auf die Antwort "Änderungen speichern?" wartet.
    pub(crate) pending: Option<PendingAction>,
    /// Schließen nach der Rückfrage erlaubt.
    pub(crate) allow_close: bool,
    pub(crate) window_title: String,
    /// Dauerhafte Einstellungen (werden beim Beenden gespeichert).
    pub(crate) settings: Settings,
    pub(crate) show_settings_window: bool,
    pub(crate) insert_dialog: Option<crate::page_dialogs::InsertDialog>,
    pub(crate) merge_dialog: Option<crate::page_dialogs::MergeDialog>,
    pub(crate) extract_dialog: Option<crate::page_dialogs::ExtractDialog>,
    pub(crate) split_dialog: Option<crate::page_dialogs::SplitDialog>,
    /// Laufender Hintergrund-Job (höchstens einer gleichzeitig).
    pub(crate) job: Option<crate::jobs::Job>,
    /// Erfolgsmeldung, die als Dialog angezeigt wird.
    pub(crate) info: Option<String>,
    /// Vertrauenswürdige Zertifikate (aus den Einstellungen) und ein Zähler, der bei jeder
    /// Änderung hochzählt – daran erkennen Dokumente, dass sie neu prüfen müssen.
    pub(crate) trust: foliocraft_sign::TrustStore,
    pub(crate) trust_version: u64,
    pub(crate) show_signatures: bool,
    pub(crate) sign_dialog: Option<crate::sign_ui::SignDialog>,
    pub(crate) cert_dialog: Option<crate::sign_ui::CertDialog>,
    /// Nach erfolgreichem Job diese Datei öffnen (z. B. frisch signierte Fassung).
    pub(crate) open_after_job: Option<PathBuf>,
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
        // Gespeicherte Einstellungen laden (beim ersten Start: Standardwerte).
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, STORAGE_KEY))
            .unwrap_or_default();
        settings.prune_missing();
        cc.egui_ctx.set_theme(settings.theme);

        let mut app = Self {
            render,
            document: None,
            error: None,
            show_thumbnails: settings.show_thumbnails,
            show_comments: settings.show_comments,
            tool: Tool::default(),
            tool_settings: settings.tool_settings(),
            pending: None,
            allow_close: false,
            window_title: String::new(),
            settings,
            show_settings_window: false,
            insert_dialog: None,
            merge_dialog: None,
            extract_dialog: None,
            split_dialog: None,
            job: None,
            info: None,
            trust: Default::default(),
            trust_version: 0,
            show_signatures: false,
            sign_dialog: None,
            cert_dialog: None,
            open_after_job: None,
        };
        app.rebuild_trust();
        if let Some(path) = initial_file {
            app.open_path(&cc.egui_ctx, &path);
        }
        app
    }

    pub fn open_path(&mut self, ctx: &egui::Context, path: &Path) {
        if let Err(e) = &self.render {
            self.error = Some(e.clone());
            return;
        }
        let _ = ctx; // Fenstertitel setzt `update_title` im nächsten Frame
        match Document::open(path) {
            Ok(doc) => {
                self.adopt_document(doc);
                self.settings.add_recent(path);
                tracing::info!(path = %path.display(), "Dokument geöffnet");
            }
            Err(e) => {
                self.error = Some(format!(
                    "„{}“ konnte nicht geöffnet werden:\n{e}",
                    path.display()
                ));
                self.settings.prune_missing(); // z. B. verschobene Datei aus "Zuletzt geöffnet"
            }
        }
    }

    /// Macht ein geladenes Dokument zum aktuellen (ersetzt ein evtl. offenes ohne Rückfrage).
    pub(crate) fn adopt_document(&mut self, doc: Document) {
        let Ok(render) = &self.render else { return };
        let id = crate::pages::new_doc_id();
        // Ungespeicherte Änderungen (z. B. zusammengeführte Seiten) stehen noch nicht in den
        // Dateibytes – dann bekommt der Renderer eine Momentaufnahme des aktuellen Stands.
        if doc.is_dirty() {
            match doc.render_snapshot() {
                Ok(s) => render.open(id, std::sync::Arc::new(s.bytes), s.hidden),
                Err(e) => {
                    self.error = Some(format!("Dokument kann nicht angezeigt werden:\n{e}"));
                    return;
                }
            }
        } else {
            // Arc-Klon: nur der Zeiger wird kopiert, nicht die Daten.
            render.open(id, doc.bytes().clone(), hidden_annots(&doc));
        }
        let sizes: Vec<_> = doc.pages().iter().map(|p| p.display_size()).collect();
        self.document = Some(OpenDocument {
            thumbs: Thumbnails::new(&sizes),
            texts: TextCache::new(id, sizes.len()),
            structure: doc.pages().iter().map(|p| (p.id, p.rotation)).collect(),
            search: SearchState::default(),
            selection: None,
            gesture: None,
            form: Default::default(),
            form_edit: Default::default(),
            prop_inputs: Default::default(),
            signatures: Default::default(),
            editor: None,
            selected: None,
            annot_drag: None,
            viewer: Viewer::new(id, sizes),
            doc,
            page_input: None,
            zoom_input: None,
        });
    }

    pub(crate) fn open_dialog(&mut self, ctx: &egui::Context) {
        // Nativer Dialog; blockiert den UI-Thread, solange er offen ist (normal für Dateidialoge).
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .pick_file()
        {
            self.request_open(ctx, path);
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
                (Some(d), Event::Text { doc, page, text }) => d.texts.insert(doc, page, text),
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
            self.request_open(ctx, path);
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

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        if let Err(e) = &self.render {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new(e).color(ui.visuals().error_fg_color))
            });
            return;
        }
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.25);
            ui.heading("PDF hierher ziehen oder mit Strg+O öffnen");
            if self.settings.recent_files.is_empty() {
                return;
            }
            ui.add_space(24.0);
            ui.label(egui::RichText::new("Zuletzt geöffnet").strong());
            let mut open = None;
            for path in &self.settings.recent_files {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                if ui
                    .link(name)
                    .on_hover_text(path.display().to_string())
                    .clicked()
                {
                    open = Some(path.clone());
                }
            }
            if let Some(path) = open {
                self.request_open(&ctx, path);
            }
        });
    }

    /// Fortschritt des laufenden Jobs; nach dem Ende Erfolgs- bzw. Fehlermeldung.
    fn job_ui(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        match job.poll() {
            None => job.show(ctx),
            Some(result) => {
                tracing::info!(job = %job.title, ?result, "Job beendet");
                self.job = None;
                let open = self.open_after_job.take();
                match result {
                    Ok(msg) => {
                        if let Some(path) = open {
                            self.open_path(ctx, &path);
                            self.show_signatures = true;
                        }
                        self.info = Some(msg);
                    }
                    Err(msg) => self.error = Some(msg),
                }
            }
        }
    }

    fn info_dialog(&mut self, ctx: &egui::Context) {
        let Some(msg) = &self.info else { return };
        let mut close = false;
        egui::Window::new("Fertig")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(msg);
                close = ui.button("OK").clicked();
            });
        if close {
            self.info = None;
        }
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

/// Welche Annotationen der Renderer weglassen soll: alle, die wir selbst zeichnen.
pub(crate) fn hidden_annots(doc: &Document) -> HiddenAnnots {
    (0..doc.page_count())
        .map(|p| (p as u32, doc.managed_annot_indices(p)))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

impl OpenDocument {
    /// Zeichnet alle Annotationen der sichtbaren Seiten (plus Vorschau eines neuen Freitexts).
    pub fn paint_annotations(&self, ui: &egui::Ui, view: &ViewOutput) {
        let painter = ui.painter_at(view.clip);
        for &(page, rect) in &view.pages {
            self.paint_page_annotations(&painter, page, rect);
        }
    }

    /// Annotationen einer Seite in ein beliebiges Rechteck zeichnen (auch für Miniaturen).
    pub fn undo(&mut self) {
        self.annot_drag = None;
        self.gesture = None;
        if let Err(e) = self.doc.undo() {
            tracing::error!(error = %e, "Rückgängig fehlgeschlagen");
        }
        self.validate_selection();
    }

    pub fn redo(&mut self) {
        self.annot_drag = None;
        self.gesture = None;
        if let Err(e) = self.doc.redo() {
            tracing::error!(error = %e, "Wiederholen fehlgeschlagen");
        }
        self.validate_selection();
    }

    pub fn paint_page_annotations(&self, painter: &egui::Painter, page: usize, rect: egui::Rect) {
        let Ok(info) = self.doc.page(page) else {
            return;
        };
        // Wird gerade eine Annotation verschoben, zeigen wir statt ihrer die verschobene Fassung.
        let dragged = self.annot_drag.as_ref().map(|d| &d.preview);
        for a in self.doc.annotations().on_page(page) {
            match dragged {
                Some(p) if p.id == a.id => annot_draw::paint(painter, info, rect, p),
                _ => annot_draw::paint(painter, info, rect, a),
            }
        }
        // Neuer Freitext/Notiz im Eingabefenster: schon während des Tippens anzeigen.
        if let Some(ed) = &self.editor
            && ed.is_new()
            && ed.annotation.page == page
        {
            let mut preview = ed.annotation.clone();
            preview.contents = ed.text.clone();
            annot_draw::paint(painter, info, rect, &preview);
        }
    }
}

impl FolioApp {
    /// Hauptansicht: Seiten zeichnen, Werkzeug-Interaktion, Overlays.
    fn document_view(&mut self, ui: &mut egui::Ui) {
        // Vor dem Ausleihen von `self.document` abfragen (beides sind Teile von `self`).
        let placing = self.is_placing_signature();
        let Some(d) = &mut self.document else { return };
        let view = d.viewer.show(ui);
        let tool = self.tool;
        // Formularfelder lassen sich mit den beiden Auswahlwerkzeugen ausfüllen. Liegt die Maus
        // über einem Feld, bekommt das Werkzeug den Klick nicht.
        let fill_forms = !placing && matches!(tool, Tool::TextSelect | Tool::Select);
        let over_field = fill_forms && d.handle_form_input(ui, &view);
        if fill_forms {
            d.handle_form_keys(ui.ctx(), &view);
        }
        if placing {
            self.handle_sign_placement(ui, &view);
        }
        let Some(d) = &mut self.document else { return };
        match tool {
            _ if over_field || placing => {}
            Tool::Select => {
                d.handle_annotation_select(ui, &view, true);
            }
            // Im Textmodus kann man Annotationen anklicken (auswählen, Doppelklick = bearbeiten);
            // Ziehen markiert weiterhin Text.
            Tool::TextSelect if d.handle_annotation_select(ui, &view, false) => {}
            Tool::TextSelect
            | Tool::Highlight
            | Tool::Underline
            | Tool::StrikeOut
            | Tool::RedactText => {
                // Text der sichtbaren Seiten vorab laden, damit Auswahl sofort geht.
                if let Ok(render) = &self.render {
                    for &(page, _) in &view.pages {
                        d.texts.ensure(render, page);
                    }
                }
                d.handle_text_select(ui, &view);
                // Mit einem Markup-Werkzeug wird die Auswahl beim Loslassen sofort zur Annotation.
                let done = view.response.drag_stopped() || view.response.double_clicked();
                if tool.is_text_markup() && done {
                    d.create_markup_from_selection(tool, &self.tool_settings);
                }
            }
            t if t.is_form_tool() => {
                if let Some(next) = d.handle_form_edit(ui, &view, t) {
                    self.tool = next;
                }
            }
            Tool::Hand => {
                let resp = &view.response;
                if resp.dragged() {
                    d.viewer.scroll_by(-resp.drag_delta());
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
            _ => d.handle_create_tool(ui, &view, tool, &self.tool_settings),
        }
        let editing_form = tool.is_form_tool();
        d.paint_forms(ui, &view, self.settings.highlight_fields || editing_form);
        if fill_forms {
            d.show_field_editor(ui, &view);
        }
        if editing_form {
            d.paint_form_edit(ui, &view);
        }
        d.paint_annotations(ui, &view);
        d.paint_annotation_selection(ui, &view);
        d.paint_gesture_preview(ui, &view, tool, &self.tool_settings);
        d.paint_search_hits(ui, &view);
        d.paint_selection(ui, &view);
        self.paint_sign_area(ui, &view);
    }
}

impl eframe::App for FolioApp {
    /// Wird von eframe regelmäßig und beim Beenden aufgerufen.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.show_thumbnails = self.show_thumbnails;
        self.settings.show_comments = self.show_comments;
        self.settings.store_tool_settings(&self.tool_settings);
        eframe::set_value(storage, STORAGE_KEY, &self.settings);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_render(&ctx);
        self.handle_close_request(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        self.update_title(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        if self.document.is_some() {
            egui::Panel::top("annotation_bar").show(ui, |ui| self.annotation_bar(ui));
        }
        egui::Panel::bottom("statusbar").show(ui, |ui| self.status_bar(ui));

        if let Some(d) = &mut self.document {
            if let Ok(render) = &self.render {
                d.sync_structure(render);
            }
            d.validate_selection();
            d.validate_field_selection();
            d.update_signatures(&self.trust, self.trust_version);
        }
        if self
            .document
            .as_ref()
            .is_some_and(|d| !d.signatures.reports.is_empty())
        {
            egui::Panel::top("signature_banner").show(ui, |ui| self.signature_banner(ui));
        }
        if self.show_signatures && self.document.is_some() && !self.tool.is_form_tool() {
            egui::Panel::right("signatures")
                .default_size(300.0)
                .size_range(220.0..=520.0)
                .show(ui, |ui| self.signatures_panel(ui));
        }
        if let Some(d) = &mut self.document {
            if self.tool.is_form_tool() {
                egui::Panel::right("field_properties")
                    .default_size(280.0)
                    .size_range(220.0..=500.0)
                    .show(ui, |ui| d.field_properties(ui));
            } else if self.show_comments && !self.show_signatures {
                egui::Panel::right("comments")
                    .default_size(280.0)
                    .size_range(200.0..=500.0)
                    .show(ui, |ui| d.comments_panel(ui));
            }
            if d.search.open {
                egui::Panel::top("searchbar").show(ui, |ui| d.search_bar(ui));
                if std::mem::take(&mut d.search.redact_all) {
                    d.redact_search_hits(&self.tool_settings);
                }
            }
            if let Ok(render) = &self.render {
                d.update_search(render);
            }
            if self.show_thumbnails {
                egui::Panel::left("thumbnails")
                    .default_size(160.0)
                    .size_range(100.0..=400.0)
                    .show(ui, |ui| {
                        // Das Closure leiht nur `d.doc`/`d.editor` aus, `show_thumbnails` nur
                        // `d.viewer` – getrennte Felder, daher erlaubt der Borrow-Checker beides.
                        let overlay = |painter: &egui::Painter, page: usize, rect: egui::Rect| {
                            let Ok(info) = d.doc.page(page) else { return };
                            for (f, w) in d.doc.forms().widgets_on(info.object_id) {
                                if f.kind.is_fillable() && !w.hidden {
                                    let style = crate::form_draw::Style::default();
                                    crate::form_draw::paint(painter, info, rect, f, w, style);
                                }
                            }
                            for a in d.doc.annotations().on_page(page) {
                                annot_draw::paint(painter, info, rect, a);
                            }
                        };
                        let ids = d.doc.page_ids();
                        let action = d.viewer.show_thumbnails(ui, &mut d.thumbs, &ids, overlay);
                        match action {
                            Some(ThumbAction::Navigate(page)) => d.viewer.scroll_to_page(page),
                            Some(ThumbAction::Page(a)) => d.apply_page_action(a),
                            None => {}
                        }
                    });
            }
        }
        if self.document.is_some() {
            // Grauer Hintergrund hinter den (weißen) Seiten – im hellen Design sonst unsichtbar.
            let backdrop = if ui.visuals().dark_mode {
                ui.visuals().extreme_bg_color
            } else {
                egui::Color32::from_gray(190)
            };
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(backdrop))
                .show(ui, |ui| self.document_view(ui));
            if let Some(d) = &mut self.document {
                d.show_editor(&ctx);
                if let Ok(render) = &self.render {
                    d.viewer.flush_requests(render);
                }
            }
        } else {
            egui::CentralPanel::default().show(ui, |ui| self.empty_state(ui));
        }
        self.settings_window(&ctx);
        self.insert_dialog_ui(&ctx);
        self.merge_dialog_ui(&ctx);
        self.extract_dialog_ui(&ctx);
        self.split_dialog_ui(&ctx);
        self.sign_dialog_ui(&ctx);
        self.cert_dialog_ui(&ctx);
        self.job_ui(&ctx);
        self.info_dialog(&ctx);
        self.unsaved_changes_dialog(&ctx);
        self.error_dialog(&ctx);
    }
}
