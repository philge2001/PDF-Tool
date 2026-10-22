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
| foliocraft-core | crates/core | Dokumentmodell, Annotationen, Commands/Undo, Text-Logik, Speichern (lopdf) | lopdf, thiserror, tracing, zune-jpeg, ttf-parser, subsetter, png |
| foliocraft-render | crates/render | PDFium-Worker-Thread, Rendering, Textextraktion, Cache | core, pdfium-render |
| foliocraft-sign | crates/sign | Signaturen prüfen/erstellen (CMS, X.509, PAdES) | core, RustCrypto (cms, x509-cert, der, spki, rsa, p256, p384, sha1, sha2), rand_core, p12-keystore |
| foliocraft-app | crates/app | eframe-App, Werkzeuge, Panels, Shortcuts | core, render, sign, eframe, rfd, der, ureq |

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
- Keine AGPL-Abhängigkeiten. Jede neue Crate wird im Commit bzw. hier mit einem Satz begründet.

## Konventionen
- Edition 2024, `unsafe_code = "forbid"` workspace-weit.
- Kommentare und UI-Texte auf Deutsch, Bezeichner auf Englisch.
- Tests für core: Roundtrip „erstellen → speichern → neu laden → vergleichen“ mit den Fixtures.
- Fixtures in `tests/fixtures/` werden ausschließlich per `gen_fixtures` erzeugt.

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
- Verschlüsselte PDFs mit Benutzerpasswort werden noch nicht unterstützt (Passwortdialog fehlt).
- Annotationen auf gedrehten Seiten: Geometrie korrekt, Freitext-Overlay nicht mitgedreht.
- Absätze bearbeiten: Formatierung innerhalb des Absatzes (z. B. ein fettes Wort) wird auf den Stil der ersten
  Zeile vereinheitlicht; nur links-/Blocksatz (zentriert/rechtsbündig wird linksbündig neu gesetzt); Kerning geht verloren;
  aus eingebetteten Teilmengen nur Zeichen, die auf der Seite schon vorkommen (Rest in Noto Sans, kein CJK);
  der Seiteninhalt wird neu kodiert (ein Stream); eingefügte, ungespeicherte Seiten erst nach dem Speichern.
- Bilder bearbeiten: nur Bilder direkt im Seiteninhalt (nicht in Formular-XObjects); Zeichenreihenfolge bleibt;
  PNG-Farbprofile/Gamma werden ignoriert.
- Formularfelder im Overlay mit egui-Schrift; eigene Erscheinungsbilder von Kontrollkästchen/Optionsfeldern
  (z. B. Kreuz statt Häkchen) zeigt FolioCraft vereinfacht – in der Datei bleiben sie erhalten.
  Felder auf gedrehten Seiten: Text nicht mitgedreht. Formular-JavaScript (Berechnungen, Formate) wird nicht ausgeführt.
- Schwärzen: Pfade, die einen Bereich kreuzen, verschwinden ganz (kein Zuschneiden); Bilder in JPEG 2000, JBIG2,
  CCITT werden ganz entfernt; Ersatztexte nur in direkten BDC-Dictionaries (nicht über /Properties-Ressourcen).
- Signaturen: keine Sperrprüfung (OCSP/CRL) und kein LTV (B-LT/B-LTA), RSA-PSS wird nicht geprüft,
  DocMDP-Berechtigungen werden angezeigt, aber Änderungen danach nicht darauf geprüft. Digitale IDs nur als
  PKCS#12-Datei (kein Windows-Zertifikatspeicher, keine Smartcards). Vertrauen nur über selbst
  hinterlegte Zertifikate (keine eingebaute Liste von Stammzertifikaten).
- Freitext nutzt die eingebettete Noto Sans (Latein, Kyrillisch, Griechisch); CJK fehlt der Schrift.
  Formularfelder und sichtbare Signaturen nutzen weiterhin Helvetica (WinAnsi).

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
  ersten Zeile (Schrift, Größe, Füllfarbe aus `TextShow::fill`, Tc/Tw/Tz/Ts/Tr, Matrix), Umbruch auf die Kastenbreite
- `app::content_edit` – Modus „Inhalt bearbeiten“ (`Tool::EditText`/`AddText`): Absätze pro Seite gecacht (nach
  Stempel), Klick = Auswahl, Ziehen = verschieben, Anfasser rechts = Breite, Doppelklick/Enter = mehrzeiliger
  Editor (Strg+Enter übernimmt), Blocksatz-Schalter; `sync_structure` erkennt Inhaltsänderungen am Stempel
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
- `app::tools::select` – Annotationen anklicken/verschieben (Vorschau via `AnnotDrag`), löschen,
  `update_selected` (zusammenführbare Updates = 1 Undo-Schritt, siehe `History::execute_merging`)
- `app::comments` – Kommentar-Panel (nach Seite und Position sortiert, direkt bearbeitbar)
- `app::file_ops` – Speichern/Speichern unter, Rückfrage bei ungespeicherten Änderungen
  (`PendingAction`), Fenstertitel mit `*`; nach dem Speichern lädt der Worker neu (`reload_renderer`)
- `app::menu` – Menüleiste Datei/Bearbeiten/Ansicht
- `app::settings` – dauerhafte Einstellungen (serde + eframe-Persistenz, Schlüssel `foliocraft_settings`):
  zuletzt geöffnet, Design, Panels, Werkzeugstil, Autor
- `app::pages` – `PageAction`, `sync_structure` (erkennt jede Strukturänderung inkl. Undo/Redo, biegt
  Texturen/Text per `PageId` auf neue Indizes um, lädt den Worker mit `render_snapshot` und NEUER DocId),
  `new_doc_id`, `target_pages` (Auswahl in der Seitenleiste oder aktuelle Seite)
- `app::viewer::thumbnails` – zusätzlich Mehrfachauswahl (als `PageId`), Kontextmenü, Drag & Drop (`ThumbAction`)
- `app::page_dialogs` – Dialoge „Seiten aus Datei einfügen“, „Dateien zusammenführen“, „Seiten extrahieren“,
  „Dokument teilen“
  (Ergebnis = neues unbenanntes Dokument, `adopt_document` zeigt es über eine Momentaufnahme an)
- `app::jobs` – Hintergrund-Jobs (`Job::spawn`, Fortschritt, Abbrechen, modales Fortschrittsfenster);
  höchstens ein Job gleichzeitig (`FolioApp::job`), Ergebnis als Info- oder Fehlerdialog
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
- `app::toolbar`, `app::shortcuts` – Werkzeugleiste und Tastenkürzel (je ein `impl FolioApp`-Block)

## Tastenkürzel
Strg+O öffnen · Strg+S speichern · Strg+Umschalt+S speichern unter · Strg+Z/Strg+Y (oder Strg+Umschalt+Z) Rückgängig/Wiederholen · Entf löscht Auswahl · Strg+C kopiert markierten Text · Esc hebt Auswahl auf · Strg+F suchen (Enter/F3 weiter, Umschalt+Enter/Umschalt+F3 zurück, Esc schließen) · Strg+Plus/Minus zoomen · Strg+1 = 100 % · Strg+2 = Seitenbreite · Strg+0 = Ganze Seite ·
Formular: Tab/Umschalt+Tab nächstes/voriges Feld · Leertaste schaltet Kästchen/Optionsfeld · Esc verlässt das Feld ·
Strg+Mausrad zoomt um den Mauszeiger · Bild↑/↓ Seite · Pos1/Ende · Pfeiltasten scrollen

## Testen im Container (ohne Bildschirm)
Datei-/Ordnerdialoge (rfd über xdg-portal) funktionieren im Container nicht – Logik dahinter per Test absichern.
PDFium liegt lokal in `pdfium/lib` (gitignored).
`cargo run -p foliocraft-core --example annot_demo -- in.pdf out.pdf` schreibt jeden Annotationstyp –
zur Gegenprobe mit anderen Readern (z. B. `pdftoppm -png out.pdf x`).
`cargo run -p foliocraft-core --example redact_demo -- in.pdf out.pdf 1:x0,y0,x1,y1 …` schwärzt Bereiche
(Gegenprobe: `pdftotext`, `pdftoppm`).
`cargo run -p foliocraft-core --example sanitize_demo -- in.pdf [out.pdf]` zeigt/entfernt versteckte Inhalte
(Gegenprobe: `pdfinfo`, `pdfdetach -list`, `pdftotext`).
`cargo run -p foliocraft-core --example content_dump -- datei.pdf [Seite]` zeigt die Interpreter-Ergebnisse
(Gegenprobe: `pdftotext -bbox`, gleiche x-Positionen).
`cargo run -p foliocraft-sign --example sign_demo -- in.pdf out.pdf [id.p12 pw] [rsa]` signiert (ohne ID mit Testzertifikat).
Zeitstempel: `--tsa-openssl tsa.cnf` (lokale OpenSSL-TSA, `openssl ts -reply`) oder `--tsa-http URL`.
Öffentliche TSAs sind aus dem Container nicht erreichbar (Netzwerkregel).
Gegenprobe: `pdfsig out.pdf`; CMS mit `openssl cms -verify -binary -inform DER -content daten.bin`.
`cargo run -p foliocraft-sign --example sign_check -- datei.pdf [vertraut.pem]` prüft Signaturen (Gegenprobe: `pdfsig`).
Unabhängig signierte Test-PDFs: OpenSSL (`openssl cms -sign -binary -outform DER`) über die ByteRange.
`cargo run -p foliocraft-core --example form_fill -- in.pdf [out.pdf name=Wert …]` listet/füllt Formularfelder. Screenshots über Xvfb + xdotool + ImageMagick `import`.
