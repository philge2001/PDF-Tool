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
- [x] M3 Textsuche
- [x] M4 Text auswählen & kopieren
- [x] M5 Annotationsmodell, Commands, Speichern (core)
- [x] M6 Annotationswerkzeuge (UI)
- [ ] M7 Bearbeiten, Kommentar-Panel, Undo/Redo-Shortcuts
- [ ] M8 Speichern in der App
- [ ] M9 Zuletzt geöffnet, Dark/Light Mode

## Bekannte Einschränkungen
- Overlay vs. Datei: Hervorhebungen im Overlay halbtransparent (egui kann kein Multiply),
  Freitext im Overlay mit egui-Schrift (Zeilenumbrüche identisch) und auf gedrehten Seiten nicht mitgedreht.
- Importierte Annotationen mit exotischer Darstellung (z. B. Wolkenrand) werden vereinfacht neu gezeichnet.
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
- `core::annotation` – `Annotation`/`AnnotKind` (9 Typen), `AnnotationStore`; `import` (PDF -> Modell,
  Unverstandenes bleibt fremd), `export` (Dictionary, Basis = Original-Dict), `appearance` (/AP-Streams)
- `core::command` – `Command`-Trait, `Model` (was Commands ändern dürfen), `History` (Undo/Redo, dirty)
- `core::save` – inkrementelles Speichern: pro Seite fremde Einträge + verwaltete; unveränderte
  behalten ihren Verweis, geänderte werden unter derselben Objektnummer neu geschrieben
- `core::font` (Helvetica-Breiten, WinAnsi, Umbruch), `core::pdf_string` (Textstrings), `core::date`
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
- `app::toolbar`, `app::shortcuts` – Werkzeugleiste und Tastenkürzel (je ein `impl FolioApp`-Block)

## Tastenkürzel
Strg+O öffnen · Strg+C kopiert markierten Text · Esc hebt Auswahl auf · Strg+F suchen (Enter/F3 weiter, Umschalt+Enter/Umschalt+F3 zurück, Esc schließen) · Strg+Plus/Minus zoomen · Strg+1 = 100 % · Strg+2 = Seitenbreite · Strg+0 = Ganze Seite ·
Strg+Mausrad zoomt um den Mauszeiger · Bild↑/↓ Seite · Pos1/Ende · Pfeiltasten scrollen

## Testen im Container (ohne Bildschirm)
PDFium liegt lokal in `pdfium/lib` (gitignored).
`cargo run -p foliocraft-core --example annot_demo -- in.pdf out.pdf` schreibt jeden Annotationstyp –
zur Gegenprobe mit anderen Readern (z. B. `pdftoppm -png out.pdf x`). Screenshots über Xvfb + xdotool + ImageMagick `import`.
