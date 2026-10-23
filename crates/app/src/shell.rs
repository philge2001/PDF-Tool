//! Rahmen des Hauptfensters (Phase 10):
//!
//! ```text
//! ┌ Kopfleiste: [☰ Menü] [Startseite] [Dokument ×]                       [⚙] [?] ┐
//! ├ Dokumentleiste: [Alle Werkzeuge] [Bearbeiten] [Konvertieren] [Signieren] … [↶][↷][🔍][💾][⋯]
//! │ Werkzeug- │  Schnell- │                                      │ Panel     │ Sym- │
//! │ bereich   │  werkzeuge│          Seiten (Viewer)             │ (rechts)  │ bol- │
//! │ (links)   │  (schwebt)│                       Seite/Zoom ◰  │           │ leiste│
//! ```
//!
//! Die rechte Symbolleiste schaltet die Panels (Kommentare, Seitenminiaturen, Signaturen,
//! Tags) um; ein zweiter Klick auf dasselbe Symbol schließt das Panel wieder.

use crate::app::PdfToolApp;
use crate::icons::{self, Icon};
use crate::style;
use crate::viewer::{ZOOM_STEPS, ZoomMode};
use eframe::egui::{self, Button, RichText, Vec2};
use serde::{Deserialize, Serialize};

/// Panels der rechten Symbolleiste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RightPane {
    Comments,
    Thumbnails,
    Bookmarks,
    Signatures,
    Tags,
}

impl RightPane {
    pub const ALL: [RightPane; 5] = [
        RightPane::Comments,
        RightPane::Thumbnails,
        RightPane::Bookmarks,
        RightPane::Signatures,
        RightPane::Tags,
    ];

    pub fn icon(self) -> Icon {
        match self {
            RightPane::Comments => Icon::Comment,
            RightPane::Thumbnails => Icon::Thumbnails,
            RightPane::Bookmarks => Icon::Bookmark,
            RightPane::Signatures => Icon::Signature,
            RightPane::Tags => Icon::Accessibility,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RightPane::Comments => "Kommentare",
            RightPane::Thumbnails => "Seitenminiaturen",
            RightPane::Bookmarks => "Lesezeichen",
            RightPane::Signatures => "Signaturen",
            RightPane::Tags => "Tags (Barrierefreiheit)",
        }
    }
}

/// Höhe der Kopf- und der Dokumentleiste.
const BAR_HEIGHT: f32 = 40.0;

impl PdfToolApp {
    /// Öffnet ein Panel rechts (oder schließt es, wenn es schon offen ist).
    pub(crate) fn toggle_pane(&mut self, pane: RightPane) {
        self.right_pane = if self.right_pane == Some(pane) {
            None
        } else {
            Some(pane)
        };
    }

    // ---------------------------------------------------------------- Kopfleiste

    pub(crate) fn global_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal_centered(|ui| {
            ui.set_height(BAR_HEIGHT);
            // ----- Menü -----
            let resp = icons::text_button(ui, Icon::Menu, "Menü", false);
            egui::Popup::menu(&resp)
                .width(260.0)
                .show(|ui| self.main_menu(ui, &ctx));
            ui.add_space(6.0);

            // ----- Tabs -----
            let home = self.document.is_none() || self.show_home;
            if icons::tab(ui, Icon::Home, "Startseite", home, false).clicked() {
                self.show_home = true;
            }
            let mut switch = None;
            let mut close = None;
            for i in 0..self.tab_count() {
                let Some(d) = self.tab(i) else { continue };
                let name = format!(
                    "{}{}",
                    d.file_name(),
                    if d.doc.is_dirty() { " •" } else { "" }
                );
                let active = !home && i == self.active_pos;
                let tab = icons::tab(ui, Icon::Export, &name, active, true);
                if tab.close_clicked {
                    close = Some(i);
                } else if tab.clicked() {
                    switch = Some(i);
                }
            }
            if let Some(i) = switch {
                self.switch_tab(i);
            }
            if let Some(i) = close {
                // Erst zum Tab wechseln, damit die Rückfrage das richtige Dokument nennt.
                self.switch_tab(i);
                self.request_close_document();
            }

            // ----- rechts -----
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Help, "Hilfe: Tastenkürzel", false).clicked() {
                    self.show_shortcuts = !self.show_shortcuts;
                }
                if icons::button(ui, Icon::Settings, "Einstellungen", false).clicked() {
                    self.show_settings_window = true;
                }
                let dark = ui.visuals().dark_mode;
                let (icon, tip) = if dark {
                    (Icon::Sun, "Helles Design")
                } else {
                    (Icon::Moon, "Dunkles Design")
                };
                if icons::button(ui, icon, tip, false).clicked() {
                    self.settings.theme = if dark {
                        egui::ThemePreference::Light
                    } else {
                        egui::ThemePreference::Dark
                    };
                    ui.ctx().set_theme(self.settings.theme);
                }
            });
        });
    }

    // ---------------------------------------------------------------- Dokumentleiste

    pub(crate) fn document_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal_centered(|ui| {
            ui.set_height(BAR_HEIGHT);
            if icons::text_button(ui, Icon::Tools, "Alle Werkzeuge", self.tools_panel).clicked() {
                self.tools_panel = !self.tools_panel;
            }
            let editing = self.tool.is_content_tool();
            if icons::text_button(ui, Icon::Edit, "Bearbeiten", editing).clicked() {
                self.toggle_content_mode();
            }
            let resp = icons::text_button(ui, Icon::Export, "Konvertieren", false);
            egui::Popup::menu(&resp).show(|ui| {
                if ui.button("Exportieren (Bild, Text, Word)…").clicked() {
                    self.open_export_dialog();
                }
                if ui.button("Komprimieren…").clicked() {
                    self.open_compress_dialog();
                }
                if ui.button("Nach PDF/A umwandeln").clicked() {
                    self.start_pdfa(&ctx);
                }
            });
            let resp = icons::text_button(ui, Icon::Signature, "Signieren", false);
            egui::Popup::menu(&resp).show(|ui| {
                if ui.button("Dokument signieren…").clicked() {
                    self.open_sign_dialog();
                }
                if ui.button("Signaturen anzeigen").clicked() {
                    self.right_pane = Some(RightPane::Signatures);
                }
                if ui.button("Testzertifikat erstellen…").clicked() {
                    self.open_cert_dialog();
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let resp = icons::button(ui, Icon::More, "Weitere Optionen", false);
                egui::Popup::menu(&resp).show(|ui| self.more_menu(ui));
                let dirty = self.document.as_ref().is_some_and(|d| d.doc.is_dirty());
                if ui
                    .add_enabled_ui(dirty, |ui| {
                        icons::button(ui, Icon::Save, "Speichern (Strg+S)", false)
                    })
                    .inner
                    .clicked()
                {
                    self.save_document(false);
                }
                let Some(d) = &mut self.document else { return };
                if icons::button(ui, Icon::Search, "Suchen (Strg+F)", d.search.open).clicked() {
                    if d.search.open {
                        d.search.open = false;
                    } else {
                        d.search.open();
                    }
                }
                let redo = d.doc.history().redo_label();
                let r = ui.add_enabled_ui(redo.is_some(), |ui| {
                    icons::button(
                        ui,
                        Icon::Redo,
                        &format!("Wiederholen: {} (Strg+Y)", redo.unwrap_or_default()),
                        false,
                    )
                });
                if r.inner.clicked() {
                    d.redo();
                }
                let undo = d.doc.history().undo_label();
                let r = ui.add_enabled_ui(undo.is_some(), |ui| {
                    icons::button(
                        ui,
                        Icon::Undo,
                        &format!("Rückgängig: {} (Strg+Z)", undo.unwrap_or_default()),
                        false,
                    )
                });
                if r.inner.clicked() {
                    d.undo();
                }
                if let Some(s) = d.doc.security() {
                    ui.add_space(6.0);
                    let r = icons::button(
                        ui,
                        Icon::Lock,
                        &format!(
                            "Geschützt ({}) · erlaubt: {}",
                            s.method,
                            s.permissions.describe()
                        ),
                        false,
                    );
                    if r.clicked() {
                        self.open_security_dialog();
                    }
                }
            });
        });
    }

    /// Menü "⋯" der Dokumentleiste.
    fn more_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Speichern unter…").clicked() {
            self.save_document(true);
        }
        if ui.button("Schützen (Passwort)…").clicked() {
            self.open_security_dialog();
        }
        if ui.button("Barrierefreiheit prüfen…").clicked() {
            self.open_a11y_report();
        }
        ui.separator();
        if let Some(d) = &mut self.document {
            if ui.button("Seitenbreite").clicked() {
                d.viewer.set_zoom_mode(ZoomMode::FitWidth);
            }
            if ui.button("Ganze Seite").clicked() {
                d.viewer.set_zoom_mode(ZoomMode::FitPage);
            }
        }
    }

    // ---------------------------------------------------------------- rechte Seite

    /// Schmale Symbolleiste ganz rechts.
    pub(crate) fn right_rail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            for pane in RightPane::ALL {
                let selected = self.right_pane == Some(pane);
                if icons::button(ui, pane.icon(), pane.label(), selected).clicked() {
                    self.toggle_pane(pane);
                }
                ui.add_space(2.0);
            }
        });
    }

    /// Kopf eines Panels mit Titel und Schließen-Kreuz; `true` = schließen geklickt.
    pub(crate) fn pane_header(ui: &mut egui::Ui, title: &str) -> bool {
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(16.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = icons::sized_button(ui, Icon::Close, "Schließen", false, 26.0).clicked();
            });
        });
        ui.separator();
        close
    }

    // ---------------------------------------------------------------- schwebende Leisten

    /// Seitennavigation und Zoom: schwebend unten rechts über dem Dokument.
    pub(crate) fn page_controls(&mut self, ctx: &egui::Context, viewer: egui::Rect) {
        let Some(d) = &mut self.document else { return };
        let dark = ctx.theme() == egui::Theme::Dark;
        egui::Area::new(egui::Id::unique("page_controls"))
            .order(egui::Order::Middle)
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(viewer.right_bottom() - Vec2::new(18.0, 18.0))
            .show(ctx, |ui| {
                floating_frame(dark).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(0.0, 4.0);
                    ui.vertical_centered(|ui| {
                        let page_count = d.doc.page_count();
                        let current = (d.viewer.current_page + 1).to_string();
                        let mut text = d.page_input.clone().unwrap_or(current);
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut text)
                                .desired_width(34.0)
                                .align(egui::Align2::CENTER_CENTER),
                        );
                        if resp.has_focus() || resp.gained_focus() {
                            d.page_input = Some(text.clone());
                        }
                        if resp.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Enter))
                                && let Ok(n) = text.trim().parse::<usize>()
                            {
                                d.viewer.scroll_to_page(n.clamp(1, page_count) - 1);
                            }
                            d.page_input = None;
                        }
                        ui.label(RichText::new(format!("/ {page_count}")).weak().size(12.0));
                        if icons::sized_button(
                            ui,
                            Icon::ChevronUp,
                            "Vorherige Seite (Bild ↑)",
                            false,
                            30.0,
                        )
                        .clicked()
                        {
                            d.viewer
                                .scroll_to_page(d.viewer.current_page.saturating_sub(1));
                        }
                        if icons::sized_button(
                            ui,
                            Icon::ChevronDown,
                            "Nächste Seite (Bild ↓)",
                            false,
                            30.0,
                        )
                        .clicked()
                        {
                            d.viewer.scroll_to_page(d.viewer.current_page + 1);
                        }
                        ui.separator();
                        if icons::sized_button(ui, Icon::ZoomIn, "Vergrößern (Strg++)", false, 30.0)
                            .clicked()
                        {
                            d.viewer.zoom_step(true);
                        }
                        if icons::sized_button(
                            ui,
                            Icon::ZoomOut,
                            "Verkleinern (Strg+−)",
                            false,
                            30.0,
                        )
                        .clicked()
                        {
                            d.viewer.zoom_step(false);
                        }
                        let fit_page = d.viewer.zoom_mode() == ZoomMode::FitPage;
                        let resp = icons::sized_button(
                            ui,
                            if fit_page {
                                Icon::FitWidth
                            } else {
                                Icon::FitPage
                            },
                            "Anpassen und Zoomstufen",
                            false,
                            30.0,
                        );
                        egui::Popup::menu(&resp)
                            .align(egui::RectAlign::LEFT_END)
                            .show(|ui| {
                                ui.label(
                                    RichText::new(format!("{:.0} %", d.viewer.zoom() * 100.0))
                                        .strong(),
                                );
                                ui.separator();
                                if ui
                                    .add(Button::new("Seitenbreite").shortcut_text("Strg+2"))
                                    .clicked()
                                {
                                    d.viewer.set_zoom_mode(ZoomMode::FitWidth);
                                }
                                if ui
                                    .add(Button::new("Ganze Seite").shortcut_text("Strg+0"))
                                    .clicked()
                                {
                                    d.viewer.set_zoom_mode(ZoomMode::FitPage);
                                }
                                ui.separator();
                                for step in ZOOM_STEPS {
                                    if ui.button(format!("{:.0} %", step * 100.0)).clicked() {
                                        d.viewer.zoom_custom(step, None);
                                    }
                                }
                            });
                    });
                });
            });
    }
}

/// Rahmen schwebender Leisten: abgerundet, mit Schatten.
pub(crate) fn floating_frame(dark: bool) -> egui::Frame {
    egui::Frame::NONE
        .fill(style::floating_fill(dark))
        .corner_radius(10.0)
        .stroke(egui::Stroke::new(1.0, style::border(dark)))
        .inner_margin(egui::Margin::same(5))
        .shadow(egui::epaint::Shadow {
            offset: [0, 2],
            blur: 10,
            spread: 0,
            color: egui::Color32::from_black_alpha(if dark { 90 } else { 40 }),
        })
}

impl PdfToolApp {
    /// Alle Bereiche um ein geöffnetes Dokument (Reihenfolge der Panels = von außen nach innen).
    pub(crate) fn document_layout(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        bar: egui::Frame,
    ) {
        let dark = ui.visuals().dark_mode;
        egui::Panel::top("document_bar")
            .frame(bar)
            .show(ui, |ui| self.document_bar(ui));
        if self
            .document
            .as_ref()
            .is_some_and(|d| !d.signatures.reports.is_empty())
        {
            egui::Panel::top("signature_banner").show(ui, |ui| self.signature_banner(ui));
        }
        if let Some(d) = &mut self.document
            && d.search.open
        {
            egui::Panel::top("searchbar").show(ui, |ui| d.search_bar(ui));
            if std::mem::take(&mut d.search.redact_all) {
                d.redact_search_hits(&self.tool_settings);
            }
        }

        // ----- links: Werkzeugbereich -----
        if self.tools_panel {
            egui::Panel::left("tools_panel")
                .default_size(270.0)
                .size_range(220.0..=420.0)
                .show(ui, |ui| self.tools_panel_ui(ui));
        }

        // ----- rechts: Symbolleiste (ganz außen), daneben das Panel -----
        egui::Panel::right("right_rail")
            .exact_size(48.0)
            .resizable(false)
            .show(ui, |ui| self.right_rail(ui));
        let mut open_autotag = false;
        if self.tool.is_form_tool() {
            if let Some(d) = &mut self.document {
                egui::Panel::right("field_properties")
                    .default_size(290.0)
                    .size_range(220.0..=500.0)
                    .show(ui, |ui| d.field_properties(ui));
            }
        } else if let Some(pane) = self.right_pane {
            let width = if pane == RightPane::Thumbnails {
                200.0
            } else {
                300.0
            };
            egui::Panel::right("right_pane")
                .default_size(width)
                .size_range(150.0..=560.0)
                .show(ui, |ui| {
                    if Self::pane_header(ui, pane.label()) {
                        self.right_pane = None;
                        return;
                    }
                    match pane {
                        RightPane::Signatures => self.signatures_panel(ui),
                        RightPane::Comments => {
                            if let Some(d) = &mut self.document {
                                d.comments_panel(ui);
                            }
                        }
                        RightPane::Tags => {
                            if let Some(d) = &mut self.document {
                                d.tags_panel(ui);
                                open_autotag = std::mem::take(&mut d.tags.request_autotag);
                            }
                        }
                        RightPane::Thumbnails => {
                            if let Some(d) = &mut self.document {
                                d.thumbnails_pane(ui);
                            }
                        }
                        RightPane::Bookmarks => {
                            if let Some(d) = &mut self.document {
                                d.bookmarks_pane(ui);
                            }
                        }
                    }
                });
        }
        if open_autotag {
            self.open_autotag_dialog();
        }

        // ----- Mitte: Seiten -----
        let viewer = egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(style::canvas(dark)))
            .show(ui, |ui| self.document_view(ui))
            .response
            .rect;
        self.quick_tools(ctx, viewer);
        self.page_controls(ctx, viewer);
        self.selection_bar(ctx, viewer);
        if let Some(d) = &mut self.document {
            d.show_editor(ctx);
            if let Ok(render) = &self.render {
                d.viewer.flush_requests(render);
            }
        }
    }

    /// Fenster mit allen Tastenkürzeln (Hilfe-Knopf in der Kopfleiste).
    pub(crate) fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_shortcuts;
        egui::Window::new("Tastenkürzel")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("shortcuts")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for (keys, what) in SHORTCUTS {
                            ui.label(RichText::new(*keys).strong());
                            ui.label(*what);
                            ui.end_row();
                        }
                    });
            });
        self.show_shortcuts = open;
    }
}

const SHORTCUTS: &[(&str, &str)] = &[
    ("Strg+O", "Öffnen"),
    ("Strg+S / Strg+Umschalt+S", "Speichern / Speichern unter"),
    ("Strg+Z / Strg+Y", "Rückgängig / Wiederholen"),
    ("Strg+F", "Suchen (Enter/F3 weiter, Umschalt+Enter zurück)"),
    ("Strg+C", "Markierten Text kopieren"),
    ("Entf", "Auswahl löschen"),
    ("Esc", "Auswahl aufheben / Werkzeug verlassen"),
    ("Strg+Plus / Strg+Minus", "Vergrößern / Verkleinern"),
    (
        "Strg+1 / Strg+2 / Strg+0",
        "100 % / Seitenbreite / Ganze Seite",
    ),
    ("Strg+Mausrad", "Zoomen um den Mauszeiger"),
    (
        "Bild↑ / Bild↓, Pos1 / Ende",
        "Seite zurück / vor, Anfang / Ende",
    ),
    ("Tab / Umschalt+Tab", "Formular: nächstes / voriges Feld"),
    ("Leertaste", "Formular: Kästchen/Option umschalten"),
];
