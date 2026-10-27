//! Die Haupt-App: hält Render-Service, geöffnetes Dokument und UI-Zustand.

use crate::annot_draw;
use crate::file_ops::PendingAction;
use crate::search::SearchState;
use crate::settings::{LEGACY_STORAGE_KEY, STORAGE_KEY, Settings};
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
use pdftool_core::{AnnotId, Document};
use pdftool_render::{Event, HiddenAnnots, RenderService};
use std::path::{Path, PathBuf};

/// Ein geöffnetes Dokument samt Ansicht.
pub struct OpenDocument {
    pub doc: Document,
    pub viewer: Viewer,
    pub thumbs: Thumbnails,
    pub texts: TextCache,
    /// Letzter bekannter Stand der Seitenstruktur (siehe `sync_structure`).
    pub structure: Vec<(pdftool_core::PageId, u16, u64)>,
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
    /// Formular ausfüllen: Fokus, Aufklappliste.
    pub form: crate::forms::FormUi,
    /// Formular bearbeiten: Auswahl, laufender Zug.
    pub form_edit: crate::form_edit::FormEdit,
    /// Noch nicht übernommene Eingaben im Eigenschaften-Panel.
    pub prop_inputs: crate::form_props::PropInputs,
    /// Geprüfte Signaturen.
    pub signatures: crate::signatures::SigState,
    /// Inhalt bearbeiten: Textblöcke, laufende Eingabe.
    pub content_edit: crate::content_edit::ContentEdit,
    /// Panel "Tags": Strukturbaum, Auswahl, Eingaben.
    pub tags: crate::tags_ui::TagState,
    /// Panel "Lesezeichen".
    pub bookmarks: crate::bookmarks::BookmarkState,
}

pub struct PdfToolApp {
    /// `Err` enthält die Fehlermeldung, wenn PDFium nicht geladen werden konnte.
    pub(crate) render: Result<RenderService, String>,
    /// Aktives Dokument (Tab). Weitere offene Dokumente liegen in `others`, siehe `tabs`.
    pub(crate) document: Option<OpenDocument>,
    pub(crate) others: Vec<OpenDocument>,
    /// Position des aktiven Tabs in der Tab-Reihenfolge.
    pub(crate) active_pos: usize,
    /// Fehlermeldung, die als Dialog angezeigt wird.
    pub(crate) error: Option<String>,
    /// Offenes Panel rechts (Kommentare, Miniaturen, …) – `None` = nur die Symbolleiste.
    pub(crate) right_pane: Option<crate::shell::RightPane>,
    /// Werkzeugbereich links offen? Und welche Werkzeuggruppe darin (None = "Alle Werkzeuge").
    pub(crate) tools_panel: bool,
    pub(crate) tool_group: Option<crate::tools_panel::ToolGroup>,
    /// Startseite statt Dokument anzeigen (Tab "Startseite").
    pub(crate) show_home: bool,
    /// Fenster mit den Tastenkürzeln.
    pub(crate) show_shortcuts: bool,
    /// Stil für neuen Text ("Text hinzufügen").
    pub(crate) new_text_style: pdftool_core::rich_text::TextStyle,
    /// Werkzeug, das nach dem Öffnen einer Datei starten soll (Kachel auf der Startseite).
    pub(crate) pending_group: Option<crate::tools_panel::ToolGroup>,
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
    pub(crate) trust: pdftool_sign::TrustStore,
    pub(crate) trust_version: u64,
    pub(crate) sign_dialog: Option<crate::sign_ui::SignDialog>,
    pub(crate) cert_dialog: Option<crate::sign_ui::CertDialog>,
    /// Nach erfolgreichem Job diese Datei öffnen (z. B. frisch signierte Fassung).
    pub(crate) open_after_job: Option<PathBuf>,
    /// Ergebnis-Dokument eines laufenden Jobs (z. B. Schwärzen) und sein Pfad.
    pub(crate) pending_doc: Option<(crate::redact::DocSlot, Option<PathBuf>)>,
    pub(crate) export_dialog: Option<crate::export::ExportDialog>,
    /// Passwortabfrage beim Öffnen und Dialog "Sicherheit".
    pub(crate) password_prompt: Option<crate::security_ui::PasswordPrompt>,
    pub(crate) security_dialog: Option<crate::security_ui::SecurityDialog>,
    pub(crate) compress_dialog: Option<crate::compress_ui::CompressDialog>,
    /// Fenster "Barrierefreiheit prüfen" und Dialog "Automatisch taggen".
    pub(crate) a11y_report: Option<crate::a11y_ui::A11yReport>,
    pub(crate) autotag_dialog: Option<crate::a11y_ui::AutoTagDialog>,
    /// Dokumentvergleich: Ergebnis des laufenden Jobs, offenes Vergleichsfenster.
    pub(crate) pending_compare: Option<(
        crate::compare::CompareSlot,
        pdftool_render::OffscreenRenderer,
    )>,
    pub(crate) compare: Option<crate::compare::CompareView>,
    /// Dialog "Texterkennung" und Ergebnis des laufenden OCR-Jobs.
    #[cfg(has_ocr)]
    pub(crate) ocr_dialog: Option<crate::ocr_ui::OcrDialog>,
    #[cfg(has_ocr)]
    pub(crate) pending_ocr: Option<crate::ocr_ui::OcrSlot>,
    pub(crate) redact_confirm: bool,
    pub(crate) sanitize_dialog: Option<crate::redact::SanitizeDialog>,
}

/// Name der egui-Schriftfamilie für die eingebettete Noto Sans (Freitext-Overlay).
pub const NOTO_FAMILY: &str = "noto";

/// Macht die eingebaute Noto Sans in egui verfügbar: als eigene Familie (Freitext sieht im
/// Overlay aus wie in der Datei) und als Ersatz für Zeichen, die der UI-Schrift fehlen.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        NOTO_FAMILY.to_owned(),
        // `from_static`: die Bytes leben das ganze Programm (`&'static [u8]`), kein Kopieren.
        std::sync::Arc::new(egui::FontData::from_static(
            pdftool_core::embedded_font::NOTO_SANS,
        )),
    );
    fonts.families.insert(
        egui::FontFamily::Name(NOTO_FAMILY.into()),
        vec![NOTO_FAMILY.to_owned()],
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(NOTO_FAMILY.to_owned());
    }
    ctx.set_fonts(fonts);
}

impl PdfToolApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        initial_files: Vec<PathBuf>,
        compare_with: Option<PathBuf>,
    ) -> Self {
        // egui würde Strg+Plus/Minus sonst für die UI-Vergrößerung nutzen – wir zoomen das Dokument.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        install_fonts(&cc.egui_ctx);
        crate::style::install(&cc.egui_ctx);
        #[cfg(target_arch = "wasm32")]
        crate::platform::install_web_hooks(&cc.egui_ctx);

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
            .and_then(|s| {
                eframe::get_value(s, STORAGE_KEY)
                    .or_else(|| eframe::get_value(s, LEGACY_STORAGE_KEY))
            })
            .unwrap_or_default();
        settings.prune_missing();
        crate::style::apply(&cc.egui_ctx, settings.appearance());

        let mut app = Self {
            render,
            document: None,
            others: Vec::new(),
            active_pos: 0,
            error: None,
            right_pane: settings.right_pane,
            tools_panel: settings.tools_panel,
            tool_group: None,
            show_home: false,
            show_shortcuts: false,
            pending_group: None,
            new_text_style: pdftool_core::rich_text::TextStyle::noto(
                settings.font_size,
                settings.tool_settings().color(Tool::AddText),
            ),
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
            sign_dialog: None,
            cert_dialog: None,
            open_after_job: None,
            pending_doc: None,
            export_dialog: None,
            password_prompt: None,
            security_dialog: None,
            compress_dialog: None,
            a11y_report: None,
            autotag_dialog: None,
            pending_compare: None,
            compare: None,
            #[cfg(has_ocr)]
            ocr_dialog: None,
            #[cfg(has_ocr)]
            pending_ocr: None,
            redact_confirm: false,
            sanitize_dialog: None,
        };
        app.rebuild_trust();
        for path in initial_files {
            app.open_path(&cc.egui_ctx, &path);
        }
        if let Some(other) = compare_with {
            app.start_compare_with(&cc.egui_ctx, other);
        }
        app
    }

    pub fn open_path(&mut self, ctx: &egui::Context, path: &Path) {
        if let Err(e) = &self.render {
            self.error = Some(e.clone());
            return;
        }
        let _ = ctx; // Fenstertitel setzt `update_title` im nächsten Frame
        if self.focus_open_file(path) {
            return;
        }
        match crate::platform::open_document(path, None) {
            Ok(doc) => {
                self.open_in_new_tab(doc);
                self.settings.add_recent(path);
                tracing::info!(path = %path.display(), "Dokument geöffnet");
            }
            Err(e) if self.handle_open_error(path, &e) => {}
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
        self.show_home = false;
        // Wunsch der Datei, welches Panel beim Öffnen zu sehen ist (/PageMode).
        match doc.page_mode().as_str() {
            "UseOutlines" if doc.has_outline() => {
                self.right_pane = Some(crate::shell::RightPane::Bookmarks);
            }
            "UseThumbs" => self.right_pane = Some(crate::shell::RightPane::Thumbnails),
            _ => {}
        }
        self.document = Some(OpenDocument {
            thumbs: Thumbnails::new(&sizes),
            texts: TextCache::new(id, sizes.len()),
            structure: doc
                .pages()
                .iter()
                .map(|p| (p.id, p.rotation, doc.content_stamp(p.object_id)))
                .collect(),
            search: SearchState::default(),
            selection: None,
            gesture: None,
            form: Default::default(),
            form_edit: Default::default(),
            prop_inputs: Default::default(),
            signatures: Default::default(),
            content_edit: Default::default(),
            tags: Default::default(),
            bookmarks: Default::default(),
            editor: None,
            selected: None,
            annot_drag: None,
            viewer: Viewer::new(id, sizes),
            doc,
            page_input: None,
        });
    }

    /// Datei öffnen; das Ergebnis kommt über `handle_picked` (`Pick::OpenPdf`).
    pub(crate) fn open_dialog(&mut self, ctx: &egui::Context) {
        crate::platform::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .pick(ctx, crate::platform::Pick::OpenPdf, true);
    }

    /// Verarbeitet gewählte Dateien (siehe `platform`: im Browser kommen sie asynchron an).
    fn handle_picked(&mut self, ctx: &egui::Context) {
        use crate::platform::Pick;
        while let Some((pick, paths)) = crate::platform::take_picked() {
            match pick {
                Pick::OpenPdf => {
                    for path in paths {
                        self.request_open(ctx, path);
                    }
                }
                Pick::InsertPages => {
                    if let Some(path) = paths.into_iter().next() {
                        self.insert_from_file(&path);
                    }
                }
                Pick::MergeFiles => {
                    if let Some(dlg) = &mut self.merge_dialog {
                        dlg.add_files(paths);
                    }
                }
                Pick::Compare => {
                    if let Some(path) = paths.into_iter().next() {
                        self.start_compare_with(ctx, path);
                    }
                }
                Pick::Certificate => {
                    if let Some(path) = paths.into_iter().next() {
                        self.import_certificate(&path);
                    }
                }
                Pick::SigningId => {
                    if let Some(path) = paths.into_iter().next() {
                        self.set_signing_id(path);
                    }
                }
                Pick::PasteImage => {
                    // Nicht, wenn gerade in ein Textfeld eingefügt wird.
                    if ctx.egui_wants_keyboard_input() {
                        continue;
                    }
                    if let Some(bytes) = paths.first().and_then(|p| crate::platform::read(p).ok()) {
                        self.insert_pasted_image(&bytes);
                    }
                }
                Pick::InsertImage | Pick::ReplaceImage => {
                    let Some(path) = paths.into_iter().next() else {
                        continue;
                    };
                    let bytes = match crate::platform::read(&path) {
                        Ok(b) => b,
                        Err(e) => {
                            self.error = Some(format!("„{}“: {e}", path.display()));
                            continue;
                        }
                    };
                    if let Some(d) = &mut self.document {
                        if pick == Pick::InsertImage {
                            d.start_image_insert(&bytes);
                        } else {
                            d.replace_selected_image(&bytes);
                        }
                    }
                }
            }
        }
    }

    /// Holt fertige Render-Ergebnisse ab.
    fn poll_render(&mut self, ctx: &egui::Context) {
        let Ok(render) = &self.render else { return };
        for event in render.poll() {
            match (&mut self.document, event) {
                (Some(d), Event::OpenFailed { doc, error }) if doc == d.viewer.doc_id => {
                    self.error = Some(format!("PDFium kann das Dokument nicht öffnen: {error}"));
                    self.close_active_tab();
                    break;
                }
                (Some(d), Event::Text { doc, page, text }) => d.texts.insert(doc, page, text),
                (Some(d), ev) => d.viewer.on_render_event(ctx, ev),
                (None, _) => {}
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        // Browser: Dateien haben keinen Pfad, ihr Inhalt kommt asynchron (siehe `platform`).
        #[cfg(target_arch = "wasm32")]
        {
            let files = ctx.input(|i| i.raw.dropped_files.clone());
            crate::platform::receive_dropped(ctx, files);
        }
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
                        self.take_pending_doc();
                        self.take_pending_compare();
                        #[cfg(has_ocr)]
                        self.take_pending_ocr();
                        if let Some(path) = open {
                            self.open_path(ctx, &path);
                            self.right_pane = Some(crate::shell::RightPane::Signatures);
                        }
                        self.info = Some(msg);
                    }
                    Err(msg) => {
                        self.pending_doc = None;
                        self.pending_compare = None;
                        #[cfg(has_ocr)]
                        {
                            self.pending_ocr = None;
                        }
                        self.error = Some(msg);
                    }
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

    /// Panel "Seitenminiaturen" (rechts): Navigation, Mehrfachauswahl, Kontextmenü, Ziehen.
    pub fn thumbnails_pane(&mut self, ui: &mut egui::Ui) {
        // Das Closure leiht nur `self.doc` aus, `show_thumbnails` nur `self.viewer`/`self.thumbs`
        // – getrennte Felder, daher erlaubt der Borrow-Checker beides gleichzeitig.
        let doc = &self.doc;
        let overlay = |painter: &egui::Painter, page: usize, rect: egui::Rect| {
            let Ok(info) = doc.page(page) else { return };
            for (f, w) in doc.forms().widgets_on(info.object_id) {
                if f.kind.is_fillable() && !w.hidden {
                    let style = crate::form_draw::Style::default();
                    crate::form_draw::paint(painter, info, rect, f, w, style);
                }
            }
            for a in doc.annotations().on_page(page) {
                annot_draw::paint(painter, info, rect, a);
            }
        };
        let ids = doc.page_ids();
        let action = self
            .viewer
            .show_thumbnails(ui, &mut self.thumbs, &ids, overlay);
        match action {
            Some(ThumbAction::Navigate(page)) => self.viewer.scroll_to_page(page),
            Some(ThumbAction::Page(a)) => self.apply_page_action(a),
            None => {}
        }
    }

    pub fn undo(&mut self) {
        self.annot_drag = None;
        self.content_edit.editing = None;
        self.content_edit.selected = None;
        self.content_edit.drag = None;
        self.content_edit.selected_image = None;
        self.content_edit.image_drag = None;
        self.gesture = None;
        if let Err(e) = self.doc.undo() {
            tracing::error!(error = %e, "Rückgängig fehlgeschlagen");
        }
        self.validate_selection();
    }

    pub fn redo(&mut self) {
        self.annot_drag = None;
        self.content_edit.editing = None;
        self.content_edit.selected = None;
        self.content_edit.drag = None;
        self.content_edit.selected_image = None;
        self.content_edit.image_drag = None;
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

impl PdfToolApp {
    /// Hauptansicht: Seiten zeichnen, Werkzeug-Interaktion, Overlays.
    pub(crate) fn document_view(&mut self, ui: &mut egui::Ui) {
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
                // Über einem Bild (und keiner Annotation) oder während eines Bild-Zugs:
                // Bild auswählen, verschieben, skalieren. Sonst Annotationen.
                let pointer = ui
                    .input(|i| i.pointer.press_origin())
                    .or(view.response.hover_pos());
                let on_annot = pointer.is_some_and(|p| d.annotation_at(&view, p).is_some());
                let on_image = !on_annot && pointer.is_some_and(|p| d.image_under(&view, p));
                if d.content_edit.image_drag.is_some() || on_image {
                    d.selected = None;
                    d.handle_image_edit(ui, &view);
                } else {
                    if view.response.clicked() {
                        d.content_edit.selected_image = None;
                    }
                    d.handle_annotation_select(ui, &view, true);
                }
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
            t if t.is_content_tool() => {
                d.content_edit.new_style = self.new_text_style.clone();
                if t == Tool::EditImage {
                    d.handle_image_edit(ui, &view);
                } else {
                    d.handle_content_edit(ui, &view, t);
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
        if self.right_pane == Some(crate::shell::RightPane::Tags) {
            d.paint_tags(ui, &view);
        }
        d.paint_search_hits(ui, &view);
        d.paint_selection(ui, &view);
        if tool == Tool::EditImage {
            d.paint_image_edit(ui, &view);
        } else if tool == Tool::Select {
            d.paint_selected_image(ui, &view);
        }
        // Sichtbare Mitte merken: Strg+V legt Bilder dort ab, nicht in der (evtl. verdeckten) Seitenmitte.
        let mid = view.clip.center();
        d.content_edit.visible_center = view
            .pages
            .iter()
            .min_by(|a, b| {
                a.1.distance_to_pos(mid)
                    .total_cmp(&b.1.distance_to_pos(mid))
            })
            .and_then(|&(page, rect)| {
                let info = d.doc.page(page).ok()?;
                let p = rect.intersect(view.clip).center();
                Some((page, crate::viewer::transform::screen_to_pdf(info, rect, p)))
            });
        let image_action = if tool.is_form_tool() {
            None
        } else {
            d.image_context_menu(&view, tool)
        };
        if tool.is_content_tool() {
            d.paint_content_edit(ui, &view, tool);
        } else if d.content_edit.editing.is_some() {
            // Modus verlassen: laufende Eingabe übernehmen.
            d.commit_content_edit();
        }
        if let Some(e) = d.content_edit.error.take() {
            self.error = Some(e);
        }
        self.paint_sign_area(ui, &view);
        if let Some((action, page, img)) = image_action {
            let ctx = ui.ctx().clone();
            self.image_action(&ctx, action, page, img);
        }
    }
}

impl PdfToolApp {
    /// Gewählte Darstellung (für den PDF Webeditor: gemeinsames Cookie mit Portal und Hausbuch).
    pub fn appearance(&self) -> crate::style::Appearance {
        self.settings.appearance()
    }

    /// Darstellung setzen und sofort anwenden (Menü, Kopfleiste oder von außen).
    pub fn set_appearance(&mut self, ctx: &egui::Context, appearance: crate::style::Appearance) {
        self.settings.set_appearance(appearance);
        crate::style::apply(ctx, appearance);
    }
}

impl eframe::App for PdfToolApp {
    /// Wird von eframe regelmäßig und beim Beenden aufgerufen.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.right_pane = self.right_pane;
        self.settings.tools_panel = self.tools_panel;
        self.settings.store_tool_settings(&self.tool_settings);
        eframe::set_value(storage, STORAGE_KEY, &self.settings);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_render(&ctx);
        self.handle_picked(&ctx);
        crate::platform::set_unsaved(self.first_dirty_tab().is_some());
        self.handle_close_request(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        self.handle_image_paste(&ctx);
        self.update_title(&ctx);

        self.sync_tool_group();
        let dark = ui.visuals().dark_mode;
        // Leisten ohne eigenen Rahmen: die Trennlinie zum Nachbarbereich zieht das Panel selbst
        // (Haarlinie wie `.topbar` im Portal).
        let bar = |fill| {
            egui::Frame::NONE
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(10, 2))
        };
        let panel_fill = ui.visuals().panel_fill;
        let window_fill = ui.visuals().window_fill;
        egui::Panel::top("global_bar")
            .frame(bar(panel_fill))
            .show(ui, |ui| self.global_bar(ui));

        if let Some(d) = &mut self.document {
            if let Ok(render) = &self.render {
                d.sync_structure(render);
            }
            d.validate_selection();
            d.validate_field_selection();
            d.update_signatures(&self.trust, self.trust_version);
            if let Ok(render) = &self.render {
                d.update_search(render);
            }
        }
        if self.document.is_some()
            && !self.show_home
            && let Some(g) = self.pending_group.take()
        {
            self.open_tool_group(&ctx, g);
        }
        if self.document.is_none() || self.show_home {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(crate::style::background(dark)))
                .show(ui, |ui| self.home_page(ui));
        } else {
            self.document_layout(ui, &ctx, bar(window_fill));
        }
        self.settings_window(&ctx);
        self.shortcuts_window(&ctx);
        self.insert_dialog_ui(&ctx);
        self.merge_dialog_ui(&ctx);
        self.extract_dialog_ui(&ctx);
        self.split_dialog_ui(&ctx);
        self.redact_confirm_ui(&ctx);
        self.sanitize_dialog_ui(&ctx);
        self.export_dialog_ui(&ctx);
        self.password_prompt_ui(&ctx);
        self.security_dialog_ui(&ctx);
        self.compress_dialog_ui(&ctx);
        self.a11y_report_ui(&ctx);
        self.autotag_dialog_ui(&ctx);
        self.compare_ui(&ctx);
        #[cfg(has_ocr)]
        self.ocr_dialog_ui(&ctx);
        self.sign_dialog_ui(&ctx);
        self.cert_dialog_ui(&ctx);
        self.job_ui(&ctx);
        self.info_dialog(&ctx);
        self.unsaved_changes_dialog(&ctx);
        self.error_dialog(&ctx);
    }
}
