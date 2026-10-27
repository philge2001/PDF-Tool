//! Startseite (Tab "Startseite"): Willkommen, empfohlene Werkzeuge als Kacheln und die Liste
//! "Zuletzt verwendet" mit Ort, Änderungsdatum und Größe.

use crate::app::PdfToolApp;
use crate::icons::{self, Icon};
use crate::style;
use crate::tools_panel::ToolGroup;
use eframe::egui::{self, RichText, Sense, Vec2};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Kacheln "Empfohlene Werkzeuge": Gruppe und Kurzbeschreibung.
const RECOMMENDED: &[(ToolGroup, &str)] = &[
    (ToolGroup::Edit, "Text und Bilder ändern"),
    (ToolGroup::Comment, "Markieren, Notizen, Zeichnen"),
    (ToolGroup::Combine, "Mehrere PDFs zu einer"),
    (ToolGroup::Organize, "Drehen, löschen, einfügen"),
    (ToolGroup::Scan, "Scans durchsuchbar machen"),
    (ToolGroup::Protect, "Passwort und Rechte"),
    (ToolGroup::Compress, "Datei verkleinern"),
    (ToolGroup::Form, "Formularfelder anlegen"),
];

/// "heute", "gestern", "vor 3 Tagen", sonst "vor N Wochen/Monaten".
pub fn relative_age(modified: SystemTime, now: SystemTime) -> String {
    let secs = now
        .duration_since(modified)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    match days {
        0 => "heute".into(),
        1 => "gestern".into(),
        2..=13 => format!("vor {days} Tagen"),
        14..=59 => format!("vor {} Wochen", days / 7),
        60..=729 => format!("vor {} Monaten", days / 30),
        _ => format!("vor {} Jahren", days / 365),
    }
}

/// Dateigröße lesbar (KB/MB).
pub fn human_size(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0).replace('.', ","),
    }
}

impl PdfToolApp {
    /// Startseite (auch, wenn kein Dokument offen ist).
    pub(crate) fn home_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Err(e) = &self.render {
            ui.add_space(16.0);
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        let dark = ui.visuals().dark_mode;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                // Mittige Spalte mit höchstens 1080 px wie `.home` im Portal.
                let full = ui.available_rect_before_wrap();
                let width = (full.width() - 48.0).clamp(220.0, MAX_WIDTH);
                let column = egui::Rect::from_min_size(
                    egui::pos2(full.center().x - width / 2.0, full.top() + 40.0),
                    Vec2::new(width, full.height()),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(column), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Willkommen bei PDF Tool").size(34.0).strong());
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new("Öffne eine PDF oder wähle ein Werkzeug.")
                                .size(16.0)
                                .weak(),
                        );
                        ui.add_space(16.0);
                        if icons::primary_button(ui, Icon::Open, "Datei öffnen").clicked() {
                            self.open_dialog(&ctx);
                        }
                    });
                    ui.add_space(36.0);
                    section(ui, "Empfohlene Werkzeuge");
                    let mut chosen = None;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(16.0, 16.0);
                        for &(group, hint) in RECOMMENDED {
                            if tile(ui, dark, group, hint).clicked() {
                                chosen = Some(group);
                            }
                        }
                    });
                    if let Some(g) = chosen {
                        self.start_tool_from_home(&ctx, g);
                    }
                    ui.add_space(32.0);
                    self.recent_list(ui, &ctx);
                    ui.add_space(48.0);
                });
            });
    }

    /// Werkzeug von der Startseite: ohne offenes Dokument erst eine Datei wählen.
    fn start_tool_from_home(&mut self, ctx: &egui::Context, group: ToolGroup) {
        if matches!(group, ToolGroup::Combine) {
            return self.open_tool_group(ctx, group);
        }
        if self.document.is_none() {
            self.open_dialog(ctx);
            if self.document.is_none() {
                // Abgebrochen – oder die Datei braucht erst ein Passwort.
                self.pending_group = self.password_prompt.as_ref().map(|_| group);
                return;
            }
        }
        self.show_home = false;
        self.open_tool_group(ctx, group);
    }

    fn recent_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        section(ui, "Zuletzt verwendet");
        // Liste als Karte (`.card` im Portal).
        let dark = ui.visuals().dark_mode;
        egui::Frame::NONE
            .fill(style::surface(dark))
            .stroke(egui::Stroke::new(1.0, style::border(dark)))
            .corner_radius(18.0)
            .inner_margin(egui::Margin::symmetric(18, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.recent_rows(ui, ctx);
            });
    }

    fn recent_rows(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.settings.recent_files.is_empty() {
            ui.label(
                RichText::new("Noch keine Dateien. Eine PDF hierher ziehen oder „Datei öffnen“.")
                    .weak(),
            );
            return;
        }
        let now = pdftool_core::date::system_now();
        let mut open: Option<PathBuf> = None;
        let mut remove: Option<PathBuf> = None;
        let row_h = 40.0;
        let dark = ui.visuals().dark_mode;
        egui::Grid::new("recent")
            .num_columns(5)
            .min_row_height(row_h)
            .spacing(Vec2::new(24.0, 0.0))
            .show(ui, |ui| {
                for h in ["Name", "Ort", "Geändert", "Größe", ""] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                // Im Browser lassen sich nur Dateien dieser Sitzung erneut öffnen.
                let reachable =
                    |p: &&PathBuf| !crate::platform::is_web() || crate::platform::exists(p);
                for path in self.settings.recent_files.iter().filter(reachable) {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    // Im Browser gibt es keine Dateiinfos (Datum/Größe bleiben leer).
                    let meta = std::fs::metadata(path).ok();
                    let resp = ui
                        .horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                            icons::paint(ui.painter(), r, Icon::Export, style::accent(dark));
                            ui.add(
                                egui::Label::new(RichText::new(&name).strong())
                                    .sense(Sense::click()),
                            )
                        })
                        .inner;
                    if resp.on_hover_text("Öffnen").clicked() {
                        open = Some(path.clone());
                    }
                    ui.label(
                        RichText::new(
                            path.parent()
                                .map(Path::display)
                                .map(|d| d.to_string())
                                .unwrap_or_default(),
                        )
                        .weak(),
                    );
                    ui.label(
                        meta.as_ref()
                            .and_then(|m| m.modified().ok())
                            .map(|m| relative_age(m, now))
                            .unwrap_or_default(),
                    );
                    ui.label(meta.map(|m| human_size(m.len())).unwrap_or_default());
                    if icons::sized_button(ui, Icon::Close, "Aus der Liste entfernen", false, 24.0)
                        .clicked()
                    {
                        remove = Some(path.clone());
                    }
                    ui.end_row();
                }
            });
        if let Some(p) = remove {
            self.settings.recent_files.retain(|r| r != &p);
        }
        if let Some(p) = open {
            self.request_open(ctx, p);
        }
    }
}

/// Größte Breite der Startseite (wie `.home` im Portal).
const MAX_WIDTH: f32 = 1080.0;

/// Abschnittstitel der Startseite: klein, grau, in Versalien (`.group > h2` im Portal).
fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title.to_uppercase()).size(12.0).weak());
    ui.add_space(8.0);
}

/// Kachel für ein empfohlenes Werkzeug: Karte mit App-Symbol, Name und Kurzbeschreibung
/// (`.tile` im Portal).
fn tile(ui: &mut egui::Ui, dark: bool, group: ToolGroup, hint: &str) -> egui::Response {
    let size = Vec2::new(200.0, 120.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let radius = egui::CornerRadius::same(18);
        // Überfahren: Schatten wird kräftiger, wie das Anheben der Kacheln im Portal.
        let mut shadow = style::soft_shadow(dark);
        if resp.hovered() {
            shadow.offset[1] += 4;
            shadow.blur = shadow.blur.saturating_add(12);
            shadow.color = shadow.color.gamma_multiply(1.8);
        }
        ui.painter().add(shadow.as_shape(rect, radius));
        ui.painter().rect_filled(rect, radius, style::surface(dark));
        ui.painter().rect_stroke(
            rect,
            radius,
            egui::Stroke::new(1.0, style::border(dark)),
            egui::StrokeKind::Inside,
        );
        let icon_rect =
            egui::Rect::from_min_size(rect.min + Vec2::new(16.0, 16.0), Vec2::splat(40.0));
        icons::glyph(ui.painter(), icon_rect, group.icon(), dark);
        ui.painter().text(
            rect.min + Vec2::new(16.0, 68.0),
            egui::Align2::LEFT_TOP,
            group.label(),
            egui::FontId::proportional(15.0),
            v.text_color(),
        );
        ui.painter().text(
            rect.min + Vec2::new(16.0, 90.0),
            egui::Align2::LEFT_TOP,
            hint,
            egui::FontId::proportional(12.5),
            v.weak_text_color(),
        );
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn ages_and_sizes() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(400 * 86_400);
        let ago = |d: u64| now - Duration::from_secs(d * 86_400 + 60);
        assert_eq!(relative_age(now, now), "heute");
        assert_eq!(relative_age(ago(1), now), "gestern");
        assert_eq!(relative_age(ago(5), now), "vor 5 Tagen");
        assert_eq!(relative_age(ago(21), now), "vor 3 Wochen");
        assert_eq!(relative_age(ago(90), now), "vor 3 Monaten");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(20 * 1024), "20 KB");
        assert_eq!(human_size(3 * 1_048_576 / 2), "1,5 MB");
    }
}
