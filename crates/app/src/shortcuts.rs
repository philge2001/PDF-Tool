//! Globale Tastenkürzel.

use crate::app::FolioApp;
use crate::viewer::ZoomMode;
use eframe::egui::{self, Key, Modifiers, vec2};

/// Wie weit Pfeiltasten scrollen (egui-Punkte).
const ARROW_SCROLL: f32 = 50.0;

impl FolioApp {
    pub(crate) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // `consume_key` prüft UND entfernt das Ereignis, damit es kein anderes Widget bekommt.
        let cmd = |key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, key));

        if cmd(Key::O) {
            self.open_dialog(ctx);
        }

        let Some(d) = &mut self.document else { return };
        let v = &mut d.viewer;

        if cmd(Key::Plus) || cmd(Key::Equals) {
            v.zoom_step(true);
        }
        if cmd(Key::Minus) {
            v.zoom_step(false);
        }
        if cmd(Key::Num1) {
            v.zoom_custom(1.0, None);
        }
        if cmd(Key::Num2) {
            v.set_zoom_mode(ZoomMode::FitWidth);
        }
        if cmd(Key::Num0) {
            v.set_zoom_mode(ZoomMode::FitPage);
        }

        // Navigationstasten nur, wenn gerade kein Textfeld tippt.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed = |key| ctx.input(|i| i.key_pressed(key) && i.modifiers.is_none());
        if pressed(Key::PageDown) {
            v.scroll_to_page(v.current_page + 1);
        }
        if pressed(Key::PageUp) {
            v.scroll_to_page(v.current_page.saturating_sub(1));
        }
        if pressed(Key::Home) {
            v.scroll_to_page(0);
        }
        if pressed(Key::End) {
            v.scroll_to_page(usize::MAX);
        }
        if pressed(Key::ArrowDown) {
            v.scroll_by(vec2(0.0, ARROW_SCROLL));
        }
        if pressed(Key::ArrowUp) {
            v.scroll_by(vec2(0.0, -ARROW_SCROLL));
        }
    }
}
