# FolioCraft

Plattformübergreifender PDF-Editor (Windows, macOS, Linux) in Rust.
„FolioCraft“ ist ein Arbeitstitel. Lizenz: MIT OR Apache-2.0.

**Stand:** Phase 1 (Viewer + Annotationen) in Arbeit.

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
```

## Projektstruktur

Siehe [CLAUDE.md](CLAUDE.md) für Architektur und Konventionen.
