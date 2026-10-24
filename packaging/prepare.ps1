# Bereitet packaging\dist\ für den Windows-Installer vor (Gegenstück zu prepare.sh):
#   dist\lib\       pdfium.dll (wird neben pdftool.exe installiert)
#   dist\licenses\  Lizenztexte aller mitgelieferten Bestandteile
#
# Aufruf (PowerShell, im Projektordner):
#   powershell -ExecutionPolicy Bypass -File packaging\prepare.ps1 [win-x64|win-arm64]
# Danach:
#   cargo packager --release -p pdftool-app --formats nsis
param([string]$Platform = "win-x64")
$ErrorActionPreference = "Stop"

# Externes Programm starten; Fehler nur über den Exit-Code. (Windows PowerShell 5.1 würde sonst
# jede Ausgabe auf stderr, z. B. Hinweise von cargo, als Fehler werten und abbrechen.)
function Invoke-Tool([string]$Exe, [string[]]$ArgList) {
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    & $Exe @ArgList 2>&1 | ForEach-Object { Write-Host $_ }
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($code -ne 0) { throw "$Exe $($ArgList -join ' ') fehlgeschlagen (Exit-Code $code)" }
}

# PDFium-Version passend zu pdfium-render 0.9.4 (Feature pdfium_latest = chromium/7881).
$PdfiumVersion = "7881"

$Root = Split-Path -Parent $PSScriptRoot
$Dist = Join-Path $Root "packaging\dist"
if (Test-Path $Dist) { Remove-Item -Recurse -Force $Dist }
New-Item -ItemType Directory -Force (Join-Path $Dist "lib") | Out-Null
$Lic = Join-Path $Dist "licenses"
New-Item -ItemType Directory -Force (Join-Path $Lic "PDFium") | Out-Null

# --- PDFium (bblanchon/pdfium-binaries) ---
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("pdftool-pdfium-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $Tmp | Out-Null
try {
    $Url = "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F$PdfiumVersion/pdfium-$Platform.tgz"
    Write-Host "Lade $Url"
    # curl.exe und tar.exe sind seit Windows 10 (1803) eingebaut.
    Invoke-Tool "curl.exe" @("-fsSL", "--retry", "3", "-o", (Join-Path $Tmp "pdfium.tgz"), $Url)
    Invoke-Tool "tar.exe" @("-xzf", (Join-Path $Tmp "pdfium.tgz"), "-C", $Tmp)
    Copy-Item (Join-Path $Tmp "bin\pdfium.dll") (Join-Path $Dist "lib")
    Copy-Item (Join-Path $Tmp "LICENSE") (Join-Path $Lic "PDFium\LICENSE.txt")
    if (Test-Path (Join-Path $Tmp "licenses")) {
        Copy-Item -Recurse (Join-Path $Tmp "licenses\*") (Join-Path $Lic "PDFium")
    }
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}

# --- Eigene Lizenz und mitgelieferte Daten ---
Copy-Item (Join-Path $Root "LICENSE-MIT") (Join-Path $Lic "PDF-Tool-LICENSE-MIT.txt")
Copy-Item (Join-Path $Root "LICENSE-APACHE") (Join-Path $Lic "PDF-Tool-LICENSE-APACHE.txt")
Copy-Item (Join-Path $Root "assets\fonts\OFL.txt") (Join-Path $Lic "NotoSans-OFL.txt")
Copy-Item (Join-Path $Root "assets\fonts\liberation\COPYRIGHT") (Join-Path $Lic "Liberation-Fonts.txt")
Copy-Item (Join-Path $Root "assets\tessdata\LICENSE") (Join-Path $Lic "Tesseract-Sprachdaten.txt")
Copy-Item (Join-Path $Root "assets\icc\LICENSE") (Join-Path $Lic "sRGB-Profil.txt")
Copy-Item (Join-Path $Root "packaging\licenses\*.txt") $Lic

# --- Rust-Abhängigkeiten (cargo-about, optional) ---
if (Get-Command cargo-about -ErrorAction SilentlyContinue) {
    Push-Location (Join-Path $Root "packaging")
    try {
        Invoke-Tool "cargo" @("about", "generate", "--manifest-path", "..\Cargo.toml", "about.hbs",
            "-o", (Join-Path $Lic "Rust-Bibliotheken.html"))
    } finally { Pop-Location }
} elseif ($env:CI) {
    throw "cargo-about fehlt"
} else {
    Write-Warning "cargo-about fehlt (cargo install cargo-about --locked --features cli) - Lizenzübersicht der Rust-Bibliotheken wird übersprungen."
}

Write-Host "Fertig: $Dist"
Get-ChildItem (Join-Path $Dist "lib")
