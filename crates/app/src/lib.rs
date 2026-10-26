//! PDF Tool – die Oberfläche (eframe/egui) als Bibliothek.
//!
//! Der Desktop-Start steht in `main.rs`; der PDF Webeditor (github.com/philge2001/PDF-Webeditor)
//! startet dieselbe [`PdfToolApp`] im Browser. Was sich zwischen Desktop und Browser
//! unterscheidet (Dateien, Dialoge, Hintergrund-Jobs), steckt in `platform`.

mod a11y_ui;
mod annot_draw;
mod app;
mod bookmarks;
mod comments;
mod compare;
mod compress_ui;
mod content_edit;
mod export;
mod file_ops;
mod form_draw;
mod form_edit;
mod form_props;
mod forms;
mod home;
mod icons;
mod image_clip;
mod image_tool;
mod jobs;
mod menu;
#[cfg(has_ocr)]
mod ocr_ui;
mod page_dialogs;
mod pages;
mod platform;
mod redact;
mod rich_edit;
mod search;
mod security_ui;
mod settings;
mod shell;
mod shortcuts;
mod sign_ui;
mod signatures;
mod style;
mod tabs;
mod tags_ui;
mod text_cache;
mod toolbar;
mod tools;
mod tools_panel;
mod viewer;

pub use app::PdfToolApp;
