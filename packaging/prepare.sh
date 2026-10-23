#!/usr/bin/env bash
# Bereitet `packaging/dist/` für den Installer vor:
#   dist/lib/       PDFium-Bibliothek der Zielplattform (wird neben die App gelegt)
#   dist/licenses/  Lizenztexte aller mitgelieferten Bestandteile
#
# Aufruf: bash packaging/prepare.sh <win-x64|mac-arm64|mac-x64|linux-x64>
# Danach: cargo packager --release -p pdftool-app --formats <nsis|dmg|deb|appimage>
set -euo pipefail

platform=${1:?"Plattform fehlt: win-x64, mac-arm64, mac-x64 oder linux-x64"}
# PDFium-Version passend zu pdfium-render 0.9.4 (Feature `pdfium_latest` = chromium/7881).
pdfium_version=7881

root=$(cd "$(dirname "$0")/.." && pwd)
dist="$root/packaging/dist"
rm -rf "$dist"
mkdir -p "$dist/lib" "$dist/licenses/PDFium"

# --- PDFium (bblanchon/pdfium-binaries) ---
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
url="https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F${pdfium_version}/pdfium-${platform}.tgz"
echo "Lade $url"
curl -fsSL --retry 3 -o "$tmp/pdfium.tgz" "$url"
tar xzf "$tmp/pdfium.tgz" -C "$tmp"
case "$platform" in
    win-*) cp "$tmp/bin/pdfium.dll" "$dist/lib/" ;;
    mac-*) cp "$tmp/lib/libpdfium.dylib" "$dist/lib/" ;;
    *)     cp "$tmp/lib/libpdfium.so" "$dist/lib/" ;;
esac
cp "$tmp/LICENSE" "$dist/licenses/PDFium/LICENSE.txt"
if [ -d "$tmp/licenses" ]; then cp -R "$tmp/licenses/." "$dist/licenses/PDFium/"; fi

# --- Eigene Lizenz und mitgelieferte Daten ---
lic="$dist/licenses"
cp "$root/LICENSE-MIT" "$lic/PDF-Tool-LICENSE-MIT.txt"
cp "$root/LICENSE-APACHE" "$lic/PDF-Tool-LICENSE-APACHE.txt"
cp "$root/assets/fonts/OFL.txt" "$lic/NotoSans-OFL.txt"
cp "$root/assets/fonts/liberation/COPYRIGHT" "$lic/Liberation-Fonts.txt"
cp "$root/assets/tessdata/LICENSE" "$lic/Tesseract-Sprachdaten.txt"
cp "$root/assets/icc/LICENSE" "$lic/sRGB-Profil.txt"
cp "$root/packaging/licenses/"*.txt "$lic/"

# --- Rust-Abhängigkeiten (cargo-about) ---
if cargo about --version >/dev/null 2>&1; then
    (cd "$root/packaging" && cargo about generate --manifest-path ../Cargo.toml about.hbs \
        -o "$lic/Rust-Bibliotheken.html")
else
    echo "Hinweis: cargo-about fehlt (cargo install cargo-about --locked --features cli)" >&2
    [ -z "${CI:-}" ] || exit 1
fi

echo "Fertig: $dist"
ls -R "$dist" | head -40
