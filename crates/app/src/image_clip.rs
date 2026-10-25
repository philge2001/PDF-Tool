//! Bilder kopieren, speichern, einfügen (Phase 11).
//!
//! - **Kopieren / Speichern:** `Document::image_snapshot` baut eine Ein-Bild-PDF in
//!   Originalauflösung, der Render-Worker zeichnet sie (Hilfs-Warteschlange). So sieht das Bild
//!   aus wie in der Anzeige – mit Farbraum, Maske und Beschneidung.
//! - **Einfügen (Strg+V):** Ein Bild aus der Zwischenablage kommt als PNG mittig auf die
//!   sichtbare Seite.
//! - **Rechtsklick** auf ein Bild: Kopieren, Speichern unter, Ersetzen, Löschen.

use crate::app::{OpenDocument, PdfToolApp};
use crate::tools::Tool;
use crate::viewer::{ViewOutput, transform};
use eframe::egui;
use pdftool_core::image_codec::{PixelFormat, encode_jpeg, encode_png};
use pdftool_core::{NewImage, PageImage};
use std::sync::Arc;

/// Was im Rechtsklick-Menü gewählt wurde.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageAction {
    Copy,
    Save,
    Replace,
    Delete,
}

impl OpenDocument {
    /// Rechtsklick-Menü über Bildern. Liefert die gewählte Aktion samt Bild.
    pub fn image_context_menu(
        &mut self,
        view: &ViewOutput,
        tool: Tool,
    ) -> Option<(ImageAction, usize, PageImage)> {
        let resp = &view.response;
        if resp.secondary_clicked() {
            self.content_edit.context_image = resp
                .interact_pointer_pos()
                .and_then(|p| self.image_at(view, p));
            if let Some((page, img)) = &self.content_edit.context_image {
                self.content_edit.selected_image = Some((*page, img.clone()));
            }
        }
        let (page, img) = self.content_edit.context_image.clone()?;
        let editable = matches!(tool, Tool::Select | Tool::EditImage);
        let mut action = None;
        let shown = egui::Popup::context_menu(resp).show(|ui| {
            if ui.button("Bild kopieren").clicked() {
                action = Some(ImageAction::Copy);
            }
            if ui.button("Bild speichern unter…").clicked() {
                action = Some(ImageAction::Save);
            }
            ui.separator();
            ui.add_enabled_ui(editable, |ui| {
                if ui.button("Bild ersetzen…").clicked() {
                    action = Some(ImageAction::Replace);
                }
                if ui.button("Bild löschen").clicked() {
                    action = Some(ImageAction::Delete);
                }
            })
            .response
            .on_disabled_hover_text("Mit „Objekte auswählen“ oder „Bearbeiten → Bilder“");
        });
        if shown.is_none() {
            self.content_edit.context_image = None;
        }
        action.map(|a| (a, page, img))
    }
}

impl PdfToolApp {
    /// Bild in Originalauflösung rendern.
    fn render_image(&self, page: usize, img: &PageImage) -> Result<(u32, u32, Vec<u8>), String> {
        let d = self.document.as_ref().ok_or("Kein Dokument")?;
        let offscreen = self
            .render
            .as_ref()
            .ok()
            .and_then(|r| r.offscreen())
            .ok_or("PDFium ist nicht verfügbar")?;
        let (bytes, size) = d.doc.image_snapshot(page, img).map_err(|e| e.to_string())?;
        let out = offscreen
            .render_page(&Arc::new(bytes), 0, size, 1.0)
            .map_err(|e| e.to_string())?;
        Ok((out.width, out.height, out.pixels))
    }

    pub(crate) fn image_action(
        &mut self,
        ctx: &egui::Context,
        action: ImageAction,
        page: usize,
        img: PageImage,
    ) {
        match action {
            ImageAction::Copy => match self.render_image(page, &img) {
                Ok((w, h, rgba)) => {
                    ctx.copy_image(egui::ColorImage::from_rgba_unmultiplied(
                        [w as usize, h as usize],
                        &rgba,
                    ));
                }
                Err(e) => self.error = Some(format!("Bild kopieren:\n{e}")),
            },
            ImageAction::Save => self.save_image_as(page, &img),
            ImageAction::Replace => {
                // Erst auswählen, dann die Datei wählen: Das Ersetzen passiert, sobald sie
                // da ist (`Pick::ReplaceImage`, im Browser asynchron).
                if let Some(d) = &mut self.document {
                    d.content_edit.selected_image = Some((page, img));
                    crate::toolbar::pick_image(ctx, crate::platform::Pick::ReplaceImage);
                }
            }
            ImageAction::Delete => {
                if let Some(d) = &mut self.document {
                    d.content_edit.selected_image = Some((page, img));
                    d.delete_selected_image();
                }
            }
        }
    }

    /// Strg+C mit ausgewähltem Bild.
    pub(crate) fn copy_selected_image(&mut self, ctx: &egui::Context) -> bool {
        let Some((page, img)) = self
            .document
            .as_ref()
            .and_then(|d| d.content_edit.selected_image.clone())
        else {
            return false;
        };
        self.image_action(ctx, ImageAction::Copy, page, img);
        true
    }

    fn save_image_as(&mut self, page: usize, img: &PageImage) {
        let name = format!(
            "{}-Bild-S{}.png",
            self.document
                .as_ref()
                .map(|d| d.file_name().trim_end_matches(".pdf").to_string())
                .unwrap_or_default(),
            page + 1
        );
        let Some(path) = crate::platform::FileDialog::new()
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .set_file_name(name)
            .save_file()
        else {
            return;
        };
        let result = self.render_image(page, img).and_then(|(w, h, rgba)| {
            let jpeg = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
            let data = if jpeg {
                encode_jpeg(&rgba, w, h, PixelFormat::Rgba, 92)
            } else {
                encode_png(&rgba, w, h, PixelFormat::Rgba)
            }
            .map_err(|e| e.to_string())?;
            crate::platform::write(&path, &data).map_err(|e| e.to_string())
        });
        match result {
            Ok(()) => self.info = Some(format!("Bild gespeichert:\n{}", path.display())),
            Err(e) => self.error = Some(format!("Bild speichern:\n{e}")),
        }
    }

    /// Strg+V mit einem Bild in der Zwischenablage: in die Mitte des sichtbaren Seitenbereichs.
    pub(crate) fn handle_image_paste(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pasted = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::PasteImage(img) => Some(img.clone()),
                _ => None,
            })
        });
        let Some(img) = pasted else {
            return;
        };
        let [w, h] = img.size;
        let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
        match encode_png(&rgba, w as u32, h as u32, PixelFormat::Rgba) {
            Ok(png) => self.insert_pasted_image(&png),
            Err(e) => self.error = Some(format!("Bild einfügen:\n{e}")),
        }
    }

    /// Fügt ein Bild (PNG/JPEG-Bytes) in die Mitte des sichtbaren Seitenbereichs ein. Im Browser
    /// kommen Bilder aus der Zwischenablage als Datei an (`Pick::PasteImage`, siehe `platform`).
    pub(crate) fn insert_pasted_image(&mut self, bytes: &[u8]) {
        let Some(d) = &mut self.document else {
            return;
        };
        let target = d.content_edit.visible_center;
        let page = target.map_or(d.viewer.current_page, |t| t.0);
        let result = NewImage::from_bytes(bytes).and_then(|new| {
            let info = d.doc.page(page)?.clone();
            let center = target.map_or(info.crop_box.center(), |t| t.1);
            let (page_w, _) = info.display_size();
            // 0,75 pt je Pixel = 96 dpi, höchstens halbe Seitenbreite.
            let width = (new.width as f32 * 0.75).min(page_w / 2.0);
            d.doc.insert_image(page, new, center, width)
        });
        if let Err(e) = result {
            self.error = Some(format!("Bild einfügen:\n{e}"));
        } else {
            self.info = None;
        }
    }
}

/// Für das Auswahlwerkzeug: liegt `pos` über einem Bild (oder dessen Anfasser)?
impl OpenDocument {
    pub fn image_under(&mut self, view: &ViewOutput, pos: egui::Pos2) -> bool {
        self.image_handle_rect(view)
            .is_some_and(|h| h.contains(pos))
            || self.image_at(view, pos).is_some()
    }

    /// Nur die Auswahl (Rahmen + Anfasser) zeichnen – für das Auswahlwerkzeug.
    pub fn paint_selected_image(&mut self, ui: &egui::Ui, view: &ViewOutput) {
        let Some((page, img)) = self.content_edit.selected_image.clone() else {
            return;
        };
        let Some((_, page_rect)) = view.pages.iter().find(|(p, _)| *p == page).copied() else {
            return;
        };
        let Ok(info) = self.doc.page(page) else {
            return;
        };
        let painter = ui.painter().with_clip_rect(view.clip);
        let accent = crate::style::ACCENT;
        let r = transform::pdf_rect_to_screen(info, page_rect, img.bounds);
        painter.rect_stroke(
            r,
            0.0,
            egui::Stroke::new(1.5, accent),
            egui::StrokeKind::Outside,
        );
        if let Some(h) = self.image_handle_rect(view) {
            painter.rect_filled(h, 1.0, accent);
        }
        self.paint_image_drag(&painter, view);
    }
}
