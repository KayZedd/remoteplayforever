# Builds the release binary and packs it into an Inno Setup installer under dist\.
# Requires Inno Setup 6 (https://jrsoftware.org/isinfo.php); pass -InstallInnoSetup to get it through winget.
param([switch]$InstallInnoSetup)

$ErrorActionPreference = "Stop"

Write-Host "[1/3] Building the release binary..." -ForegroundColor Yellow
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Error "cargo build failed"
}

Write-Host "[2/3] Locating the Inno Setup compiler (ISCC)..." -ForegroundColor Yellow
$candidates = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
)
$iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    $onPath = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($onPath) { $iscc = $onPath.Source }
}
if (-not $iscc -and $InstallInnoSetup) {
    winget install JRSoftware.InnoSetup --silent --accept-package-agreements --accept-source-agreements
    $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) {
    Write-Error "Inno Setup 6 not found. Install it from https://jrsoftware.org/isinfo.php or rerun with -InstallInnoSetup."
}
Write-Host "  -> $iscc" -ForegroundColor Green

Write-Host "[3/3] Building the installer..." -ForegroundColor Yellow
& $iscc "installer\installer.iss"
if ($LASTEXITCODE -ne 0) {
    Write-Error "Inno Setup failed"
}

$installer = Get-ChildItem "dist\RemotePlayForever-Setup-*.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
Write-Host "Done: $($installer.FullName) ($([math]::Round($installer.Length / 1MB, 2)) MB)" -ForegroundColor Green
