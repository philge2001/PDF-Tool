//! Panel "Lesezeichen": das Inhaltsverzeichnis der PDF als aufklappbarer Baum. Klick springt
//! zum Ziel; Weblinks öffnen nach Rückfrage im Browser.

use crate::app::OpenDocument;
use eframe::egui::{self, Color32, RichText};
use pdftool_core::ObjectId;
use pdftool_core::outline::OutlineItem;

/// Gelesene Lesezeichen und wofür sie gelten (Dateibytes, Seitenreihenfolge).
#[derive(Default)]
pub struct BookmarkState {
    key: Option<(usize, Vec<ObjectId>)>,
    items: Vec<OutlineItem>,
    /// Weblink, der nach Rückfrage geöffnet werden soll.
    pub confirm_uri: Option<String>,
}

/// Was im Baum angeklickt wurde.
enum Clicked {
    Go(usize, Option<f32>),
    Uri(String),
}

impl OpenDocument {
    pub fn bookmarks_pane(&mut self, ui: &mut egui::Ui) {
        let key = (
            std::sync::Arc::as_ptr(self.doc.bytes()) as usize,
            self.doc.pages().iter().map(|p| p.object_id).collect(),
        );
        if self.bookmarks.key.as_ref() != Some(&key) {
            self.bookmarks.items = self.doc.outline();
            self.bookmarks.key = Some(key);
        }
        if self.bookmarks.items.is_empty() {
            ui.label(RichText::new("Dieses Dokument hat keine Lesezeichen.").weak());
            return;
        }
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (i, item) in self.bookmarks.items.iter().enumerate() {
                    tree(ui, item, egui::Id::unique(("bm", i)), &mut clicked);
                }
            });
        match clicked {
            Some(Clicked::Go(page, top)) => self.go_to_destination(page, top),
            Some(Clicked::Uri(u)) => self.bookmarks.confirm_uri = Some(u),
            None => {}
        }
        self.confirm_link(ui.ctx());
    }

    /// Springt zu einer Seite, auf Wunsch an eine Höhe (PDF-Raum).
    pub fn go_to_destination(&mut self, page: usize, top: Option<f32>) {
        if page >= self.doc.page_count() {
            return;
        }
        match (top, self.doc.page(page)) {
            (Some(top), Ok(info)) => {
                let p = info.pdf_to_display_unit(pdftool_core::Point::new(info.crop_box.x0, top));
                self.viewer.scroll_to_page_at(page, p.y);
            }
            _ => self.viewer.scroll_to_page(page),
        }
    }

    fn confirm_link(&mut self, ctx: &egui::Context) {
        let Some(uri) = self.bookmarks.confirm_uri.clone() else {
            return;
        };
        let mut close = false;
        egui::Modal::new(egui::Id::unique("confirm_link")).show(ctx, |ui| {
            ui.heading("Link öffnen?");
            ui.label("Das Lesezeichen verweist auf eine Webseite:");
            ui.label(RichText::new(&uri).monospace());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Im Browser öffnen").clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab(&uri));
                    close = true;
                }
                if ui.button("Abbrechen").clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.bookmarks.confirm_uri = None;
        }
    }
}

/// Ein Eintrag samt Unterpunkten (rekursiv).
fn tree(ui: &mut egui::Ui, item: &OutlineItem, id: egui::Id, clicked: &mut Option<Clicked>) {
    let mut text = RichText::new(&item.title);
    if item.bold {
        text = text.strong();
    }
    if item.italic {
        text = text.italics();
    }
    if let Some([r, g, b]) = item.color {
        text = text.color(Color32::from_rgb(
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8,
        ));
    }
    let mut row = |ui: &mut egui::Ui| {
        let resp = ui.add(egui::Button::new(text.clone()).frame(false).truncate());
        let resp = match &item.uri {
            Some(u) => resp.on_hover_text(u),
            None => resp,
        };
        if resp.clicked() {
            *clicked = match (&item.dest, &item.uri) {
                (Some(d), _) => Some(Clicked::Go(d.page, d.top)),
                (None, Some(u)) => Some(Clicked::Uri(u.clone())),
                _ => None,
            };
        }
    };
    if item.children.is_empty() {
        ui.horizontal(|ui| {
            // Platz des Aufklapp-Pfeils, damit Einträge bündig stehen.
            ui.add_space(ui.spacing().icon_width + ui.spacing().item_spacing.x);
            row(ui);
        });
        return;
    }
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, item.open)
        .show_header(ui, |ui| row(ui))
        .body(|ui| {
            for (i, c) in item.children.iter().enumerate() {
                tree(ui, c, id.with(i), clicked);
            }
        });
}
