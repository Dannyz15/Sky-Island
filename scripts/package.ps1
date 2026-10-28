# Builds the release exe and packages it into the setup program: dist\Sky-Island-Setup-<version>.exe
# Needs the free Inno Setup 6 compiler (https://jrsoftware.org/isinfo.php).
#
# Run from anywhere:  powershell -ExecutionPolicy Bypass -File scripts\package.ps1

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "Packaging Sky Island $version"

$candidates = @(
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
)
$iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    throw 'Inno Setup 6 is not installed. Get it from https://jrsoftware.org/isinfo.php (or: winget install JRSoftware.InnoSetup)'
}

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

& $iscc "/DAppVersion=$version" 'installer\sky-island.iss'
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }
Write-Host "Created dist\Sky-Island-Setup-$version.exe"
