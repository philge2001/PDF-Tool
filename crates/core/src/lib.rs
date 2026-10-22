//! FolioCraft-Core: Dokumentmodell ohne UI- und ohne PDFium-Abhängigkeit.
//!
//! Alles hier ist reine Logik und kann ohne PDFium-Binary getestet werden.
//! PDF-Strukturen werden über `lopdf` gelesen und geschrieben.

pub mod annotation;
pub mod command;
pub mod content;
pub mod date;
pub mod document;
pub mod embedded_font;
pub mod error;
pub mod font;
pub mod form;
mod form_save;
pub mod geometry;
pub mod page_export;
pub mod page_import;
pub mod page_ops;
pub mod pdf_string;
pub mod raster;
pub mod redact;
pub mod sanitize;
mod save;
pub mod signature_prep;
pub mod text;

pub use annotation::{AnnotId, AnnotKind, Annotation, AnnotationStore};
pub use command::{
    AddAnnotation, AddField, Batch, Command, History, Model, RemoveAnnotation, RemoveField,
    UpdateAnnotation, UpdateField,
};
pub use document::{Document, PageId, PageInfo, RenderSnapshot};
pub use error::{CoreError, Result};
pub use form::{FieldId, FieldKind, FieldValue, FormField, FormStore, Widget};
pub use geometry::{Color, Point, Quad, Rect};
pub use page_export::{extract_pages, split_by_ranges, split_every};
pub use page_import::{InsertPages, parse_page_ranges};
pub use page_ops::{DeletePages, ReorderPages, RotatePages};
pub use text::{CharBox, PageText, TextRange};
