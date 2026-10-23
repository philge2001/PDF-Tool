# FolioCraft

Plattformübergreifender PDF-Editor (Windows, macOS, Linux) in Rust.
„FolioCraft“ ist ein Arbeitstitel. Lizenz: MIT OR Apache-2.0.

**Stand:** Phase 1 (Viewer + Annotationen), Phase 2 (Seitenorganisation), Phase 3 (Formulare), Phase 4 (Signaturen), Phase 5 (Schwärzen & Bereinigen), Phase 6 (Text und Bilder bearbeiten) Phase 7 (Texterkennung) und Phase 8 (Export, Vergleich, Komprimierung, PDF/A) abgeschlossen.

## Funktionen (Phase 1)

- PDF öffnen (Dialog, Drag & Drop, Kommandozeile, „Zuletzt geöffnet“), flüssiges Scrollen auch bei 500+ Seiten
- Zoom (Strg+Mausrad um den Mauszeiger, Prozentfeld, Seitenbreite, Ganze Seite), Seitenleiste mit Miniaturen
- Textsuche mit Trefferhervorhebung, Text auswählen und kopieren
- Annotationen: Hervorheben, Unterstreichen, Durchstreichen, Notiz, Freitext, Freihand, Rechteck, Ellipse, Pfeil
- Annotationen auswählen, verschieben, löschen, Farbe ändern; Kommentar-Panel mit direkter Bearbeitung
- Rückgängig/Wiederholen für alle Änderungen
- Speichern als standardkonforme PDF-Annotationen mit Appearance Streams (sichtbar in anderen Readern),
  **inkrementell** – der Originalinhalt der Datei bleibt Byte für Byte erhalten
- Helles/dunkles Design, Einstellungen (Autorname) werden gespeichert

## Funktionen (Phase 2 – Seitenorganisation)

- Seiten drehen, löschen und per Drag & Drop in der Seitenleiste neu anordnen (Mehrfachauswahl mit
  Strg/Umschalt, Rechtsklick-Menü, Menü „Seiten“) – alles rückgängig machbar
- Seiten aus einer anderen PDF einfügen (Seitenbereich, Position), leere Seite einfügen
- Dateien zusammenführen (Datei → Dateien zusammenführen…)
- Seiten als neue Datei extrahieren (optional danach löschen), Dokument teilen (alle N Seiten oder nach
  Bereichen wie `1-3; 4-6; 7-`) – läuft im Hintergrund mit Fortschritt und Abbrechen

## Funktionen (Phase 3 – Formulare)

- Formulare ausfüllen: Textfelder (ein-/mehrzeilig, Passwort, Höchstlänge), Kontrollkästchen, Optionsfelder,
  Kombinations- und Listenfelder (Mehrfachauswahl mit Strg+Klick); Tab springt zum nächsten Feld
- Felder hervorheben, Pflichtfelder rot umrandet, schreibgeschützte Felder gesperrt; „Formular zurücksetzen“
- Formular bearbeiten (Werkzeugleiste „Formular bearbeiten“): Felder aufziehen, verschieben, Größe ändern,
  löschen; Eigenschaften-Panel (Name, Kurzinfo, Pflicht, Schreibschutz, Einträge, Exportwerte, Schrift,
  Ausrichtung, Rahmen, Hintergrund, Standardwert)
- Speichern mit eigenen Appearance Streams – andere Reader zeigen die Werte ohne Neuberechnung;
  inkrementell, vorhandene Feldeigenschaften und -aktionen bleiben erhalten
- Seiten einfügen/zusammenführen übernimmt die Formularfelder (bei Namenskonflikten mit Zusatz `_2`),
  Extrahieren/Teilen nimmt das Formular der gewählten Seiten mit
- JavaScript in Formularen (Berechnungen, Formatierungen) wird bewusst nicht ausgeführt

## Funktionen (Phase 4 – Digitale Signaturen)

- Signaturen prüfen: Hinweisleiste und Signatur-Panel mit Ampel (gültig / Identität nicht bestätigt /
  ungültig), Unterzeichner, Zeit, Grund, Zertifikatskette, „nach dem Signieren geändert“,
  „Signierte Fassung speichern“; RSA und ECDSA (P-256/P-384), SHA-1 bis SHA-512
- Vertrauenswürdige Zertifikate verwalten (Einstellungen, oder „… vertrauen“ direkt im Panel)
- Signieren (PAdES B-B, `ETSI.CAdES.detached`) mit einer digitalen ID als PKCS#12-Datei (.p12/.pfx):
  sichtbar (Bereich aufziehen), unsichtbar oder in ein vorhandenes Signaturfeld
- Zeitstempel eines Zeitstempeldienstes (RFC 3161, PAdES B-T) – optional, URL frei wählbar
- „Testzertifikat erstellen“: selbst ausgestellte digitale ID zum Ausprobieren
- Signieren ist inkrementell: vorhandene Signaturen bleiben gültig; Ausfüllen/Kommentieren nach dem
  Signieren wird als „danach geändert“ angezeigt

## Funktionen (Phase 5 – Schwärzen & Bereinigen)

- Schwärzungen markieren: „Text schwärzen“ (wie Hervorheben), „Bereich schwärzen“ (Rechteck aufziehen),
  „Alle Treffer schwärzen“ in der Suche – zunächst als rot umrandete Vorschau, rückgängig machbar
- „Schwärzungen anwenden“ entfernt die Inhalte **wirklich**: Zeichen aus dem Text (der Rest bleibt an seiner
  Stelle), Grafik, Bildpixel (auch in JPEG-Scans), Inhalte von Formular-XObjects, Kommentare und Formularfelder
  im Bereich, Ersatztexte für Screenreader; dazu schwarze Kästen. Die Datei wird komplett neu geschrieben,
  alte Revisionen verschwinden.
- „Dokument bereinigen“: Metadaten, JavaScript, eingebettete Dateien, ausgeblendete Ebenen, Kommentare,
  Formulardaten – mit Anzeige, was gefunden wurde
- Warnung, wenn signierte Dokumente betroffen sind

## Funktionen (Phase 9 – Sicherheit & Barrierefreiheit)

- Passwortgeschützte PDFs öffnen (RC4, AES-128, AES-256); beim Speichern bleibt der Schutz erhalten
- „Datei → Sicherheit…“: Schutz festlegen (AES-256) mit Passwort zum Öffnen, Besitzerpasswort und
  Berechtigungen (Drucken, Kopieren, Kommentieren, Ändern) – oder Schutz entfernen (mit Besitzerpasswort)
- „Barrierefreiheit → Prüfen…“: Bericht mit Ampel (getaggt? Sprache? Titel? Alternativtexte? Schriften
  eingebettet? Inhalte ohne Tag? Scans? Tab-Reihenfolge? Lesezeichen?) und passenden Knöpfen zum Beheben
- „Barrierefreiheit → Automatisch taggen…“: Überschriften, Absätze, Bilder, Links und Felder werden aus dem
  Layout erkannt, Linien und Kopf-/Fußzeilen als Artefakt markiert; Sprache, Titel und Schriften gleich mit
- Panel „Tags“: Titel und Sprache ändern, Strukturbaum in Lesereihenfolge (Klick zeigt die Stelle auf der
  Seite), Alternativtexte für Abbildungen, nummerierte Lesereihenfolge auf den Seiten

## Funktionen (Phase 8 – Export, Vergleich, Komprimierung, PDF/A)

- „Datei → Exportieren…“: Seiten als PNG oder JPEG (Auflösung, Qualität), Text als TXT, einfaches Word (DOCX)
  mit Absätzen und Bildern
- „Datei → Vergleichen mit…“: Textunterschiede zu einer anderen PDF – Liste und beide Seiten nebeneinander
  (gelöscht rot, eingefügt grün); auch per `foliocraft alt.pdf --vergleichen neu.pdf`
- „Datei → Komprimieren…“: Bilder auf eine Ziel-Auflösung verkleinern (JPEG), alte Fassungen und unbenutzte
  Objekte entfernen, Objekt-Streams – mit Größe vorher/nachher
- „Datei → Nach PDF/A umwandeln“: PDF/A-2b für die Archivierung (Schriften einbetten, Farbprofil, Metadaten,
  fehlende Erscheinungsbilder erzeugen) – mit Bericht, was angepasst wurde und was nicht ging

## Funktionen (Phase 7 – Texterkennung)

- „Seiten → Texterkennung (OCR)…“: gescannte Seiten werden durchsuchbar – der erkannte Text liegt
  unsichtbar über dem Bild (Suchen, Markieren, Kopieren funktionieren, auch in anderen Readern)
- Auswahl der Seiten, „Nur Seiten ohne Text“, Sprachen Deutsch/Englisch (fest eingebaut);
  läuft im Hintergrund mit Fortschritt und Abbrechen, Ergebnis ist ein Undo-Schritt
- Erneutes Erkennen ersetzt die alte Textebene

## Funktionen (Phase 6 – Text und Bilder bearbeiten)

- Freitext mit vollem Unicode (Umlaute, Kyrillisch, Griechisch, Sonderzeichen): eingebettet wird nur die
  benötigte Teilmenge der Schrift Noto Sans, Kopieren und Suchen liefern den richtigen Text
- „Inhalt bearbeiten“ → „Text bearbeiten“ arbeitet mit **Absätzen**: anklicken = Kasten auswählen, ziehen =
  verschieben, Anfasser rechts = Breite ändern (Text bricht neu um), Doppelklick = Text ändern (Strg+Enter),
  „Blocksatz“ an/aus. Originalschrift, -größe und -farbe bleiben, fehlende Zeichen kommen aus Noto Sans.
  Rückgängig machbar.
- „Text hinzufügen“: auf die Seite klicken, Text eingeben (Farbe und Größe in der Leiste)
- „Bilder“: Bild anklicken und ziehen (verschieben), Anfasser unten rechts (Größe, Seitenverhältnis bleibt),
  „Ersetzen…“, „Löschen“ (Entf), „Bild einfügen…“ (PNG mit Transparenz oder JPEG, Klick platziert)

## Bedienung

| Aktion | Tastenkürzel |
|---|---|
| Öffnen / Speichern / Speichern unter | Strg+O / Strg+S / Strg+Umschalt+S |
| Suchen (weiter / zurück / schließen) | Strg+F (Enter, F3 / Umschalt+Enter, Umschalt+F3 / Esc) |
| Kopieren | Strg+C |
| Rückgängig / Wiederholen | Strg+Z / Strg+Y oder Strg+Umschalt+Z |
| Annotation löschen | Entf |
| Zoomen | Strg+Mausrad, Strg+Plus/Minus |
| 100 % / Seitenbreite / Ganze Seite | Strg+1 / Strg+2 / Strg+0 |
| Seite vor/zurück, Anfang/Ende | Bild↓/Bild↑, Pos1/Ende |
| Abbrechen / zurück zur Textauswahl | Esc |
| Formular: nächstes / voriges Feld | Tab / Umschalt+Tab |
| Formular: Kästchen/Option umschalten | Leertaste |
| Formular bearbeiten: Feld löschen | Entf |

Annotationswerkzeuge stehen in der Leiste „Kommentieren“. Markup-Werkzeuge (Hervorheben usw.)
wirken auf markierten Text: entweder erst Text markieren und dann das Werkzeug anklicken, oder
Werkzeug wählen und über den Text ziehen. Doppelklick auf eine Notiz oder einen Freitext bearbeitet ihn.

**Bekannte Einschränkungen:** siehe [CLAUDE.md](CLAUDE.md#bekannte-einschränkungen).

## Bauen

Voraussetzungen: Rust (stable, ≥ 1.85), die PDFium-Bibliothek (siehe unten) und – für die
Texterkennung – **CMake** und ein C++-Compiler.

**Texterkennung (Tesseract):** Beim ersten Build lädt `tesseract-rs` die Quellen von Tesseract 5 und
Leptonica (~50 MB) herunter und baut sie mit CMake (einmalig 5–15 Minuten). Danach liegen sie im Cache
(`%APPDATA%\tesseract-rs` unter Windows, `~/.tesseract-rs` unter Linux,
`~/Library/Application Support/tesseract-rs` unter macOS).
- Windows: `winget install Kitware.CMake` (die C++-Build-Tools von Visual Studio braucht Rust ohnehin),
  danach ein neues Terminal öffnen.
- macOS: `brew install cmake` · Linux: `sudo apt install cmake g++`
- Ohne Texterkennung bauen: `cargo build -p foliocraft-app --no-default-features`

```bash
cargo build --release
cargo run -p foliocraft-app -- pfad/zur/datei.pdf   # startet die App, optional mit Datei
cargo test --workspace                              # Tests (brauchen KEIN PDFium)
cargo clippy --workspace --all-targets -- -D warnings
```

Linux braucht zusätzlich die üblichen Fenster-Bibliotheken, z. B. unter Debian/Ubuntu:
`sudo apt install libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`

## PDFium beschaffen

FolioCraft rendert PDFs mit [PDFium](https://pdfium.googlesource.com/pdfium/) (Apache-2.0),
das **zur Laufzeit dynamisch geladen** wird. Fertige Binaries gibt es bei
[bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases) (Apache-2.0/BSD).

| Plattform | Archiv | Datei darin |
|---|---|---|
| Windows x64 | `pdfium-win-x64.tgz` | `bin/pdfium.dll` |
| Windows ARM64 | `pdfium-win-arm64.tgz` | `bin/pdfium.dll` |
| macOS Apple Silicon | `pdfium-mac-arm64.tgz` | `lib/libpdfium.dylib` |
| macOS Intel | `pdfium-mac-x64.tgz` | `lib/libpdfium.dylib` |
| Linux x64 | `pdfium-linux-x64.tgz` | `lib/libpdfium.so` |
| Linux ARM64 | `pdfium-linux-arm64.tgz` | `lib/libpdfium.so` |

Beispiel Linux/macOS:

```bash
mkdir -p pdfium && cd pdfium
curl -L -o pdfium.tgz https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-x64.tgz
tar xzf pdfium.tgz && cd ..
export FOLIOCRAFT_PDFIUM_PATH="$PWD/pdfium/lib"   # Ordner ODER direkter Dateipfad
cargo run -p foliocraft-app
```

Windows (PowerShell):

```powershell
mkdir pdfium; cd pdfium
curl.exe -L -o pdfium.tgz https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-win-x64.tgz
tar xzf pdfium.tgz; cd ..
$env:FOLIOCRAFT_PDFIUM_PATH = "$PWD\pdfium\bin"
cargo run -p foliocraft-app
```

Die App sucht PDFium in dieser Reihenfolge:
1. Umgebungsvariable `FOLIOCRAFT_PDFIUM_PATH` (Ordner oder Datei)
2. Ordner der ausführbaren Datei (für Auslieferung: Bibliothek neben `foliocraft[.exe]` legen)
3. Unterordner `pdfium/lib` bzw. `pdfium/bin` im aktuellen Arbeitsverzeichnis
4. Systembibliothek (z. B. `/usr/lib`)

Fehlt PDFium, zeigt die App einen Hinweis statt abzustürzen.

## Test-PDFs

Alle Dateien in `tests/fixtures/` sind selbst erzeugt:

```bash
cargo run -p foliocraft-core --example gen_fixtures                    # Fixtures neu erzeugen
cargo run -p foliocraft-core --example gen_fixtures -- big 500 big.pdf # 500-Seiten-PDF für Performance-Tests
cargo run -p foliocraft-core --example annot_demo -- in.pdf out.pdf     # jeden Annotationstyp schreiben (Gegenprobe in anderen Readern)
cargo run -p foliocraft-core --example redact_demo -- in.pdf out.pdf 1:x0,y0,x1,y1                # Bereich schwärzen
cargo run -p foliocraft-core --example sanitize_demo -- in.pdf [out.pdf]                           # versteckte Inhalte zeigen/entfernen
cargo run -p foliocraft-sign --example sign_check -- datei.pdf [vertraut.pem]                     # Signaturen prüfen
cargo run -p foliocraft-sign --example sign_demo -- in.pdf out.pdf [id.p12 passwort]              # signieren (ohne ID: Testzertifikat)
cargo run -p foliocraft-core --example form_fill -- tests/fixtures/form.pdf                        # Formularfelder auflisten
cargo run -p foliocraft-core --example form_fill -- in.pdf out.pdf "person.name=Erika" agb=an     # Formular füllen
cargo run -p foliocraft-core --example text_edit_demo -- in.pdf                                   # Textblöcke von Seite 1
cargo run -p foliocraft-core --example text_edit_demo -- in.pdf out.pdf "Lorem=Neuer Text" "+72,500=Hallo"  # bearbeiten/hinzufügen
cargo run -p foliocraft-core --example text_edit_demo -- in.pdf out.pdf "!bild.png@300,400,120" "#0=foto.jpg" "-#1"  # Bilder
cargo run -p foliocraft-core --example text_edit_demo -- in.pdf out.pdf "¶Lorem|72,720,260,j=Neuer Absatz …"      # Absatz neu setzen
cargo run -p foliocraft-core --example security_demo -- schützen in.pdf out.pdf benutzer besitzer [nur-lesen]  # verschlüsseln
cargo run -p foliocraft-core --example security_demo -- öffnen datei.pdf [passwort]                # Schutz anzeigen
cargo run -p foliocraft-core --example autotag_demo -- in.pdf [out.pdf] [de-DE] [Titel]          # prüfen / automatisch taggen
```

## Projektstruktur

Siehe [CLAUDE.md](CLAUDE.md) für Architektur und Konventionen.

## Mitgelieferte Schriften und Profile (Phase 8)

- `assets/fonts/liberation/` – Liberation Sans/Serif/Mono (SIL Open Font License 1.1, `COPYRIGHT`):
  Ersatz für nicht eingebettete Standardschriften bei der PDF/A-Umwandlung.
- `assets/icc/sRGB-v2-magic.icc` – sRGB-Profil aus Compact-ICC-Profiles (CC0, `assets/icc/LICENSE`).
- JPEG-Kodierung mit `jpeg-encoder`: „This software is based in part on the work of the Independent JPEG Group.“

## Mitgelieferte Sprachdaten

`assets/tessdata/deu.traineddata` und `eng.traineddata` aus
[tessdata_fast](https://github.com/tesseract-ocr/tessdata_fast) (Apache-2.0, `assets/tessdata/LICENSE`),
fest in das Programm eingebaut.

## Mitgelieferte Schrift

`assets/fonts/NotoSans-Regular.ttf` – Noto Sans, © The Noto Project Authors, unter der
SIL Open Font License 1.1 (`assets/fonts/OFL.txt`). Sie wird in das Programm eingebaut und als
Teilmenge in PDFs eingebettet; die OFL erlaubt das ausdrücklich.
