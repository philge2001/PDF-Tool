//! Farbschema und Abstände der Oberfläche (hell und dunkel).
//!
//! Helle, ruhige Flächen, ein grauer Hintergrund hinter den Seiten, ein blauer Akzent für
//! aktive Elemente und abgerundete Schaltflächen. Werkzeug-Icons im Werkzeugbereich bekommen je
//! nach Kategorie eine eigene Farbe (`icon_tint`), damit man sie schneller wiederfindet.

use crate::icons::Icon;
use eframe::egui::{self, Color32, CornerRadius, Stroke, Theme};

/// Akzentfarbe (Auswahl, aktive Werkzeuge, Hauptschaltflächen).
pub const ACCENT: Color32 = Color32::from_rgb(38, 128, 235);

/// Hintergrund hinter den Seiten.
pub fn canvas(dark: bool) -> Color32 {
    if dark {
        Color32::from_gray(30)
    } else {
        Color32::from_gray(222)
    }
}

/// Fläche schwebender Leisten (Schnellwerkzeuge, Seitennavigation).
pub fn floating_fill(dark: bool) -> Color32 {
    if dark {
        Color32::from_gray(52)
    } else {
        Color32::WHITE
    }
}

/// Feine Trennlinien zwischen den Bereichen.
pub fn border(dark: bool) -> Color32 {
    if dark {
        Color32::from_gray(64)
    } else {
        Color32::from_gray(214)
    }
}

/// Wendet das Schema auf beide Designs an (egui wählt je nach Einstellung).
pub fn install(ctx: &egui::Context) {
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |style| {
            let dark = theme == Theme::Dark;
            let v = &mut style.visuals;
            let (panel, window, faint, extreme, text) = if dark {
                (
                    Color32::from_gray(40),
                    Color32::from_gray(46),
                    Color32::from_gray(48),
                    Color32::from_gray(24),
                    Color32::from_gray(225),
                )
            } else {
                (
                    Color32::from_gray(250),
                    Color32::WHITE,
                    Color32::from_gray(243),
                    Color32::WHITE,
                    Color32::from_gray(44),
                )
            };
            v.panel_fill = panel;
            v.window_fill = window;
            v.faint_bg_color = faint;
            v.extreme_bg_color = extreme;
            // Eingabefelder: im hellen Design sonst weiß auf weißem Dialog und ohne Rahmen –
            // also unsichtbar, bis man hineinklickt.
            v.text_edit_bg_color = (!dark).then(|| Color32::from_gray(242));
            v.window_stroke = Stroke::new(1.0, border(dark));
            v.window_corner_radius = CornerRadius::same(10);
            v.menu_corner_radius = CornerRadius::same(8);
            v.override_text_color = Some(text);
            v.hyperlink_color = ACCENT;
            v.selection.bg_fill = ACCENT;
            v.selection.stroke = Stroke::new(1.0, if dark { Color32::WHITE } else { ACCENT });
            let r = CornerRadius::same(6);
            let w = &mut v.widgets;
            for wv in [
                &mut w.noninteractive,
                &mut w.inactive,
                &mut w.hovered,
                &mut w.active,
                &mut w.open,
            ] {
                wv.corner_radius = r;
            }
            w.noninteractive.bg_stroke = Stroke::new(1.0, border(dark));
            let (idle, hover, press) = if dark {
                (
                    Color32::from_gray(58),
                    Color32::from_gray(70),
                    Color32::from_gray(84),
                )
            } else {
                (
                    Color32::from_gray(236),
                    Color32::from_gray(226),
                    Color32::from_gray(212),
                )
            };
            w.inactive.weak_bg_fill = idle;
            w.inactive.bg_fill = idle;
            w.inactive.bg_stroke = Stroke::NONE;
            w.hovered.weak_bg_fill = hover;
            w.hovered.bg_fill = hover;
            w.hovered.bg_stroke = Stroke::NONE;
            w.active.weak_bg_fill = press;
            w.active.bg_fill = press;
            w.open.weak_bg_fill = hover;

            let s = &mut style.spacing;
            s.item_spacing = egui::vec2(8.0, 6.0);
            s.button_padding = egui::vec2(10.0, 5.0);
            s.interact_size.y = 26.0;
            s.menu_margin = egui::Margin::same(8);
            s.window_margin = egui::Margin::same(14);
        });
    }
}

/// Farbe eines Werkzeug-Icons im Werkzeugbereich (Kategorie-Farben).
pub fn icon_tint(icon: Icon, default: Color32) -> Color32 {
    let rgb = |r, g, b| Color32::from_rgb(r, g, b);
    match icon {
        Icon::Comment | Icon::Note | Icon::Highlight => rgb(232, 170, 20),
        Icon::Edit | Icon::AddText | Icon::Image => rgb(226, 72, 136),
        Icon::Export | Icon::Compress | Icon::Archive => rgb(30, 160, 120),
        Icon::Combine | Icon::Organize | Icon::Insert | Icon::Split => rgb(60, 140, 230),
        Icon::Scan => rgb(20, 160, 180),
        Icon::Lock | Icon::Redact | Icon::Sanitize => rgb(200, 60, 60),
        Icon::Form | Icon::Signature | Icon::Certificate => rgb(140, 90, 220),
        Icon::Compare => rgb(230, 110, 40),
        Icon::Accessibility => rgb(40, 120, 200),
        _ => default,
    }
}
