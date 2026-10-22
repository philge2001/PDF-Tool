//! FolioCraft-Core: Dokumentmodell ohne UI- und ohne PDFium-Abhängigkeit.
//!
//! Alles hier ist reine Logik und kann ohne PDFium-Binary getestet werden.
//! PDF-Strukturen werden über `lopdf` gelesen und geschrieben.

pub mod annotation;
pub mod command;
pub mod date;
pub mod document;
pub mod error;
pub mod font;
pub mod geometry;
pub mod page_ops;
pub mod pdf_string;
mod save;
pub mod text;

pub use annotation::{AnnotId, AnnotKind, Annotation, AnnotationStore};
pub use command::{
    AddAnnotation, Batch, Command, History, Model, RemoveAnnotation, UpdateAnnotation,
};
pub use document::{Document, PageId, PageInfo, RenderSnapshot};
pub use error::{CoreError, Result};
pub use geometry::{Color, Point, Quad, Rect};
pub use page_ops::{DeletePages, ReorderPages, RotatePages};
pub use text::{CharBox, PageText, TextRange};
