//! FolioCraft-Render: kapselt PDFium (Rendering, Textextraktion) in einem eigenen
//! Hintergrund-Thread. Kennt egui nicht – die App bekommt rohe RGBA-Puffer.

pub mod cache;
pub mod error;
pub mod loader;
pub mod worker;

pub use cache::ByteLru;
pub use error::RenderError;
pub use worker::{
    DocId, Event, HiddenAnnots, Priority, RenderKey, RenderRequest, RenderService, RgbaImage,
    TILE_SIZE, needs_tiles, page_pixel_size, tile_grid, tile_rect,
};
