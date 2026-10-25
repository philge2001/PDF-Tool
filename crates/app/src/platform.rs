//! Unterschiede zwischen Desktop und Browser (PDF Webeditor): Dateidialoge, Dateien lesen und
//! schreiben, Hintergrund-Arbeit.
//!
//! - **Desktop:** echte Dateien und native Dialoge (rfd), Hintergrund-Arbeit in Threads.
//! - **Browser:** Es gibt kein Dateisystem. Gewählte Dateien landen in einem Speicher im
//!   Browser („virtuelle Dateien“, Schlüssel = Dateiname); Schreiben bietet die Datei als
//!   Download an. „Speichern unter“ fragt nicht nach einem Ort – der Browser lädt herunter.
//!   Dateiauswahl ist dort asynchron: Das Ergebnis kommt erst in einem späteren Frame an.
//!   Deshalb liefern Auswahl-Dialoge ihr Ergebnis auf beiden Plattformen über [`take_picked`]
//!   mit einem [`Pick`], der sagt, wofür die Datei gedacht ist. Threads gibt es im Browser
//!   nicht: [`spawn`] führt die Arbeit sofort aus.

use eframe::egui;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};

use pdftool_core::Document;

/// Wofür eine gewählte Datei gedacht ist (bestimmt, was `PdfToolApp::handle_picked` tut).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pick {
    /// PDF(s) öffnen, jede als Tab.
    OpenPdf,
    /// Seiten aus einer anderen PDF einfügen.
    InsertPages,
    /// Weitere Dateien für „Dateien zusammenführen“.
    MergeFiles,
    /// „Vergleichen mit…“.
    Compare,
    /// Vertrauenswürdiges Zertifikat importieren.
    Certificate,
    /// Digitale ID (PKCS#12) zum Signieren.
    SigningId,
    /// Bild einfügen (Werkzeug „Bilder“).
    InsertImage,
    /// Ausgewähltes Bild ersetzen.
    ReplaceImage,
    /// Bild aus der Zwischenablage (Browser: Strg+V, siehe `install_web_hooks`).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    PasteImage,
}

thread_local! {
    // Gewählte, noch nicht verarbeitete Dateien. Nur der UI-Thread greift zu.
    static PICKED: RefCell<VecDeque<(Pick, Vec<PathBuf>)>> = const { RefCell::new(VecDeque::new()) };
}

fn push_picked(pick: Pick, paths: Vec<PathBuf>) {
    if !paths.is_empty() {
        PICKED.with(|p| p.borrow_mut().push_back((pick, paths)));
    }
}

/// Nächste gewählte Datei(en), die noch verarbeitet werden müssen.
pub(crate) fn take_picked() -> Option<(Pick, Vec<PathBuf>)> {
    PICKED.with(|p| p.borrow_mut().pop_front())
}

/// Dateidialog mit derselben Bauweise wie `rfd::FileDialog`.
#[derive(Default)]
pub(crate) struct FileDialog {
    filters: Vec<(String, Vec<String>)>,
    file_name: Option<String>,
    title: Option<String>,
    // Im Browser bedeutungslos (kein Dateisystem).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    directory: Option<PathBuf>,
}

impl FileDialog {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn add_filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        let exts = extensions.iter().map(|e| e.to_string()).collect();
        self.filters.push((name.into(), exts));
        self
    }

    pub(crate) fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    /// Startordner des Dialogs (nur Desktop).
    pub(crate) fn set_directory(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directory = Some(dir.into());
        self
    }

    pub(crate) fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Datei(en) wählen. Das Ergebnis kommt über [`take_picked`] (Desktop: sofort, Browser:
    /// sobald der Nutzer gewählt hat). Abbrechen liefert nichts.
    pub(crate) fn pick(self, ctx: &egui::Context, pick: Pick, multiple: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dialog = self.native();
            let paths = if multiple {
                dialog.pick_files().unwrap_or_default()
            } else {
                dialog.pick_file().into_iter().collect()
            };
            push_picked(pick, paths);
            ctx.request_repaint();
        }
        #[cfg(target_arch = "wasm32")]
        web::pick(self, ctx.clone(), pick, multiple);
    }

    /// Speicherort wählen. Browser: kein Dialog – der Dateiname (oder „Dokument“ mit der
    /// Endung des ersten Filters) wird beim Schreiben als Download angeboten.
    pub(crate) fn save_file(self) -> Option<PathBuf> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.native().save_file()
        }
        #[cfg(target_arch = "wasm32")]
        {
            let ext = self.filters.first().and_then(|(_, e)| e.first()).cloned();
            let name = self.file_name.unwrap_or_else(|| match ext {
                Some(ext) => format!("Dokument.{ext}"),
                None => "Dokument".into(),
            });
            Some(PathBuf::from(name))
        }
    }

    /// Ordner wählen. Browser: kein Dialog – jede Datei darin wird einzeln heruntergeladen.
    pub(crate) fn pick_folder(self) -> Option<PathBuf> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.native().pick_folder()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Some(PathBuf::new())
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn native(self) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        for (name, exts) in &self.filters {
            d = d.add_filter(name, exts);
        }
        if let Some(n) = self.file_name {
            d = d.set_file_name(n);
        }
        if let Some(t) = self.title {
            d = d.set_title(t);
        }
        if let Some(dir) = self.directory {
            d = d.set_directory(dir);
        }
        d
    }
}

/// Liest eine Datei (Browser: eine zuvor gewählte virtuelle Datei).
pub(crate) fn read(path: &Path) -> io::Result<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::read(path)
    }
}

/// Schreibt eine Datei (Browser: Download, zusätzlich als virtuelle Datei gemerkt).
pub(crate) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::write(path, bytes)
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::write(path, bytes)
    }
}

/// Gibt es die Datei (noch)? Browser: nur gewählte oder gespeicherte Dateien dieser Sitzung.
pub(crate) fn exists(path: &Path) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        path.is_file()
    }
    #[cfg(target_arch = "wasm32")]
    {
        web::exists(path)
    }
}

/// Öffnet ein PDF (optional mit Passwort) und merkt sich den Pfad fürs Speichern.
pub(crate) fn open_document(path: &Path, password: Option<&str>) -> pdftool_core::Result<Document> {
    let bytes = read(path).map_err(|source| pdftool_core::CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut doc = Document::from_bytes_with_password(bytes, password)?;
    doc.set_path(Some(path.to_path_buf()));
    Ok(doc)
}

/// Speichert das Dokument unter `path` (Desktop: atomar in die Datei, Browser: Download).
pub(crate) fn save_document(doc: &mut Document, path: &Path) -> pdftool_core::Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        doc.save_as(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        doc.save_with(path, |p, bytes| {
            write(p, bytes).map_err(|source| pdftool_core::CoreError::Io {
                path: p.to_path_buf(),
                source,
            })
        })
    }
}

/// Führt `work` im Hintergrund aus (Desktop: eigener Thread, Browser: sofort, blockierend).
pub(crate) fn spawn(work: impl FnOnce() + Send + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::spawn(work);
    }
    #[cfg(target_arch = "wasm32")]
    {
        work();
    }
}

/// Browser: per Drag & Drop abgelegte Dateien einlesen und wie gewählte PDFs öffnen.
#[cfg(target_arch = "wasm32")]
pub(crate) fn receive_dropped(ctx: &egui::Context, files: Vec<egui::DroppedFileHandle>) {
    for file in files {
        let ctx = ctx.clone();
        let path = PathBuf::from(
            file.path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Dokument.pdf".into()),
        );
        // `file` (ein `Arc`) wandert in die Future und lebt, bis der Inhalt gelesen ist.
        wasm_bindgen_futures::spawn_local(async move {
            match file.bytes_async().await {
                Ok(bytes) => {
                    web::insert(path.clone(), bytes);
                    push_picked(Pick::OpenPdf, vec![path]);
                }
                Err(e) => tracing::error!(error = %e, "abgelegte Datei nicht lesbar"),
            }
            ctx.request_repaint();
        });
    }
}

/// Merkt sich, ob es ungespeicherte Änderungen gibt (Browser: Rückfrage beim Schließen des Tabs;
/// auf dem Desktop fragt die App selbst beim Beenden).
pub(crate) fn set_unsaved(unsaved: bool) {
    #[cfg(target_arch = "wasm32")]
    web::UNSAVED.with(|u| u.set(unsaved));
    #[cfg(not(target_arch = "wasm32"))]
    let _ = unsaved;
}

/// Browser: Ereignisse der Webseite anmelden, die egui nicht selbst liefert – Rückfrage beim
/// Schließen mit ungespeicherten Änderungen und Bilder aus der Zwischenablage (Strg+V).
#[cfg(target_arch = "wasm32")]
pub(crate) fn install_web_hooks(ctx: &egui::Context) {
    web::install(ctx.clone());
}

/// Läuft die App im Browser?
pub(crate) const fn is_web() -> bool {
    cfg!(target_arch = "wasm32")
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashMap;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    thread_local! {
        // Virtuelle Dateien dieser Sitzung (gewählt oder gespeichert), Schlüssel = Pfad/Name.
        static FILES: RefCell<HashMap<PathBuf, Vec<u8>>> = RefCell::new(HashMap::new());
        /// Gibt es ungespeicherte Änderungen? (setzt die App jeden Frame, siehe `set_unsaved`)
        pub(super) static UNSAVED: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn install(ctx: egui::Context) {
        let Some(window) = web_sys::window() else {
            return;
        };
        load_builtin_fonts(window.clone(), ctx.clone());
        // Schließen/Neuladen mit ungespeicherten Änderungen: Der Browser zeigt seine eigene
        // Rückfrage („Seite verlassen?“), sobald das Ereignis abgebrochen wird.
        let on_unload = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
            |e: web_sys::BeforeUnloadEvent| {
                if UNSAVED.with(Cell::get) {
                    e.prevent_default();
                    e.set_return_value("");
                }
            },
        );
        let _ = window
            .add_event_listener_with_callback("beforeunload", on_unload.as_ref().unchecked_ref());
        // `forget`: Die Funktion muss so lange leben wie die Seite (sonst ruft JS ins Leere).
        on_unload.forget();

        // Strg+V: egui liefert im Browser nur Text. Bilder holen wir uns selbst aus dem Ereignis.
        let on_paste = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(
            move |e: web_sys::ClipboardEvent| {
                let Some(files) = e.clipboard_data().and_then(|d| d.files()) else {
                    return;
                };
                let image = (0..files.length())
                    .filter_map(|i| files.get(i))
                    .find(|f| f.type_().starts_with("image/"));
                let Some(file) = image else {
                    return;
                };
                let ctx = ctx.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                        Ok(buffer) => {
                            let path = PathBuf::from("Eingefügtes Bild");
                            insert(path.clone(), js_sys::Uint8Array::new(&buffer).to_vec());
                            push_picked(Pick::PasteImage, vec![path]);
                        }
                        Err(e) => tracing::error!(?e, "Bild aus der Zwischenablage nicht lesbar"),
                    }
                    ctx.request_repaint();
                });
            },
        );
        if let Some(document) = window.document() {
            let _ = document
                .add_event_listener_with_callback("paste", on_paste.as_ref().unchecked_ref());
        }
        on_paste.forget();
    }

    pub(super) fn pick(dialog: FileDialog, ctx: egui::Context, pick: Pick, multiple: bool) {
        let mut d = rfd::AsyncFileDialog::new();
        for (name, exts) in &dialog.filters {
            d = d.add_filter(name, exts);
        }
        if let Some(t) = &dialog.title {
            d = d.set_title(t);
        }
        // Die Auswahl läuft als Future weiter, während die App normal Frames zeichnet.
        wasm_bindgen_futures::spawn_local(async move {
            let handles = if multiple {
                d.pick_files().await.unwrap_or_default()
            } else {
                d.pick_file().await.into_iter().collect()
            };
            let mut paths = Vec::new();
            for h in handles {
                let path = PathBuf::from(h.file_name());
                let bytes = h.read().await;
                FILES.with(|f| f.borrow_mut().insert(path.clone(), bytes));
                paths.push(path);
            }
            push_picked(pick, paths);
            ctx.request_repaint();
        });
    }

    /// Lädt die Liberation-Schriften (`fonts/<Name>.ttf` neben der Seite) im Hintergrund und
    /// meldet sie im Kern an. Bis dahin fehlen sie in der Schriftauswahl und für PDF/A.
    fn load_builtin_fonts(window: web_sys::Window, ctx: egui::Context) {
        wasm_bindgen_futures::spawn_local(async move {
            let mut loaded = 0;
            for name in pdftool_core::embedded_font::LIBERATION_NAMES {
                match fetch_bytes(&window, &format!("fonts/{name}.ttf")).await {
                    Ok(bytes) => {
                        pdftool_core::embedded_font::register_liberation(name, bytes);
                        loaded += 1;
                    }
                    Err(e) => tracing::warn!(name, error = %e, "Schrift nicht geladen"),
                }
            }
            tracing::info!(loaded, "Liberation-Schriften nachgeladen");
            ctx.request_repaint();
        });
    }

    /// Lädt eine Datei vom Webserver der Seite.
    async fn fetch_bytes(window: &web_sys::Window, url: &str) -> Result<Vec<u8>, String> {
        use wasm_bindgen_futures::JsFuture;
        let response: web_sys::Response = JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(|e| format!("{e:?}"))?
            .dyn_into()
            .map_err(|e| format!("{e:?}"))?;
        if !response.ok() {
            return Err(format!("HTTP {}", response.status()));
        }
        let buffer = JsFuture::from(response.array_buffer().map_err(|e| format!("{e:?}"))?)
            .await
            .map_err(|e| format!("{e:?}"))?;
        Ok(js_sys::Uint8Array::new(&buffer).to_vec())
    }

    pub(super) fn insert(path: PathBuf, bytes: Vec<u8>) {
        FILES.with(|f| f.borrow_mut().insert(path, bytes));
    }

    pub(super) fn read(path: &Path) -> io::Result<Vec<u8>> {
        FILES
            .with(|f| f.borrow().get(path).cloned())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "im Browser nicht mehr verfügbar – bitte die Datei erneut öffnen",
                )
            })
    }

    pub(super) fn exists(path: &Path) -> bool {
        FILES.with(|f| f.borrow().contains_key(path))
    }

    pub(super) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Dokument".into());
        download(&name, bytes).map_err(|e| io::Error::other(format!("Download: {e:?}")))?;
        FILES.with(|f| f.borrow_mut().insert(path.to_path_buf(), bytes.to_vec()));
        Ok(())
    }

    /// Bietet `bytes` als Datei-Download an (Blob-URL + unsichtbarer Link).
    fn download(file_name: &str, bytes: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
        let options = web_sys::BlobPropertyBag::new();
        options.set_type(mime_type(file_name));
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)?;
        let document = web_sys::window()
            .and_then(|w| w.document())
            .ok_or("kein Dokument")?;
        let link = document
            .create_element("a")?
            .dyn_into::<web_sys::HtmlAnchorElement>()?;
        link.set_href(&url);
        link.set_download(file_name);
        link.click();
        web_sys::Url::revoke_object_url(&url)?;
        Ok(())
    }

    fn mime_type(file_name: &str) -> &'static str {
        let ext = file_name
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "pdf" => "application/pdf",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "txt" => "text/plain",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "p12" | "pfx" => "application/x-pkcs12",
            _ => "application/octet-stream",
        }
    }
}
