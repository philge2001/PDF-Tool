# FolioCraft

Plattformübergreifender PDF-Editor (Windows, macOS, Linux) in Rust.
„FolioCraft“ ist ein Arbeitstitel. Lizenz: MIT OR Apache-2.0.

**Stand:** Phase 1 (Viewer + Annotationen) und Phase 2 (Seitenorganisation) abgeschlossen.

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

Annotationswerkzeuge stehen in der Leiste „Kommentieren“. Markup-Werkzeuge (Hervorheben usw.)
wirken auf markierten Text: entweder erst Text markieren und dann das Werkzeug anklicken, oder
Werkzeug wählen und über den Text ziehen. Doppelklick auf eine Notiz oder einen Freitext bearbeitet ihn.

**Bekannte Einschränkungen:** siehe [CLAUDE.md](CLAUDE.md#bekannte-einschränkungen).

## Bauen

Voraussetzungen: Rust (stable, ≥ 1.85) und die PDFium-Bibliothek (siehe unten).

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
```

## Projektstruktur

Siehe [CLAUDE.md](CLAUDE.md) für Architektur und Konventionen.
