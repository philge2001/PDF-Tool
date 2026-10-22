# CLAUDE.md – FolioCraft

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
cargo run -p foliocraft-core --example gen_fixtures      # tests/fixtures neu erzeugen
FOLIOCRAFT_PDFIUM_PATH=/pfad/zu/pdfium/lib cargo run -p foliocraft-app -- datei.pdf
```

## Workspace
| Crate | Pfad | Aufgabe | Darf abhängen von |
|---|---|---|---|
| foliocraft-core | crates/core | Dokumentmodell, Annotationen, Commands/Undo, Text-Logik, Speichern (lopdf) | lopdf, thiserror, tracing |
| foliocraft-render | crates/render | PDFium-Worker-Thread, Rendering, Textextraktion, Cache | core, pdfium-render |
| foliocraft-app | crates/app | eframe-App, Werkzeuge, Panels, Shortcuts | core, render, eframe, rfd |

## Architekturregeln
- **core** kennt weder UI noch PDFium und ist vollständig unit-testbar.
- **PDFium läuft nur im Worker-Thread** von `render`. Die UI kommuniziert über Channels, sie blockiert nie.
- **Koordinaten im Modell** sind immer PDF-Benutzerraum (pt, Ursprung unten links, ohne /Rotate).
  Die Umrechnung zum Bildschirm passiert ausschließlich in `app::viewer::transform`.
- **Jede Modelländerung läuft über ein `Command`** (core::command). Nur so funktioniert Undo/Redo.
- **Annotationen**: Unterstützte Typen werden ins Modell importiert und von der App als Overlay gezeichnet.
  Der Render-Worker blendet sie beim Rendern aus. Beim Speichern schreibt core sie mit Appearance Stream (/AP).
- **Speichern** ist ein inkrementelles Update: Die Originalbytes bleiben unverändert, die Änderungen werden angehängt.
- Fehler: `thiserror` in Library-Crates, `anyhow` nur in der App. Logging mit `tracing`.
- Keine AGPL-Abhängigkeiten. Jede neue Crate wird im Commit bzw. hier mit einem Satz begründet.

## Konventionen
- Edition 2024, `unsafe_code = "forbid"` workspace-weit.
- Kommentare und UI-Texte auf Deutsch, Bezeichner auf Englisch.
- Tests für core: Roundtrip „erstellen → speichern → neu laden → vergleichen“ mit den Fixtures.
- Fixtures in `tests/fixtures/` werden ausschließlich per `gen_fixtures` erzeugt.

## Meilensteine Phase 1
- [x] M0 Grundgerüst
- [x] M1 Öffnen & Rendern
- [x] M2 Zoom, Navigation, Thumbnails, Cache
- [ ] M3 Textsuche
- [ ] M4 Text auswählen & kopieren
- [ ] M5 Annotationsmodell, Commands, Speichern (core)
- [ ] M6 Annotationswerkzeuge (UI)
- [ ] M7 Bearbeiten, Kommentar-Panel, Undo/Redo-Shortcuts
- [ ] M8 Speichern in der App
- [ ] M9 Zuletzt geöffnet, Dark/Light Mode

## Bekannte Einschränkungen
- FreeText/Notizen nutzen Helvetica (WinAnsi): Westeuropäische Zeichen gehen, CJK/Kyrillisch noch nicht.

## Wichtige Module
- `core::document` – `Document::open`, `PageInfo` (CropBox, /Rotate, Umrechnung PDF ↔ Anzeige)
- `render::worker` – `RenderService` (Thread + Channels), `RenderKey` (Seite, Skalierungsstufe, Kachel),
  `page_pixel_size`/`tile_grid`/`tile_rect` (gemeinsame Rechenregeln für App und Worker)
- `render::cache::ByteLru` – LRU mit Byte-Budget
- `app::viewer` – `Layout` (Präfixsummen, Binärsuche), `PageTextures` (GPU-Texturen, pending/wanted),
  `Viewer::show` (zeichnet nur sichtbare Seiten, Kacheln ab 3072 px), Zoom mit Anker (`set_zoom`),
  `ZoomMode` (Custom/FitWidth/FitPage), `FrameRequests` (Render-Aufträge pro Frame, `flush_requests`)
- `app::viewer::thumbnails` – Seitenleiste, gleiche Texturen, Priorität `Thumbnail`
- `app::toolbar`, `app::shortcuts` – Werkzeugleiste und Tastenkürzel (je ein `impl FolioApp`-Block)

## Tastenkürzel
Strg+O öffnen · Strg+Plus/Minus zoomen · Strg+1 = 100 % · Strg+2 = Seitenbreite · Strg+0 = Ganze Seite ·
Strg+Mausrad zoomt um den Mauszeiger · Bild↑/↓ Seite · Pos1/Ende · Pfeiltasten scrollen

## Testen im Container (ohne Bildschirm)
PDFium liegt lokal in `pdfium/lib` (gitignored). Screenshots über Xvfb + xdotool + ImageMagick `import`.
