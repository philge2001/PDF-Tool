//! FolioCraft-Core: Dokumentmodell ohne UI- und ohne PDFium-Abhängigkeit.
//!
//! Alles hier ist reine Logik und kann ohne PDFium-Binary getestet werden.
//! PDF-Strukturen werden über `lopdf` gelesen und geschrieben.

pub mod document;
pub mod error;
pub mod geometry;

pub use document::{Document, PageInfo};
pub use error::{CoreError, Result};
pub use geometry::{Color, Point, Quad, Rect};
