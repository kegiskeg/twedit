param (
    [switch]$SkipMsi
)

Write-Host "Building twedit release..." -ForegroundColor Cyan

# 1. Build the release binary
cargo build --release --workspace
if ($LASTEXITCODE -ne 0) {
    Write-Error "Release build failed."
    exit $LASTEXITCODE
}

# 2. Package into a portable ZIP
Write-Host "Creating portable twedit-release.zip..." -ForegroundColor Cyan
$ReleaseDir = "target\release"
$ZipPath = "twedit-portable-release.zip"

if (Test-Path $ZipPath) {
    Remove-Item $ZipPath -Force
}

# Compress the twedit-ui.exe
Compress-Archive -Path "$ReleaseDir\twedit-ui.exe" -DestinationPath $ZipPath

if (-not $SkipMsi) {
    # 3. Build the MSI Installer
    Write-Host "Building MSI installer using cargo-wix..." -ForegroundColor Cyan
    # Build wix in twedit-ui package
    cargo wix -p twedit-ui
    if ($LASTEXITCODE -ne 0) {
        Write-Error "MSI build failed. Make sure cargo-wix and WiX are installed."
        exit $LASTEXITCODE
    }
}

Write-Host "Release packaging complete!" -ForegroundColor Green
Write-Host "Portable ZIP: $ZipPath" -ForegroundColor Green
if (-not $SkipMsi) {
    Write-Host "MSI Installer: Check target\wix\" -ForegroundColor Green
}
