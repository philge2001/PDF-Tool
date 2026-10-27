# CLAUDE.md – PDF Tool

Plattformübergreifender PDF-Editor in Rust (egui/eframe + PDFium + lopdf).
Eigenständige Neuentwicklung: keine fremden Codes, Marken, Logos, Icons oder Assets.
Der Nutzer ist Rust-Einsteiger: größere Schritte kurz erklären, nicht-offensichtlichen Code
(Ownership, Lifetimes, Traits) kommentieren, nach jedem Meilenstein eine Test-Anleitung geben.

## Build-Befehle
```bash
cargo build
cargo clippy --workspace --all-targets -- -D warnings   # muss warnungsfrei sein
cargo test --workspace                                  # braucht kein PDFium
cargo fmt --all
cargo run -p pdftool-core --example gen_fixtures      # tests/fixtures neu erzeugen
PDFTOOL_PDFIUM_PATH=/pfad/zu/pdfium/lib cargo run -p pdftool-app -- datei.pdf
```

## Workspace
| Crate | Pfad | Aufgabe | Darf abhängen von |
|---|---|---|---|
| pdftool-core | crates/core | Dokumentmodell, Annotationen, Commands/Undo, Text-Logik, Speichern (lopdf) | lopdf, thiserror, tracing, zune-jpeg, ttf-parser, subsetter, png, jpeg-encoder, zip, similar, getrandom (Zufallsschlüssel für AES-256; wasm32: Feature `wasm_js`), web-time (Uhrzeit, auch im Browser) |
| pdftool-render | crates/render | PDFium-Worker-Thread, Rendering, Textextraktion, Cache | core, pdfium-render, web-time (Zeitscheiben im Browser) |
| pdftool-sign | crates/sign | Signaturen prüfen/erstellen (CMS, X.509, PAdES) | core, RustCrypto (cms, x509-cert, der, spki, rsa, p256, p384, sha1, sha2), rand_core, p12-keystore; wasm32: getrandom 0.2 mit Feature `js` |
| pdftool-ocr | crates/ocr | Texterkennung (Tesseract), Wörter mit Pixelrechteck | tesseract-rs (baut Tesseract/Leptonica mit CMake) |
| pdftool-app | crates/app | eframe-App (Bibliothek `pdftool_app` + Programm `pdftool`), Werkzeuge, Panels, Shortcuts | core, render, sign, ocr (Feature `ocr`, Standard), eframe (Feature `links`: Weblinks im Browser öffnen), rfd, der, ureq (nicht wasm32); wasm32: wasm-bindgen, wasm-bindgen-futures, js-sys, web-sys (Dateiauswahl, Downloads); Build: winresource (nur Windows, Icon in der .exe) |

## Architekturregeln
- **core** kennt weder UI noch PDFium und ist vollständig unit-testbar.
- **PDFium läuft nur im Worker-Thread** von `render`. Die UI kommuniziert über Channels, sie blockiert nie.
- **Koordinaten im Modell** sind immer PDF-Benutzerraum (pt, Ursprung unten links, ohne /Rotate).
  Die Umrechnung zum Bildschirm passiert ausschließlich in `app::viewer::transform`.
- **Jede Modelländerung läuft über ein `Command`** (core::command). Nur so funktioniert Undo/Redo.
- **Annotationen**: Unterstützte Typen werden ins Modell importiert und von der App als Overlay gezeichnet.
  Der Render-Worker blendet sie beim Rendern aus. Beim Speichern schreibt core sie mit Appearance Stream (/AP).
- **Formularfelder**: ausfüllbare Felder (Text, Kontrollkästchen, Optionsfeld, Kombi-/Listenfeld) stehen im
  Modell (`Model::forms`); ihre Widgets blendet der Worker aus, die App zeichnet sie. Schaltflächen und
  Signaturfelder bleiben PDFium überlassen. Widgets merken sich ihre Seite als Objektnummer (nicht Index).
  JavaScript wird nicht ausgeführt, /NeedAppearances bleibt unverändert.
- **Signaturen** (Crate `sign`): Kryptografie nur dort, `core` bleibt frei davon. Geprüft werden immer die
  Dateibytes (`Document::bytes`), nie ungespeicherte Änderungen. RustCrypto bewusst im stabilen Stand
  (cms 0.2, x509-cert 0.2, der 0.7, rsa 0.9, p256/p384 0.13) – die nächste Generation ist noch RC.
- **Speichern** ist ein inkrementelles Update: Die Originalbytes bleiben unverändert, die Änderungen werden angehängt.
- Fehler: `thiserror` in Library-Crates, `anyhow` nur in der App. Logging mit `tracing`.
- **Browser (PDF Webeditor):** Die ganze App läuft auch als WebAssembly (github.com/philge2001/PDF-Webeditor
  bindet `pdftool-app` per Git ein, ohne Feature `ocr`). Regeln:
  - Kein `SystemTime::now()`/`Instant::now()` (Absturz im Browser; `clippy.toml` verbietet sie, auch
    Bibliotheksfunktionen wie `x509_cert::time::Validity::from_now`) – stattdessen `core::date::system_now()` bzw.
    `web_time::Instant`. Neue Abhängigkeiten auf versteckte `::now()`-Aufrufe prüfen (Absturz erst zur Laufzeit).
  - In der App keine Dateien/Dialoge/Threads direkt: immer über `app::platform` (`FileDialog` mit `pick(ctx, Pick, …)`
    → Ergebnis über `take_picked`/`PdfToolApp::handle_picked`; `save_file`/`pick_folder`; `read`, `write`,
    `exists`, `open_document`, `save_document`, `spawn`, `is_web`). Neue Dateiauswahl = neuer `Pick`-Fall.
  - `std::fs`/`std::env` liefern im Browser nur Fehler/`None` (kein Absturz): Pfad-Funktionen in core
    (`Document::open`/`save_as`, `system_fonts`) sind dort nicht nutzbar – `from_bytes`+`set_path`/`save_with`.
  - CI-Job `wasm`: Clippy für wasm32 über core, sign, render und app.
- Keine AGPL-Abhängigkeiten. Jede neue Crate wird im Commit bzw. hier mit einem Satz begründet.

## Konventionen
- Edition 2024, `unsafe_code = "forbid"` workspace-weit.
- Kommentare und UI-Texte auf Deutsch, Bezeichner auf Englisch.
- Tests für core: Roundtrip „erstellen → speichern → neu laden → vergleichen“ mit den Fixtures.
- Fixtures in `tests/fixtures/` werden ausschließlich per `gen_fixtures` erzeugt.

## Meilensteine Phase 10 (Neues Oberflächen-Layout)
- [x] 10.1 Eigene Icons, Farbschema, Kopfleiste, rechte Symbolleiste, schwebende Seitennavigation
- [x] 10.2 Werkzeugbereich links (Alle Werkzeuge + Werkzeug-Panels), schwebende Schnellwerkzeuge
- [x] 10.3 Startseite, Lesezeichen, Menü-Knopf
- [x] 10.4 Mehrere Dokumente als Tabs + Abschluss
- [x] 10.5 Gestaltung wie Portal und Hausbuch (Apple-Systemfarben, Pillen, Glyphen, Startseite, Werkzeug-Abschnitte)
- [x] 10.6 Darstellung „Dämmerung“ zwischen hell und dunkel (Kopfleiste schaltet Hell → Dämmerung → Dunkel)

## Installer / Releases
- `cargo-packager` (Konfiguration `[package.metadata.packager]` in `crates/app/Cargo.toml`, Pfade relativ zu
  `crates/app`): Windows NSIS (Benutzerinstallation, deutsch über `packaging/German.nsh` mit UTF-8-BOM),
  macOS DMG (nur Apple Silicon), Linux deb + AppImage (optional). Werkzeuge nur in CI/lokal installiert, keine Abhängigkeit.
- `packaging/prepare.sh <plattform>` bzw. `packaging/prepare.ps1` (Windows, PowerShell 5.1, mit UTF-8-BOM) → `packaging/dist/` (gitignored): PDFium chromium/7881 (= `pdfium_latest`
  von pdfium-render 0.9.4 – bei Update von pdfium-render mitziehen), Lizenztexte, `cargo about` →
  `Rust-Bibliotheken.html` (`packaging/about.toml` = erlaubte Lizenzen, `about.hbs` = Vorlage).
- `before-packaging-command` prüft zuerst mit `render`-Example `check_dist`, ob PDFium in `packaging/dist/lib` liegt
  (cargo-packager würde sonst stillschweigend ohne PDFium paketieren).
- PDFium-Fundorte im Paket: neben der exe, `../Resources` (macOS), `../lib/pdftool` (deb/AppImage) – `render::loader`.
- Workflow `.github/workflows/release.yml`: Tag `v*` = bauen + GitHub-Release (Version muss zu Cargo.toml passen),
  PR mit Paketier-Änderungen/„Run workflow“ = nur Artifacts.
- App-Icon: `assets/icons/pdftool.svg` (eigene Zeichnung) → PNG/ICO (gerendert mit Chromium headless + ImageMagick);
  Fenster-Icon per `include_bytes!` in `main.rs`, exe-Icon über `crates/app/build.rs`. Release-Builds unter
  Windows ohne Konsole (`windows_subsystem`).
- Nicht signiert (SmartScreen-/Gatekeeper-Warnung); jeder Doppelklick auf eine PDF startet ein eigenes Fenster.

## Meilensteine Phase 11 (Textformat, Systemschriften, Bilder)
- [x] 11.1 Fehlende Zeichen aus Systemschriften (Suche, Lizenzprüfung, Teilmenge)
- [x] 11.2 Textformat bei Auswahl (Schrift, Größe, Fett, Kursiv, Unter-/Durchgestrichen, Farbe)
- [x] 11.3 Bilder mit dem Auswahlwerkzeug, Zwischenablage, Bilder in Formular-XObjects + Abschluss

## Meilensteine Phase 9 (Sicherheit & Barrierefreiheit)
- [x] 9.1 Passwortgeschützte PDFs öffnen
- [x] 9.2 Schutz setzen/entfernen (AES-256)
- [x] 9.3 Tagged PDF (Strukturbaum, Titel/Sprache, Alternativtexte, Panel „Tags“)
- [x] 9.4 Barrierefreiheit prüfen, automatisch taggen + Abschluss

## Meilensteine Phase 8 (Export, Vergleich, Komprimierung, PDF/A)
- [x] 8.1 Export Bilder (PNG/JPEG) & Text
- [x] 8.2 Einfacher DOCX-Export
- [x] 8.3 Dokumentvergleich
- [x] 8.4 Komprimieren
- [x] 8.5 PDF/A-2b (bestmöglich) + Abschluss

## Meilensteine Phase 7 (Texterkennung)
- [x] 7.1 Tesseract einbinden (ocr-Crate, Sprachdaten, Offscreen-Rendern)
- [x] 7.2 Unsichtbare Textebene (core)
- [x] 7.3 OCR-Oberfläche + Abschluss

## Meilensteine Phase 6 (Text und Bilder bearbeiten)
- [x] 6.1 Schriften einbetten (Noto Sans als Teilmenge, Unicode-Freitext)
- [x] 6.2 Text bearbeiten (Textblöcke, Originalschrift, Ersatzschrift, neuer Text)
- [x] 6.3 Bilder bearbeiten + Abschluss

## Meilensteine Phase 5 (Schwärzen & Bereinigen)
- [x] 5.1 Content-Stream-Interpreter (core)
- [x] 5.2 Schwärzungen markieren (Redact-Annotationen, UI)
- [x] 5.3 Schwärzungen anwenden + komplett neu schreiben
- [x] 5.4 Bereinigen + Abschluss

## Meilensteine Phase 4 (Signaturen)
- [x] 4.1 Signaturen prüfen (sign-Crate, Panel, Vertrauensspeicher)
- [x] 4.2 Signieren PAdES B-B (PKCS#12, Platzhalter, CMS)
- [x] 4.3 Zeitstempel (PAdES B-T) + Abschluss

## Meilensteine Phase 3 (Formulare)
- [x] 3.1 Formularmodell, Werte-Commands, Speichern mit AP (core)
- [x] 3.2 Formulare ausfüllen (UI)
- [x] 3.3 Formulare gestalten (Felder erstellen/bearbeiten)
- [x] 3.4 AcroForm bei Einfügen/Extrahieren übernehmen + Abschluss

## Meilensteine Phase 2 (Seitenorganisation)
- [x] 2.1 Seitenmodell & Seiten-Commands (core)
- [x] 2.2 Seiten-UI (Mehrfachauswahl, Kontextmenü, Drag & Drop)
- [x] 2.3 Einfügen, Zusammenführen, Leerseite
- [x] 2.4 Extrahieren & Teilen, Hintergrund-Jobs

## Meilensteine Phase 1
- [x] M0 Grundgerüst
- [x] M1 Öffnen & Rendern
- [x] M2 Zoom, Navigation, Thumbnails, Cache
- [x] M3 Textsuche
- [x] M4 Text auswählen & kopieren
- [x] M5 Annotationsmodell, Commands, Speichern (core)
- [x] M6 Annotationswerkzeuge (UI)
- [x] M7 Bearbeiten, Kommentar-Panel, Undo/Redo-Shortcuts
- [x] M8 Speichern in der App
- [x] M9 Zuletzt geöffnet, Dark/Light Mode

## Vorbereitung späterer Phasen
- Phase 2 (Seiten): neue Commands auf `core::command::Model` (z. B. Seitenreihenfolge im Modell),
  Speichern über `IncrementalDocument` wie in `core::save`.
- Phase 4 (Signaturen): inkrementelles Speichern erhält bestehende Signaturen bereits.
- Phase 5/6 (Schwärzen, Text bearbeiten): Content-Stream-Änderungen in core, PDFium nur zum Rendern.

## Bekannte Einschränkungen
- Overlay vs. Datei: Hervorhebungen im Overlay halbtransparent (egui kann kein Multiply),
  Freitext im Overlay mit derselben Noto Sans wie in der Datei, auf gedrehten Seiten aber nicht mitgedreht.
- Importierte Annotationen mit exotischer Darstellung (z. B. Wolkenrand) werden vereinfacht neu gezeichnet.
- Gelöschte Seiten sind nach inkrementellem Speichern nicht mehr erreichbar, ihre Bytes stehen aber
  noch in der Datei – „Schwärzen → Dokument bereinigen“ schreibt die Datei komplett neu.
- Dokumente werden im UI-Thread geladen (lopdf); sehr große Dateien (> 100 MB) blockieren kurz.
- Verschlüsselung: Gespeichert wird immer komplett neu (verschlüsselt), nicht inkrementell – Signaturen werden
  dabei ungültig (Warnung im Dialog). Neuer Schutz nur als AES-256; Zertifikats-Verschlüsselung (/PubSec) fehlt.
  Ohne Besitzerpasswort bleiben Berechtigungen beim Speichern unverändert.
- Barrierefreiheit: Die Prüfung ist eine Auswahl wichtiger PDF/UA-Punkte (keine vollständige Validierung, z. B.
  keine Kontrastprüfung, keine Prüfung der Überschriften-Hierarchie). Automatisches Taggen ist heuristisch:
  Lesereihenfolge = Reihenfolge im Content-Stream, Überschriften nur über die Schriftgröße, keine Listen/Tabellen,
  Text in Formular-XObjects wird als Ganzes ein Absatz; nur ungetaggte Dokumente. Nach dem Taggen in der App
  neu angelegte Annotationen bekommen keinen Tag. Geänderte Titel aktualisieren /Info, nicht das XMP (dc:title).
  Strukturelemente ändern/verschieben ist nicht möglich (nur Alternativtext, Titel, Sprache).
- Annotationen auf gedrehten Seiten: Geometrie korrekt, Freitext-Overlay nicht mitgedreht.
- Oberfläche: Tabs lassen sich nicht per Ziehen umsortieren; Werkzeug-/Bearbeitungsmodus gilt pro Fenster
  (beim Tab-Wechsel zurück auf Textauswahl); Lesezeichen nur lesen (kein Anlegen/Umbenennen); Icons sind
  selbst gezeichnete Vektorgrafiken (kein Icon-Font); Drucken fehlt.
- Absätze bearbeiten: Formatierung innerhalb des Absatzes (z. B. ein fettes Wort) wird auf den Stil der ersten
  Zeile vereinheitlicht; nur links-/Blocksatz (zentriert/rechtsbündig wird linksbündig neu gesetzt); Kerning geht verloren;
  aus eingebetteten Teilmengen nur Zeichen, die auf der Seite schon vorkommen (Rest in Noto Sans, kein CJK);
  der Seiteninhalt wird neu kodiert (ein Stream); eingefügte, ungespeicherte Seiten erst nach dem Speichern.
- Export DOCX: kein Layout (Spalten, Tabellen, Positionen), Text in Formular-XObjects fehlt, CMYK/Indexed-Bilder fehlen.
- Vergleich: nur Text (kein Pixelvergleich), Formatierung wird nicht verglichen.
- Komprimieren: nur Grau/RGB-Bilder mit 8 Bit ohne /Mask und /Decode; Inline-Bilder bleiben.
- PDF/A: nur 2b; nicht eingebettete CID- und Symbol-Schriften, DeviceCMYK und unbekannte Annotationen ohne
  /AP werden gemeldet statt behoben. Ersatzschriften werden voll eingebettet (≈ 0,4 MB je Schnitt).
  Unbekannte nicht eingebettete Schriften werden durch Liberation Sans genähert (Breiten können abweichen).
- Texterkennung: nur waagerechter Text in der Anzeige (keine automatische Drehung schiefer Scans), Wörter unter
  30 % Sicherheit werden verworfen; CJK ist nicht eingebaut (Noto Sans ohne CJK). Der Tesseract-Build lädt beim ersten
  Mal Quellen von github.com (im Container gesperrt: Quellen per `git clone` nach `~/.tesseract-rs/third_party/`,
  `eng`/`tur` aus tessdata_best per raw.githubusercontent.com nach `~/.tesseract-rs/tessdata/`).
- Bilder bearbeiten: nur Bilder direkt im Seiteninhalt (nicht in Formular-XObjects); Zeichenreihenfolge bleibt;
  PNG-Farbprofile/Gamma werden ignoriert.
- Formularfelder im Overlay mit egui-Schrift; eigene Erscheinungsbilder von Kontrollkästchen/Optionsfeldern
  (z. B. Kreuz statt Häkchen) zeigt PDF Tool vereinfacht – in der Datei bleiben sie erhalten.
  Felder auf gedrehten Seiten: Text nicht mitgedreht. Formular-JavaScript (Berechnungen, Formate) wird nicht ausgeführt.
- Schwärzen: Pfade, die einen Bereich kreuzen, verschwinden ganz (kein Zuschneiden); Bilder in JPEG 2000, JBIG2,
  CCITT werden ganz entfernt; Ersatztexte nur in direkten BDC-Dictionaries (nicht über /Properties-Ressourcen).
- Signaturen: keine Sperrprüfung (OCSP/CRL) und kein LTV (B-LT/B-LTA), RSA-PSS wird nicht geprüft,
  DocMDP-Berechtigungen werden angezeigt, aber Änderungen danach nicht darauf geprüft. Digitale IDs nur als
  PKCS#12-Datei (kein Windows-Zertifikatspeicher, keine Smartcards). Vertrauen nur über selbst
  hinterlegte Zertifikate (keine eingebaute Liste von Stammzertifikaten).
- Browser-OCR: tesseract.js (im PDF Webeditor eingebunden), Abbrechen erst zwischen zwei Seiten; das Rendern einer
  Seite (300 dpi) läuft kurz im UI-Thread.
- Browser: keine Systemschriften – zur Auswahl stehen Noto Sans und Liberation Sans/Serif/Mono; Liberation wird
  nach dem Start nachgeladen (bis dahin fehlt sie in der Auswahl und für PDF/A/Taggen).
- Systemschriften: nur TrueType mit glyf-Tabelle (keine CFF/OpenType-PS-Schriften), nur wenn fsType Einbetten und
  Teilmengen erlaubt. Fehlender Fett-/Kursivschnitt wird nachgeahmt (Tr 2 mit Kontur, Schrägstellung 0,21).
  Die Editor-Vorschau zeigt Schriftarten nicht originalgetreu (egui-Schrift, nur fett/kursiv/Linien angedeutet).
  Formatierter Text wird stückweise mit absoluten Positionen (Tm) geschrieben; Unter-/Durchstreichung als Rechtecke.
- Bilder: Formular-XObjects zählen nur als Bild, wenn sie Bilder und keinen Text enthalten (dann wird das ganze
  XObject verschoben). Kopieren/Speichern rendert das Bild über PDFium (Pixelauflösung, max. 25 MP).
  Eingefügte Bilder aus der Zwischenablage werden als PNG eingebettet (96 dpi). Die Hand bleibt reines Verschieben der Ansicht.
- Freitext nutzt die eingebettete Noto Sans (Latein, Kyrillisch, Griechisch); CJK fehlt der Schrift.
  Formularfelder und sichtbare Signaturen nutzen weiterhin Helvetica (WinAnsi).

## Wichtige Module
- `core::document` – `Document::open`, `from_bytes`+`set_path`, `save_as`/`save_with` (eigene Schreibfunktion), `PageInfo` (CropBox, /Rotate, Umrechnung PDF ↔ Anzeige)
- `render::worker` – `RenderService` (Thread + Channels), `RenderKey` (Seite, Skalierungsstufe, Kachel),
  `page_pixel_size`/`tile_grid`/`tile_rect` (gemeinsame Rechenregeln für App und Worker)
- `render::worker::Worker` – Zustand des Workers (Dokument, Warteschlangen, Offscreen-Dokument), `step` = eine
  Arbeitseinheit. Desktop: Thread-Schleife `run`. Browser (`worker::web`): kein Thread, Worker in einem
  `thread_local`, Aufträge sammeln, `poll` arbeitet pro Frame höchstens `FRAME_BUDGET_MS` ab; `OffscreenRenderer`
  rendert dort synchron
- `render::cache::ByteLru` – LRU mit Byte-Budget
- `app::viewer` – `Layout` (Präfixsummen, Binärsuche), `PageTextures` (GPU-Texturen, pending/wanted),
  `Viewer::show` (zeichnet nur sichtbare Seiten, Kacheln ab 3072 px), Zoom mit Anker (`set_zoom`),
  `ZoomMode` (Custom/FitWidth/FitPage), `FrameRequests` (Render-Aufträge pro Frame, `flush_requests`)
- `app::viewer::thumbnails` – Seitenminiaturen (Panel rechts), gleiche Texturen, Priorität `Thumbnail`;
  `FIT_MARGIN` hält bei Seitenbreite/Ganze Seite seitlich Platz für die schwebenden Leisten
- `core::annotation` – `Annotation`/`AnnotKind` (9 Typen), `AnnotationStore`; `import` (PDF -> Modell,
  Unverstandenes bleibt fremd), `export` (Dictionary, Basis = Original-Dict), `appearance` (/AP-Streams)
- `core::command` – `Command`-Trait, `Model` (Annotationen + Seitenliste), `History` (Undo/Redo, dirty)
- `core::page_ops` – `RotatePages`, `DeletePages`, `ReorderPages` (rechnen Annotations-Seitenindizes mit um);
  `PageInfo::id` (`PageId`) ist stabil über Umsortieren hinweg
- `core::page_import` – `Document::prepare_insert` (Tiefenkopie von Seiten aus einer anderen PDF mit
  Umnummerierung; /Parent und fremde Seiten werden nicht verfolgt; verwaltete Annotationen werden
  ins Modell übernommen), `prepare_blank_page`, `InsertPages`-Command, `parse_page_ranges("1-3, 5, 8-")`.
  Neue Objekte liegen bis zum Speichern in `Model::extra_objects`; Objektnummern vergibt ein
  nur hochzählender Zähler (`next_object_id`)
- `core::page_export` – `extract_pages` (neue, eigenständige PDF nur mit den gewünschten Seiten, aus
  `Document::snapshot_pdf` = aktueller Stand inkl. ungespeicherter Änderungen), `split_every`, `split_by_ranges`
- `Document::render_snapshot` – Bytes des aktuellen Stands + Ausblendliste für den Renderer
  (nach strukturellen Änderungen, ohne zu speichern)
- `core::save` – inkrementelles Speichern: Seitenbaum (bei neuer Reihenfolge flach unter der Wurzel,
  geerbte MediaBox/CropBox/Resources werden in die Seite übernommen), pro Seite fremde Einträge +
  verwaltete Annotationen; unveränderte behalten ihren Verweis, geänderte werden unter derselben
  Objektnummer neu geschrieben
- `core::content` – Content-Stream-Interpreter: `interpret_page`/`interpret_form` → `Interpreted { ops, items }`;
  `Item::{Text(TextShow), Path, Image, Form, Shading}` mit Rechtecken im PDF-Raum und Op-Indizes;
  `Glyph { at: GlyphRef{op, part, start, len}, rect, advance }`; `Matrix` (PDF-Reihenfolge, `then`);
  `FontInfo` (/Widths, CID /W+/DW, Type3 /FontMatrix, Helvetica/Courier ohne /Widths; sonst `exact = false`
  und `TextShow::bounds` als großzügige Obergrenze). Ops kommen von `lopdf::content::Content::decode`
  (inkl. Inline-Bildern als `BI` mit Stream-Operand)
- `core::embedded_font` – Noto Sans (`assets/fonts`, OFL, per `include_bytes!` eingebaut): Breiten, Umbruch,
  `SubsetFont` (Text -> 2-Byte-Glyphencodes, `write` = Type0/CIDFontType2 + /W + ToUnicode als Teilmenge).
  Die App registriert dieselbe Schrift als egui-Familie `app::NOTO_FAMILY` (Freitext-Overlay)
- `core::text_edit` – `PageContent` (bearbeiteter Inhalt einer Seite in `Model::page_content`, mit
  Noto-Teilmengen als `ExtraFonts` und `stamp` aus `Model::content_epoch`), `ReplacePageContent`-Command,
  `Document::text_blocks`/`edit_text_block`/`add_text`/`content_stamp`. Blöcke = gleiche Schrift/Größe/Grundlinie
  (Ersatzschrift-Läufe gehören dazu); beim Bearbeiten bleibt der Vorschub erhalten (Ausgleichs-TJ),
  Originalschrift nur mit sicher enthaltenen Codes. `save::write_page_contents` schreibt Stream + Ressourcen
- `core::image_edit` – `PageImage` (Do/Inline-Bild, Matrix, Bounds, Stempel), `Document::page_images`/
  `transform_image` (q cm … Q; vorhandene Klammer wird angepasst)/`delete_image`/`replace_image` (Seitenverhältnis,
  zentriert)/`insert_image`; `NewImage::from_bytes` (PNG → Flate + /SMask, JPEG → DCTDecode). Neue Bilder liegen
  bis zum Speichern in `PageContent::images` (Interpreter: `ExtraImages`), danach nur noch in der Datei
- `app::image_tool` – Werkzeug „Bilder“: auswählen, ziehen, Anfasser (gleichmäßig skalieren), Ersetzen/Löschen/
  Einfügen (Klick platziert); Auswahl wird nach Änderungen über die Bildmitte wiedergefunden
- `core::paragraph` – `group_paragraphs` (Zeilen = Ausgaben auf einer Grundlinie, egal welche Schrift; Absatz =
  gleiche Richtung/Größe, gleicher linker Rand (erste Zeile darf eingerückt sein), gleichmäßiger Abstand; Blocksatz
  erkannt), `Document::paragraphs`/`set_paragraph(…, ParaFrame { origin, width, justify })`: alte Ausgaben →
  Verschiebungen, Absatz neu hinter dem Inhalt in `/FcPara BMC … EMC` (wird beim nächsten Mal ersetzt), Stil der
  ersten Zeile (Schrift, Größe, Füllfarbe aus `TextShow::fill`, Tc/Tw/Tz/Ts/Tr, Matrix), Umbruch auf die Kastenbreite; harte
  Umbrüche (`\n`) als `/FcBr MP` hinter der Zeile, beim Erkennen `Paragraph::breaks` (ohne Markierung: erstes Wort der
  nächsten Zeile hätte noch gepasst), `rows()` = Zeilen inkl. Leerzeilen
- `core::image_codec` – `encode_png`/`encode_jpeg` (Gray/Rgb/Rgba, JPEG: Alpha auf Weiß)
- `core::docx` – `export_docx(doc, seiten, progress)`: Absätze (`core::paragraph`, Größe, Fett aus /BaseFont) und
  Bilder (JPEG direkt, Grau/RGB-8-Bit als PNG; seitenfüllendes Bild nur ohne Text) nach Höhe sortiert,
  Seitenumbrüche; ZIP mit minimalem WordprocessingML (`zip`, nur Deflate)
- `core::compare` – `words(seite, PageText)`, `compare(alt, neu)` (similar, Patience, dokumentweit) → `Change`
  (Deleted/Inserted/Replaced, Wörter mit Rechteck und Seite)
- `core::compress` – `compress_pdf(bytes, CompressOptions { image_dpi, jpeg_quality }, progress)`: größte Darstellung je
  Bild (Seiten + Formular-XObjects rekursiv), Grau/RGB-8-Bit-Bilder ohne /Mask//Decode verkleinern (Flächenmittel) →
  JPEG, /SMask mit; prune, compress, `SaveOptions` mit Objekt-Streams. `CompressReport::summary`, `human`
- `core::pdfa` – `convert_to_pdfa(bytes)` → (Bytes, `PdfaReport { fixed, problems }`): sanitize (JS, eingebettete
  Dateien), über `Document` fehlende /AP erzeugen (Annotationen, Formularfelder), /Btn-/N als Zustands-Dictionary,
  leere /AP als Notlösung, geerbte /Resources in die Seite, direkte Schriften zu Objekten, nicht eingebettete
  Standardschriften → Liberation (TrueType, WinAnsi, Breiten aus der Schrift, voll eingebettet), sRGB-OutputIntent,
  XMP (pdfaid 2/B, abgeglichen mit /Info, unkomprimiert), LZW/TR/Interpolate/AA/NeedAppearances weg, /F druckbar,
  /ID, Version 1.7. Assets: `assets/fonts/liberation` (OFL), `assets/icc/sRGB-v2-magic.icc` (CC0)
- `render::worker::OffscreenRenderer` – zusätzlich `page_text` (Lesereihenfolge) und `page_chars` (`PageText`)
- `app::export` – Datei → Exportieren… (PNG/JPEG je Seite mit dpi/Qualität, TXT mit Seitenvorschub, DOCX);
  testbar über `export_images`/`export_text` (mit PDFium), `page_file_name`
- `app::compare` – Datei → Vergleichen mit… bzw. `pdftool alt.pdf --vergleichen neu.pdf`; Job (Wörter beider
  Dokumente offscreen) → `CompareView` (Liste + Seiten nebeneinander, Vorschaubilder per Hilfsthread)
- `app::compress_ui` – Datei → Komprimieren… (dpi, Qualität) und Datei → Nach PDF/A umwandeln; Ergebnis per
  `pending_doc` als ungespeicherte Fassung, Bericht als Meldung
- `ocr` (Crate) – `Engine::new("deu+eng")`/`recognize(&Image)` → `Word` (Pixel, oben links, Konfidenz) aus
  Tesseracts TSV; `LANGUAGES` (eingebettete tessdata_fast, beim ersten Gebrauch in `tessdata_dir()` im Temp-Ordner)
- `render::worker::OffscreenRenderer` – `RenderService::offscreen()`, klonbar/`Send`: Jobs rendern Seiten
  beliebiger Bytes im Render-Thread (eigene Warteschlange nach sichtbaren Seiten und Text, blockierende Antwort).
  pdfium-render ignoriert `set_fixed_size` ohne Formulardaten – deshalb `render_form_data(true)`
- `core::ocr_layer` – `OcrWord` (Anzeige-Einheiten 0..1, oben links), `Document::add_ocr_layers` (3 Tr, Noto, Tz =
  erkannte Breite, Größe = Worthöhe, gedreht wie die Seite, `/FcOcr BMC … EMC` wird ersetzt, `Batch` = 1 Undo),
  `page_has_text`
- `app::ocr_ui` (`cfg(has_ocr)` = Feature `ocr` oder wasm32, gesetzt in `build.rs`) – Dialog „Texterkennung (OCR)“
  (Seiten, nur ohne Text, Sprachen), Job mit Fortschritt/Abbrechen (Momentaufnahme + Offscreen 300 dpi + Tesseract),
  Ergebnis über `OcrSlot` → `take_pending_ocr`. Desktop: `pdftool-ocr` im Job-Thread (`run_tesseract`). Browser
  (`ocr_ui::web`): `Job::external` + Future, ruft `window.pdftoolOcr(rgba, w, h, "deu+eng")` (tesseract.js im
  PDF Webeditor, liefert Tesseracts TSV → `parse_tsv`); `available()` prüft, ob die Seite die Funktion hat
- `app::content_edit` – Modus „Inhalt bearbeiten“ (`Tool::EditText`/`AddText`): Absätze pro Seite gecacht (nach
  Stempel), Klick = Auswahl, Ziehen = verschieben, Anfasser rechts = Breite, Doppelklick/Enter = mehrzeiliger
  Editor (Strg+Enter übernimmt), Blocksatz-Schalter; `sync_structure` erkennt Inhaltsänderungen am Stempel
- `core::font` (Helvetica-Breiten, WinAnsi, Umbruch), `core::pdf_string` (Textstrings), `core::date`
  (`system_now` = browserfeste Systemzeit als `std::time::SystemTime`, `pdf_date_now`)
- `core::text` – `PageText`/`CharBox` (Zeichen + Rechteck im PDF-Raum), `find`, `range_rects`, `char_at`
- `render::worker` – Textextraktion (`extract_text`, Event `Text`), eigene Warteschlange:
  sichtbare Seiten > Text > Prefetch > Miniaturen
- `app::viewer::transform` – EINZIGE Umrechnung PDF ↔ Bildschirm (inkl. /Rotate)
- `app::text_cache` – Seitentext pro Dokument (lazy angefordert)
- `app::search` – Suchleiste, Treffer seitenweise sortiert, Overlay-Hervorhebung
- `app::tools` – `Tool`-Enum (aktives Werkzeug); `tools::text_select` – `TextSelection`
  (Anker/Fokus als `TextPos`, seitenübergreifend), Kopieren, Doppelklick = Wort
- `app::annot_draw` – zeichnet `Annotation` als egui-Overlay (auch Vorschau & Miniaturen)
- `app::tools::create` – Gesten (Freihand/Formen/Pfeil), Markup aus Textauswahl (`Batch` = 1 Undo-Schritt)
- `app::tools::editor` – Eingabefenster für Notiz-/Freitext (neu oder bestehend)
- Render-Worker blendet per `HiddenAnnots` genau die von core verwalteten /Annots-Indizes aus
  (`Document::managed_annot_indices`); die App zeichnet sie selbst
- `app::tools::select` – Annotationen anklicken/verschieben (Vorschau via `AnnotDrag`), löschen,
  `update_selected` (zusammenführbare Updates = 1 Undo-Schritt, siehe `History::execute_merging`)
- `app::comments` – Kommentar-Panel (nach Seite und Position sortiert, direkt bearbeitbar)
- `app::file_ops` – Speichern/Speichern unter, Rückfrage bei ungespeicherten Änderungen
  (`PendingAction`), Fenstertitel mit `*`; nach dem Speichern lädt der Worker neu (`reload_renderer`)
- `app::menu` – Inhalt des Knopfs „Menü“ (`main_menu`: Datei-Befehle direkt, Untermenüs Bearbeiten, Seiten,
  Formular, Schwärzen, Signieren, Barrierefreiheit, Ansicht; „Beenden“ zuletzt)
- `app::icons` – eigene Vektor-Icons (`Icon`, 24er-Raster, `paint`), `glyph` (farbiges Quadrat mit weißem
  Symbol), Widgets `button`/`sized_button` (rund), `text_button` (Pille, ausgewählt = getönt),
  `primary_button` (Akzent), `row_button` (Listenzeile mit Glyph, ausgewählt = Akzentfläche),
  `tab` (Pille, `TabResponse` mit Schließen-Kreuz)
- `app::style` – Farbschema hell/dunkel wie Portal (github.com/philge2001/portal) und Hausbuch (vermietung):
  `Palette` (Apple-Systemfarben, Namen wie die CSS-Variablen des Portals), `palette(dark)`, `install` über
  `style_mut_of`, `accent`/`accent_soft`, `background` (Startseite), `canvas` (hinter den Seiten), `surface`,
  `floating_fill`, `border`, `soft_shadow`, `icon_tint` (Kategorie-Farben = Icon-Farben des Portals).
  Neue Farben nur hier, nicht im Code verteilt. `Appearance` (Wie System, Hell, Dämmerung, Dunkel): Dämmerung =
  dunkles egui-Design mit der Palette `DUSK` (globaler Schalter, `apply`), `visible`; gespeichert als
  `Settings::theme` + `dusk` (`Settings::appearance`/`set_appearance`). `PdfToolApp::appearance`/`set_appearance`
  (öffentlich) nutzt der PDF Webeditor für das mit Portal und Hausbuch geteilte Cookie `pz_theme`
- `app::shell` – Fensterrahmen: `global_bar` (Menü, Tabs Startseite/Dokumente, Design/Einstellungen/Hilfe),
  `document_bar` (Alle Werkzeuge, Bearbeiten, Konvertieren, Signieren | Schloss, Rückgängig, Wiederholen,
  Suchen, Speichern, ⋯), `document_layout` (Reihenfolge der Panels), `right_rail` + `RightPane`
  (Kommentare, Seitenminiaturen, Lesezeichen, Signaturen, Tags; erneuter Klick schließt), `pane_header`,
  `page_controls` (schwebend unten rechts: Seite, ↑↓, Zoom, Anpassen), `floating_frame`, `shortcuts_window`
- `app::tools_panel` – Werkzeugbereich links: `ToolGroup` (14 Gruppen in `SECTIONS`-Abschnitten wie die Seitenleiste im Hausbuch; mit Panel: Kommentieren, Bearbeiten,
  Organisieren, Schwärzen, Formular, Zertifikat, Barrierefreiheit – sonst Dialog direkt), `open_tool_group`,
  `leave_tool_group`, `sync_tool_group` (Gruppe folgt dem Werkzeug), `tool_icon`; `quick_tools` (schwebend
  links), `selection_bar` (Farbe/Löschen der ausgewählten Annotation)
- `app::home` – Startseite (mittige Spalte ≤ 1080 px wie im Portal, Begrüßung, „Datei öffnen“): Kacheln „Empfohlene Werkzeuge“ (ohne Dokument erst Datei wählen, `pending_group`),
  „Zuletzt verwendet“ (Ort, `relative_age`, `human_size`)
- `app::bookmarks` – Panel „Lesezeichen“ (Baum mit Aufklapp-Zustand, `go_to_destination` →
  `Viewer::scroll_to_page_at`, Weblinks nach Rückfrage per `ctx.open_url`)
- `app::tabs` – mehrere Dokumente: aktives in `document`, übrige in `others`, `active_pos`; `switch_tab`
  (Worker bekommt das Dokument unter seiner alten `DocId` → Seitenbilder bleiben), `open_in_new_tab`,
  `focus_open_file`, `close_active_tab`, `first_dirty_tab` (Beenden fragt nacheinander je geändertem Tab).
  `pdftool a.pdf b.pdf` öffnet mehrere Tabs
- `core::outline` – `Document::outline` → `OutlineItem` (Titel, `Destination { page, top, left }`, URI, offen,
  Farbe, fett/kursiv, Kinder; /Dest, /GoTo, benannte Ziele über /Dests und den Namensbaum), `page_mode`
- `app::settings` – dauerhafte Einstellungen (serde + eframe-Persistenz, Schlüssel `pdftool_settings`, alter Schlüssel `foliocraft_settings` und alter Ordner werden übernommen):
  zuletzt geöffnet, Design, Panels, Werkzeugstil, Autor
- `app::pages` – `PageAction`, `sync_structure` (erkennt jede Strukturänderung inkl. Undo/Redo, biegt
  Texturen/Text per `PageId` auf neue Indizes um, lädt den Worker mit `render_snapshot` und NEUER DocId),
  `new_doc_id`, `target_pages` (Auswahl in der Seitenleiste oder aktuelle Seite)
- `app::viewer::thumbnails` – zusätzlich Mehrfachauswahl (als `PageId`), Kontextmenü, Drag & Drop (`ThumbAction`)
- `app::page_dialogs` – Dialoge „Seiten aus Datei einfügen“, „Dateien zusammenführen“, „Seiten extrahieren“,
  „Dokument teilen“
  (Ergebnis = neues unbenanntes Dokument, `adopt_document` zeigt es über eine Momentaufnahme an)
- `app::platform` – Desktop/Browser-Unterschiede: `FileDialog` (wie rfd; Browser: Auswahl asynchron, Speichern =
  Download ohne Dialog), `Pick` + `take_picked` (gewählte Dateien, verarbeitet in `PdfToolApp::handle_picked`),
  `read`/`write`/`exists` (Browser: virtuelle Dateien der Sitzung, Schreiben = Download), `open_document`,
  `save_document` (`Document::save_with`), `spawn` (Browser: sofort), `receive_dropped` (Drag & Drop im Browser),
  `install_web_hooks` (Browser: Rückfrage beim Schließen über `set_unsaved`, Bilder per Strg+V als `Pick::PasteImage`,
  Liberation-Schriften im Hintergrund nachladen)
- `app::jobs` – Hintergrund-Jobs (`Job::spawn`, Fortschritt, Abbrechen, modales Fortschrittsfenster; `Job::external`
  + `JobHandle` für Arbeit, die der Aufrufer selbst antreibt, z. B. eine Future im Browser);
  höchstens ein Job gleichzeitig (`PdfToolApp::job`), Ergebnis als Info- oder Fehlerdialog
- `core::form` – `FormField`/`FieldKind`/`FieldValue`/`Widget`, `FormStore` (Dateireihenfolge = Tab-Reihenfolge);
  `import` (Feldbaum mit vererbten /FT /Ff /V /DA …, voller Name), `appearance` (`layout` = gemeinsame
  Textanordnung für Overlay und Datei, `build` = /AP-Streams, Helvetica WinAnsi)
- `core::command::{AddField, RemoveField, UpdateField}` – Felder anlegen/löschen/ändern (`UpdateField::value`
  nur Wert, `mergeable` fürs Tippen); `FormField::new` mit Standardwerten, `FormStore::unique_name`
- `core::form_save` – neue Felder (Feld+Widget in einem Objekt, Optionsfelder als Feld mit /Kids) samt
  /AcroForm (/Fields, /DR /Helv), geänderte nur mit geänderten Einträgen (Original-/DA bleibt), gelöschte aus
  /Fields bzw. /Kids und /Annots; Schalter: vorhandene Zustandsbilder bleiben, solange das Aussehen gleich ist.
  `FormChanges` liefert `save` neue/entfernte Widgets pro Seite; `after_save` übernimmt neue Objektnummern
- `app::forms` – Ausfüllen (`FormUi`: Fokus als `FieldFocus`, Aufklappliste): `handle_form_input` (Klick/Cursor,
  hat Vorrang vor Textauswahl/Auswählen), `handle_form_keys` (Tab, Leertaste), `show_field_editor`
  (egui-TextEdit über dem Feld), `reset_form`; erste Änderung nach Fokuswechsel = neuer Undo-Schritt
- `core::page_import` – übernimmt ausfüllbare Felder der eingefügten Seiten ins Modell (`InsertPages::fields`,
  Widgets nicht als Objekt kopiert, Namen ohne Punkt und eindeutig: `free_field_name`)
- `core::page_export::copy_acroform` – /AcroForm für extrahierte Seiten, Feldbäume auf mitkopierte Widgets
  beschnitten (`prune_field`), danach `prune_objects`
- `app::form_edit` – Modus „Formular bearbeiten“ (`Tool::FORM`): Felder aufziehen/klicken, auswählen, verschieben,
  Größe am Anfasser ändern (Vorschau, 1 Undo-Schritt pro Zug), Entf löscht
- `app::form_props` – Eigenschaften-Panel (ersetzt im Bearbeiten-Modus die Kommentare); Name/Optionsliste
  werden erst übernommen, wenn gültig (`PropInputs`)
- `app::form_draw` – zeichnet Widgets (Layout aus `core::form::appearance::layout`), Hervorhebung, Pflichtrahmen
- `sign::crypto` – OIDs, `Hash` (SHA-1/256/384/512), `verify`/`verify_prehashed` (RSA PKCS#1 v1.5, ECDSA P-256/P-384)
- `sign::cert` – `CertInfo`, `TrustStore` (PEM aus den Einstellungen), `build_chain` → `Trust` (Trusted/Unknown/Invalid)
- `sign::verify` – `verify_document` → `SignatureReport` (ByteRange-Prüfung, CMS, messageDigest, Signatur,
  Kette, spätere Revisionen, DocMDP); `Status` für die Ampel
- `core::signature_prep` – PDF-Teil des Signierens ohne Kryptografie: `prepare_signature` (Revision mit
  Feld/Widget/Darstellung, /SigFlags 3, Platzhalter für /Contents und /ByteRange – ByteRange wird sofort
  eingetragen, Länge bleibt gleich), `PreparedSignature::{signed_parts, finish}`, `empty_signature_fields`
- `sign::signer` – `Signer` (Schlüssel + Zertifikat + Kette): `from_pkcs12`, `to_pkcs12`, `self_signed`
  (Profil Leaf mit sich selbst als Aussteller, EKU E-Mail + Dokumentsignatur), `PrivateKey::sign_prehashed`
- `sign::create` – `build_cms` (PAdES B-B: contentType, messageDigest, signing-certificate-v2; keine
  signingTime), `sign_pdf` (SubFilter ETSI.CAdES.detached, 24 KB Platz)
- `sign::timestamp` – RFC 3161: `request` (TimeStampReq mit Nonce, prüft Antwort/Imprint/Nonce; Übertragung
  als `TsaTransport`-Funktion von außen – die Crate macht kein Netzwerk), `verify_token` → `TimestampInfo`;
  Test-TSA `tests::fake_tsa`. Zeitstempel = unsigniertes Attribut 1.2.840.113549.1.9.16.2.14 über den
  Signaturwert; Dokument-Zeitstempel (ETSI.RFC3161) werden beim Prüfen erkannt
- `app::sign_ui` – Dialog „Dokument signieren“ (ID-Datei, Passwort nur im Speicher, Grund/Ort/Kontakt,
  sichtbar = Bereich aufziehen, unsichtbar, vorhandenes Feld), Ergebnis als neue Datei; „Testzertifikat erstellen“;
  Signieren läuft als Job (`open_after_job`), Zeitstempel per `ureq` (`http_tsa`)
- `app::signatures` – `SigState` (neu prüfen bei neuen Bytes/Vertrauensstand), Hinweisleiste, Panel
  (Details, Zertifikate, „vertrauen“, „Signierte Fassung speichern“), Vertrauensspeicher in den Einstellungen
- `core::raster` – Bilddaten dekodieren (roh/Flate/LZW/ASCII, JPEG via zune-jpeg; Grau/RGB/CMYK/ICC/Indexed/Masken,
  1–16 Bit), `blacken` (Pixelbereich, /Decode-Umkehr beachtet), `encode` (Flate); `normalize_inline` für BI-Abkürzungen
- `core::redact` – `redact_pdf(bytes, Bereiche je Seitenobjekt)`: Zeichen ≥ 40 % im Bereich raus (TJ-Verschiebung
  gleicher Breite), unsichere Breiten = ganze Ausgabe raus, /ActualText betroffener BDC-Abschnitte weg, Pfade
  (außer reinen Rechtecken) → `n`, Bilder als geschwärzte Kopie (sonst ganz entfernt), Formular-XObjects
  rekursiv als Kopie, ersetzte Originale aus den Ressourcen entfernt, Annotationen/Widgets im Bereich und
  alle /Redact raus (Felder aus /AcroForm), schwarze Kästen; danach `rewrite` (prune, ohne /Prev).
  `Document::{redaction_input, has_signatures, replacement}`
- `core::sanitize` – `inspect` → `Findings`, `sanitize_pdf(bytes, SanitizeOptions)`: Metadaten (Info, XMP, PieceInfo),
  JavaScript (/OpenAction, /AA, /Names/JavaScript, /JS), eingebettete Dateien (+ FileAttachment), ausgeblendete
  Ebenen (BDC /OC-Abschnitte, XObjects/Annotationen mit /OC, dann /OCProperties weg), Kommentare, Formulardaten
  (/V, /DV, /XFA, Text-AP, Schalter Off); immer `redact::rewrite`
- `AnnotKind::Redact { quads }` – Schwärzungs-Markierung (/Redact, /QuadPoints, /IC schwarz), Vorschau rot
- `app::redact` – zusätzlich Dialog „Dokument bereinigen“ (Fundstellen je Option, Job wie beim Schwärzen);
  Rückfrage (Signaturwarnung), Schwärzen als Job, Ergebnis per `pending_doc`/`DocSlot` als
  ungespeicherte Fassung (`Document::replacement`); `redact_search_hits` (alle Suchtreffer → Redact, 1 Undo-Schritt), `pending_redactions`;
  Werkzeuge `Tool::RedactText` (wie Hervorheben) und `Tool::RedactArea` (Rechteck), Menü „Schwärzen“
- `core::security` – `load` (entschlüsselt beim Öffnen, Fehler `PasswordRequired`/`WrongPassword`), `Security`
  (Verfahren, Berechtigungen, Besitzerpasswort bekannt?), `Protection` → AES-256 (V5/R6, CF mit /Length und /AuthEvent
  für Poppler); `Document::{open_with_password, security, unlock, set_protection}`, Speichern verschlüsselt komplett neu
- `core::tags` – `Document::{is_tagged, structure_tree, marked_content, properties}`, `StructNode` (RoleMap aufgelöst,
  Inhalte als (Seite, MCID)), Commands `SetProperties` (Titel/Sprache in `Model::properties`) und `SetAltText`
  (`Model::alt_texts`); `tags::write` beim Speichern (/Lang, /Info /Title, DisplayDocTitle, /Alt).
  Interpreter: `Interpreted::marked` (Text/Rechteck je MCID), `unmarked` (Inhalte weder MCID noch Artefakt)
- `core::accessibility` – `Document::accessibility_check` → `Check { kind, status, title, detail }`
- `core::autotag` – `auto_tag_pdf(bytes, AutoTagOptions { lang, title, embed_fonts }, progress)`: Absätze/Überschriften
  (Schriftgröße ≥ 1,15 × Fließtext, H1–H3), Bilder → Figure, Pfade/Kopf-/Fußzeilen → Artefakt, jede Ausgabe einzeln in
  BDC/EMC, Annotationen als Link/Form/Annot mit OBJR und /Alt, ParentTree, /Tabs /S, MarkInfo, XMP mit
  `pdfuaid:part=1` (bei PDF/A-2 mit Erweiterungsschema), Schriften über `pdfa::embed_standard_fonts`
- `app::security_ui` – Passwortabfrage beim Öffnen, Dialog „Sicherheit“
- `app::tags_ui` – Panel „Tags“ (`TagState`: Baum gecacht nach Bytes/Seiten/Alt-Texten, markierte Inhalte je Seite
  nach Stempel), Overlay `paint_tags` (Auswahl, nummerierte Lesereihenfolge)
- `app::a11y_ui` – Menü „Barrierefreiheit“: Prüfbericht-Fenster, Dialog „Automatisch taggen“ (Job, `pending_doc`)
- `core::embedded_font::FontData` – beliebige TrueType-Schrift (statisch oder geladen) für `SubsetFont::with_font`;
  `embeddable` prüft glyf und fsType
- `core::system_fonts` – Systemschriften suchen (nur name/OS2 lesen) plus die eingebauten Liberation-Schriften
  (`SystemFont::builtin`, installierte haben Vorrang; Liste wird neu gebildet, wenn eingebaute dazukommen),
  `find_style`, `find_for_base_font` (Aliase wie Arial → Liberation Sans, Calibri → Carlito), `load` (gecacht)
- `core::embedded_font::{LIBERATION_NAMES, liberation()}` – Liberation (Sans/Serif/Mono × 4 Schnitte): Desktop fest
  eingebaut; Browser nicht im WASM, sondern nachgeladen (`app::platform`, `fonts/<Name>.ttf`) und per
  `register_liberation` angemeldet. `FontData::from_static` für eingebaute Daten ohne Kopie
- `core::rich_text` – `TextStyle`/`StyledRun`/`FontRef {Original, Family, Noto}`, `Resolver` (je Zeichen: Original →
  Systemschrift gleicher Familie → Noto), `layout_ops` (Umbruch, Blocksatz, Linien); `core::paragraph::{paragraph_runs,
  set_paragraph_rich, add_rich_text}`
- `app::rich_edit` – `RichEdit` (Text + Stil je Zeichen), `format_bar` (Schrift, Größe, F/K/U/S, Farbe)
- `core::image_edit` – zusätzlich `PageImage::{ctm, in_form, pixels}`, `image_snapshot` (Ein-Bild-PDF zum Kopieren)
- `app::image_clip` – Rechtsklick-Menü für Bilder, Strg+C/Strg+V/Entf, `paint_selected_image` (Auswahlwerkzeug)
- `app::toolbar` (`pick_image`), `app::shortcuts` – Tastenkürzel (`impl PdfToolApp`-Block)

## Tastenkürzel
Strg+O öffnen · Strg+S speichern · Strg+Umschalt+S speichern unter · Strg+Z/Strg+Y (oder Strg+Umschalt+Z) Rückgängig/Wiederholen · Entf löscht Auswahl · Strg+C kopiert markierten Text bzw. das ausgewählte Bild · Strg+V fügt ein Bild ein · Esc hebt Auswahl auf · Strg+F suchen (Enter/F3 weiter, Umschalt+Enter/Umschalt+F3 zurück, Esc schließen) · Strg+Plus/Minus zoomen · Strg+1 = 100 % · Strg+2 = Seitenbreite · Strg+0 = Ganze Seite ·
Formular: Tab/Umschalt+Tab nächstes/voriges Feld · Leertaste schaltet Kästchen/Optionsfeld · Esc verlässt das Feld ·
Strg+Mausrad zoomt um den Mauszeiger · Bild↑/↓ Seite · Pos1/Ende · Pfeiltasten scrollen

## Testen im Container (ohne Bildschirm)
Datei-/Ordnerdialoge (rfd über xdg-portal) funktionieren im Container nicht – Logik dahinter per Test absichern.
PDFium liegt lokal in `pdfium/lib` (gitignored).
`cargo run -p pdftool-core --example annot_demo -- in.pdf out.pdf` schreibt jeden Annotationstyp –
zur Gegenprobe mit anderen Readern (z. B. `pdftoppm -png out.pdf x`).
`cargo run -p pdftool-core --example redact_demo -- in.pdf out.pdf 1:x0,y0,x1,y1 …` schwärzt Bereiche
(Gegenprobe: `pdftotext`, `pdftoppm`).
`cargo run -p pdftool-core --example sanitize_demo -- in.pdf [out.pdf]` zeigt/entfernt versteckte Inhalte
(Gegenprobe: `pdfinfo`, `pdfdetach -list`, `pdftotext`).
`cargo run -p pdftool-core --example content_dump -- datei.pdf [Seite]` zeigt die Interpreter-Ergebnisse
(Gegenprobe: `pdftotext -bbox`, gleiche x-Positionen).
`cargo run -p pdftool-sign --example sign_demo -- in.pdf out.pdf [id.p12 pw] [rsa]` signiert (ohne ID mit Testzertifikat).
Zeitstempel: `--tsa-openssl tsa.cnf` (lokale OpenSSL-TSA, `openssl ts -reply`) oder `--tsa-http URL`.
Öffentliche TSAs sind aus dem Container nicht erreichbar (Netzwerkregel).
Gegenprobe: `pdfsig out.pdf`; CMS mit `openssl cms -verify -binary -inform DER -content daten.bin`.
`cargo run -p pdftool-sign --example sign_check -- datei.pdf [vertraut.pem]` prüft Signaturen (Gegenprobe: `pdfsig`).
Unabhängig signierte Test-PDFs: OpenSSL (`openssl cms -sign -binary -outform DER`) über die ByteRange.
`cargo run -p pdftool-core --example docx_demo -- in.pdf out.docx` (Gegenprobe: python-docx, `soffice --convert-to pdf`).
`cargo run -p pdftool-core --example compress_demo -- in.pdf out.pdf [dpi] [qualität]` (Gegenprobe: `pdfimages -list`).
`cargo run -p pdftool-core --example autotag_demo -- in.pdf [out.pdf] [de-DE] [Titel]` prüft/taggt
(Gegenprobe veraPDF mit `--flavour ua1`), `… --example security_demo -- schützen|öffnen …` (Gegenprobe `qpdf --check`, `pdfinfo`).
`cargo run -p pdftool-core --example pdfa_demo -- in.pdf out.pdf`, Prüfung mit veraPDF (nur Testwerkzeug, nicht im Repo):
`curl -o ~/tools/verapdf.jar https://maven-central.storage-download.googleapis.com/maven2/org/verapdf/apps/greenfield-apps/1.28.2/greenfield-apps-1.28.2.jar`,
`java -cp ~/tools/verapdf.jar org.verapdf.apps.GreenfieldCliWrapper --flavour 2b --format text -v datei.pdf`.
Platz: `target/` wächst schnell (> 25 GB) – im Container mit `CARGO_INCREMENTAL=0` bauen und bei Bedarf `cargo clean`.
`cargo run -p pdftool-ocr --example ocr_demo -- bild.pgm [deu+eng] [dpi]` erkennt Text in einem 8-Bit-PGM.
`PDFTOOL_PDFIUM_PATH=… cargo run -p pdftool-render --example offscreen_pgm -- datei.pdf out.pgm 4.1667` rendert
Seite 1 offscreen (300 dpi), `… --example text_boxes -- datei.pdf [Zeichen]` zeigt PDFiums Zeichen-Rechtecke.
`cargo run -p pdftool-core --example font_list -- [Schriftname …]` zeigt Systemschriften und Zuordnung,
`… --example rich_text_demo -- in.pdf out.pdf [Familie]` setzt einen Absatz formatiert neu (Gegenprobe: `pdffonts`).
`cargo run -p pdftool-core --example form_fill -- in.pdf [out.pdf name=Wert …]` listet/füllt Formularfelder. Screenshots über Xvfb + xdotool + ImageMagick `import`.
