//! Farbschema und Abstände der Oberfläche (hell und dunkel).
//!
//! Gestaltet wie das Portal (github.com/philge2001/portal) und das Hausbuch
//! (github.com/philge2001/vermietung): Apple-Systemfarben, hellgrauer Fenstergrund, weiße Flächen,
//! Haarlinien, Systemblau als Akzent, runde Pill-Buttons und weiche Schatten. Werkzeug-Icons
//! bekommen je nach Kategorie eine eigene Farbe (`icon_tint`), in Listen und auf der Startseite als
//! farbiges Quadrat mit weißem Symbol (`icons::glyph`), wie die Seitenleiste im Hausbuch.
//!
//! Alle Farben stehen in [`Palette`]; die Namen folgen den CSS-Variablen des Portals.

use crate::icons::Icon;
use eframe::egui::{self, Color32, CornerRadius, Shadow, Stroke, Theme};

/// Akzentfarbe im hellen Design (Systemblau). Für das aktuelle Design: [`accent`].
pub const ACCENT: Color32 = Color32::from_rgb(0, 113, 227);

/// Farb-Tokens eines Designs. Halbtransparente Portal-Farben (z. B. `rgba(118, 118, 128, 0.12)`)
/// sind schon mit dem Untergrund verrechnet: egui zeichnet deckend, ohne Unschärfe dahinter.
pub struct Palette {
    /// Fenstergrund (`--bg`): Startseite.
    pub background: Color32,
    /// Leisten und Seitenbereiche (wie die Glasflächen im Hausbuch).
    pub panel: Color32,
    /// Hintergrund hinter den PDF-Seiten.
    pub canvas: Color32,
    /// Deckende Flächen (`--surface-solid`): Dialoge, Menüs, Kacheln, Karten.
    pub surface: Color32,
    /// Schwebende Leisten über dem Dokument.
    pub floating: Color32,
    /// Eingabefelder (graue Füllung wie das iOS-Suchfeld).
    pub field: Color32,
    /// Sekundär-Buttons (`--surface-2`): Ruhe, überfahren, gedrückt.
    pub fill: Color32,
    pub fill_hover: Color32,
    pub fill_press: Color32,
    /// Haarlinien (`--border`).
    pub border: Color32,
    /// Text (`--text`, `--text-2`).
    pub text: Color32,
    pub text_2: Color32,
    /// Systemblau, -rot, -orange.
    pub accent: Color32,
    pub danger: Color32,
    pub warn: Color32,
    /// Grau für Symbole ohne eigene Kategorie-Farbe (`.c-graphite`).
    pub graphite: Color32,
    /// Farbe der Schatten (`--shadow-lg`).
    pub shadow: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const LIGHT: Palette = Palette {
    background: rgb(242, 242, 247),
    panel: rgb(250, 250, 252),
    canvas: rgb(232, 232, 237),
    surface: rgb(255, 255, 255),
    floating: rgb(255, 255, 255),
    field: rgb(236, 236, 240),
    fill: rgb(236, 236, 240),
    fill_hover: rgb(226, 226, 231),
    fill_press: rgb(214, 214, 219),
    border: rgb(220, 220, 225),
    text: rgb(29, 29, 31),
    text_2: rgb(110, 110, 115),
    accent: ACCENT,
    danger: rgb(215, 0, 21),
    warn: rgb(201, 52, 0),
    graphite: rgb(142, 142, 147),
    shadow: Color32::from_black_alpha(30),
};

const DARK: Palette = Palette {
    background: rgb(0, 0, 0),
    panel: rgb(28, 28, 30),
    canvas: rgb(0, 0, 0),
    surface: rgb(28, 28, 30),
    floating: rgb(44, 44, 46),
    field: rgb(44, 44, 46),
    fill: rgb(44, 44, 46),
    fill_hover: rgb(58, 58, 60),
    fill_press: rgb(72, 72, 74),
    border: rgb(56, 56, 58),
    text: rgb(245, 245, 247),
    text_2: rgb(161, 161, 166),
    accent: rgb(10, 132, 255),
    danger: rgb(255, 69, 58),
    warn: rgb(255, 159, 10),
    graphite: rgb(99, 99, 102),
    shadow: Color32::from_black_alpha(110),
};

/// Farben des hellen bzw. dunklen Designs.
pub fn palette(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

/// Akzentfarbe des Designs.
pub fn accent(dark: bool) -> Color32 {
    palette(dark).accent
}

/// Hinterlegung ausgewählter Schaltflächen: Akzent, leicht getönt (`--color-accent-soft`).
pub fn accent_soft(dark: bool) -> Color32 {
    accent(dark).gamma_multiply(if dark { 0.22 } else { 0.12 })
}

/// Hintergrund hinter den Seiten.
pub fn canvas(dark: bool) -> Color32 {
    palette(dark).canvas
}

/// Fenstergrund der Startseite.
pub fn background(dark: bool) -> Color32 {
    palette(dark).background
}

/// Fläche schwebender Leisten (Schnellwerkzeuge, Seitennavigation).
pub fn floating_fill(dark: bool) -> Color32 {
    palette(dark).floating
}

/// Deckende Fläche für Kacheln und Karten.
pub fn surface(dark: bool) -> Color32 {
    palette(dark).surface
}

/// Feine Trennlinien zwischen den Bereichen.
pub fn border(dark: bool) -> Color32 {
    palette(dark).border
}

/// Weicher Schatten unter Karten und schwebenden Leisten (`--shadow`).
pub fn soft_shadow(dark: bool) -> Shadow {
    Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 80 } else { 18 }),
    }
}

/// Wendet das Schema auf beide Designs an (egui wählt je nach Einstellung).
pub fn install(ctx: &egui::Context) {
    for theme in [Theme::Light, Theme::Dark] {
        ctx.style_mut_of(theme, |style| {
            let t = palette(theme == Theme::Dark);
            let v = &mut style.visuals;
            v.panel_fill = t.panel;
            v.window_fill = t.surface;
            v.faint_bg_color = t.field;
            v.extreme_bg_color = t.surface;
            // Eingabefelder grau gefüllt: auf weißen Dialogen sonst unsichtbar, bis man hineinklickt.
            v.text_edit_bg_color = Some(t.field);
            v.code_bg_color = t.field;
            v.override_text_color = Some(t.text);
            v.weak_text_color = Some(t.text_2);
            v.hyperlink_color = t.accent;
            v.error_fg_color = t.danger;
            v.warn_fg_color = t.warn;
            v.selection.bg_fill = t.accent;
            // Fokusrahmen von Eingabefeldern und Vordergrund ausgewählter Schaltflächen.
            v.selection.stroke = Stroke::new(1.0, t.accent);

            // Dialoge wie `dialog` im Portal, Menüs wie `.menu`.
            v.window_stroke = Stroke::new(1.0, t.border);
            v.window_corner_radius = CornerRadius::same(18);
            v.menu_corner_radius = CornerRadius::same(14);
            v.window_shadow = Shadow {
                offset: [0, 16],
                blur: 48,
                spread: 0,
                color: t.shadow,
            };
            v.popup_shadow = Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: t.shadow,
            };

            let w = &mut v.widgets;
            for wv in [
                &mut w.noninteractive,
                &mut w.inactive,
                &mut w.hovered,
                &mut w.active,
                &mut w.open,
            ] {
                wv.corner_radius = CornerRadius::same(8);
                wv.fg_stroke.color = t.text;
            }
            // Trennlinien zwischen Panels und `ui.separator()`.
            w.noninteractive.bg_fill = t.panel;
            w.noninteractive.weak_bg_fill = t.panel;
            w.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
            w.noninteractive.fg_stroke.color = t.text_2;
            for (wv, fill) in [
                (&mut w.inactive, t.fill),
                (&mut w.hovered, t.fill_hover),
                (&mut w.active, t.fill_press),
                (&mut w.open, t.fill_hover),
            ] {
                wv.bg_fill = fill;
                wv.weak_bg_fill = fill;
                wv.bg_stroke = Stroke::NONE;
                // Kein „Aufblähen“ beim Überfahren: Apple-Buttons bleiben ruhig stehen.
                wv.expansion = 0.0;
            }
            // egui füllt damit auch die Titelzeile des aktiven Fensters; im Portal haben Dialoge
            // keine abgesetzte Titelzeile. Geöffnete Menüknöpfe zeigen dadurch eine helle Fläche.
            w.open.weak_bg_fill = t.surface;

            let s = &mut style.spacing;
            s.item_spacing = egui::vec2(8.0, 6.0);
            s.button_padding = egui::vec2(12.0, 5.0);
            s.interact_size.y = 28.0;
            s.menu_margin = egui::Margin::same(6);
            s.window_margin = egui::Margin::same(20);
        });
    }
}

/// Farbe eines Werkzeug-Icons im Werkzeugbereich (Kategorie-Farben, die Icon-Farben des Portals
/// `.c-yellow`, `.c-pink`, …). Kräftig genug für ein weißes Symbol darauf (`icons::glyph`).
pub fn icon_tint(icon: Icon, default: Color32) -> Color32 {
    match icon {
        Icon::Comment | Icon::Note | Icon::Highlight => rgb(245, 184, 0),
        Icon::Edit | Icon::AddText | Icon::Image => rgb(255, 45, 85),
        Icon::Export | Icon::Compress | Icon::Archive => rgb(40, 167, 69),
        Icon::Combine | Icon::Organize | Icon::Insert | Icon::Split => rgb(0, 122, 255),
        Icon::Scan => rgb(10, 147, 150),
        Icon::Lock | Icon::Redact | Icon::Sanitize => rgb(255, 59, 48),
        Icon::Form | Icon::Signature | Icon::Certificate => rgb(137, 68, 171),
        Icon::Compare => rgb(255, 149, 0),
        Icon::Accessibility => rgb(88, 86, 214),
        _ => default,
    }
}
