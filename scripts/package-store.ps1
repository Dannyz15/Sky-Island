# Builds the release exe and packs the Microsoft Store package: dist\Sky-Island-<version>.msix
# The file is NOT signed on purpose: you upload it in Partner Center and the Store signs it. It cannot be installed
# on this PC as it is (Windows only installs signed packages); the Windows App Certification Kit can still check it.
# Needs the Windows SDK (makeappx.exe), which Visual Studio installs.
#
# Run from anywhere:  powershell -ExecutionPolicy Bypass -File scripts\package-store.ps1
#   -AllowDirty   build even with other changes in the working tree (a test build: do not upload it)
param(
    [switch]$AllowDirty
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "The version '$version' is not three numbers." }
$msixVersion = "$version.0"   # the Store wants the fourth number to be 0
Write-Host "Packing the Store build of Sky Island $msixVersion"

$makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
if (-not $makeappx) { throw 'makeappx.exe was not found. Install the Windows SDK (Visual Studio installer, or: winget install Microsoft.WindowsSDK.10.0.26100).' }

# The package must be made from what was committed, like the setup (see package.ps1).
$bump = @('Cargo.toml', 'Cargo.lock', 'installer/sky-island.iss', 'THIRD-PARTY-LICENSES.txt')
if (Get-Command git -ErrorAction SilentlyContinue) {
    $other = @(git status --porcelain --untracked-files=all | ForEach-Object { $_.Substring(3).Trim('"') } | Where-Object { $_ -and ($bump -notcontains $_) })
    if ($other.Count -gt 0) {
        $list = ($other | ForEach-Object { "  $_" }) -join "`n"
        if ($AllowDirty) {
            Write-Warning "These files differ from the last commit (building anyway, because of -AllowDirty):`n$list"
        } else {
            throw "These files differ from the last commit, so the package would not be what CI tested:`n$list`nCommit or undo them, or use -AllowDirty for a test build."
        }
    }
}

cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

& (Join-Path $PSScriptRoot 'third-party.ps1')
if ($LASTEXITCODE -ne 0 -or -not $?) { throw 'third-party licence list failed' }

# The folder that becomes the package: the exe, the manifest, the licences and the logos.
$layout = Join-Path 'dist' 'store-layout'
if (Test-Path $layout) { Remove-Item $layout -Recurse -Force }
New-Item -ItemType Directory -Path (Join-Path $layout 'Assets') -Force | Out-Null
Copy-Item 'target\release\sky-island.exe' $layout
Copy-Item 'LICENSE', 'THIRD-PARTY-LICENSES.txt' $layout
(Get-Content 'packaging\store\AppxManifest.xml' -Raw).Replace('__VERSION__', $msixVersion) | Set-Content (Join-Path $layout 'AppxManifest.xml') -Encoding UTF8

# The logos, drawn from assets\logo.png: square ones fill the size, the wide one has the logo in the middle.
Add-Type -AssemblyName System.Drawing
function New-Logo([string]$name, [int]$w, [int]$h) {
    $src = [System.Drawing.Image]::FromFile((Resolve-Path 'assets\logo.png'))
    $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    $side = [Math]::Min($w, $h)
    $g.DrawImage($src, [int](($w - $side) / 2), [int](($h - $side) / 2), $side, $side)
    $bmp.Save((Join-Path (Resolve-Path (Join-Path $layout 'Assets')) $name), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose(); $src.Dispose()
}
New-Logo 'StoreLogo.png' 50 50
New-Logo 'Square44x44Logo.png' 44 44
New-Logo 'Square71x71Logo.png' 71 71
New-Logo 'Square150x150Logo.png' 150 150
New-Logo 'Wide310x150Logo.png' 310 150
New-Logo 'Square310x310Logo.png' 310 310

$out = "dist\Sky-Island-$msixVersion.msix"
if (Test-Path $out) { Remove-Item $out -Force }
& $makeappx.FullName pack /d $layout /p $out /o
if ($LASTEXITCODE -ne 0) { throw 'makeappx failed' }
Remove-Item $layout -Recurse -Force
Write-Host "Created $out (unsigned: upload it in Partner Center)"
