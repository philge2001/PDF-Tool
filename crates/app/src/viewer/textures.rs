//! Verwaltet die GPU-Texturen gerenderter Seiten/Kacheln.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use pdftool_render::{ByteLru, RenderKey, RgbaImage};
use std::collections::HashSet;

/// Wie viel Texturspeicher wir maximal belegen (in Bytes).
const TEXTURE_BUDGET: usize = 512 * 1024 * 1024;

pub struct PageTextures {
    cache: ByteLru<RenderKey, TextureHandle>,
    /// Bereits beim Worker angefragt, Ergebnis steht noch aus.
    pending: HashSet<RenderKey>,
    /// Was im letzten Frame gebraucht wurde (für `Retain`).
    last_wanted: HashSet<RenderKey>,
}

impl Default for PageTextures {
    fn default() -> Self {
        Self {
            cache: ByteLru::new(TEXTURE_BUDGET),
            pending: HashSet::new(),
            last_wanted: HashSet::new(),
        }
    }
}

impl PageTextures {
    /// Nimmt ein Render-Ergebnis entgegen und lädt es als Textur hoch.
    pub fn insert(&mut self, ctx: &egui::Context, key: RenderKey, image: RgbaImage) {
        self.pending.remove(&key);
        let size = [image.width as usize, image.height as usize];
        // PDFium liefert deckende Pixel (Alpha 255) – damit ist "premultiplied" identisch und schneller.
        let color = ColorImage::from_rgba_premultiplied(size, &image.pixels);
        let name = format!("page{}-{}-{:?}", key.page, key.scale_milli, key.tile);
        let tex = ctx.load_texture(name, color, TextureOptions::LINEAR);
        let bytes = image.pixels.len();
        self.cache.insert(key, tex, bytes);
    }

    pub fn mark_failed(&mut self, key: &RenderKey) {
        self.pending.remove(key);
    }

    pub fn get(&mut self, key: &RenderKey) -> Option<&TextureHandle> {
        self.cache.get(key)
    }

    pub fn is_pending(&self, key: &RenderKey) -> bool {
        self.pending.contains(key)
    }

    pub fn has(&self, key: &RenderKey) -> bool {
        self.cache.contains(key)
    }

    /// Der Worker hat alle offenen Aufträge verworfen (z. B. nach Neuladen).
    pub fn reset_pending(&mut self) {
        self.pending.clear();
        self.last_wanted.clear();
    }

    pub fn mark_pending(&mut self, key: RenderKey) {
        self.pending.insert(key);
    }

    /// Beste vorhandene Ganzseiten-Textur einer Seite (beliebige Skalierung) als Platzhalter,
    /// bevorzugt die schärfste. Gibt eine Kopie des Handles zurück (billig: nur ein Zähler).
    pub fn best_fallback(&self, doc: u64, page: u32) -> Option<TextureHandle> {
        self.cache
            .keys()
            .filter(|k| k.doc == doc && k.page == page && k.tile.is_none())
            .max_by_key(|k| k.scale_milli)
            .and_then(|k| self.cache.peek(k).cloned())
    }

    /// Nach einer Strukturänderung (Seiten umsortiert/gelöscht/gedreht): behält die Texturen
    /// der Seiten, für die `new_page` einen neuen Index liefert, und ordnet sie dem neuen
    /// Dokument zu. Alles andere wird freigegeben.
    pub fn remap(&mut self, new_doc: u64, new_page: impl Fn(u32) -> Option<u32>) {
        for (key, tex, bytes) in self.cache.drain() {
            if let Some(page) = new_page(key.page) {
                let key = RenderKey {
                    doc: new_doc,
                    page,
                    ..key
                };
                self.cache.insert(key, tex, bytes);
            }
        }
        self.reset_pending();
    }

    /// Gibt `Some(wanted)` zurück, wenn sich die gewünschte Menge geändert hat und der
    /// Worker informiert werden muss. Nicht mehr gewünschte Aufträge gelten als verworfen.
    pub fn update_wanted(&mut self, wanted: HashSet<RenderKey>) -> Option<HashSet<RenderKey>> {
        if wanted == self.last_wanted {
            return None;
        }
        self.pending.retain(|k| wanted.contains(k));
        self.last_wanted = wanted.clone();
        Some(wanted)
    }
}
