# Builds the release exe and packages it into the setup program: dist\Sky-Island-Setup-<version>.exe
# Needs the free Inno Setup 7 compiler (https://jrsoftware.org/isinfo.php).
#
# Run from anywhere:  powershell -ExecutionPolicy Bypass -File scripts\package.ps1
#
# Before it builds, it checks that the working tree is what a release is made from (see below), runs the tests, and
# runs the crate check (cargo deny) when that is installed. The build uses exactly the crate versions in Cargo.lock.
#   -CheckOnly    only check the working tree and the tools, then stop (builds nothing, writes nothing)
#   -AllowDirty   build even with other changes in the working tree (a test build: do not release it)
param(
    [switch]$AllowDirty,
    [switch]$CheckOnly
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "Packaging Sky Island $version"

$candidates = @(
    "$env:ProgramFiles\Inno Setup 7\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 7\ISCC.exe"
)
$iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    throw 'Inno Setup 7 is not installed. Get it from https://jrsoftware.org/isinfo.php (or: winget install JRSoftware.InnoSetup.7)'
}

# The compiler may sit in a folder any program of yours can write to (Inno Setup installs into the user's own folder
# when it is not run as administrator), and whatever runs here goes into the setup. Run only the real one: its
# Authenticode signature must be valid and be the Inno Setup author's (Pyrsys B.V.).
$signature = Get-AuthenticodeSignature $iscc
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|, )O=Pyrsys B\.V\.(,|$)') {
    throw "ISCC.exe at $iscc is not signed by the Inno Setup author (signature: $($signature.Status), signer: $($signature.SignerCertificate.Subject)). Reinstall Inno Setup 7 from https://jrsoftware.org/isinfo.php."
}

# A release is built from the working tree BEFORE it is committed, and the only changes it makes are the version bump
# and the regenerated licence list. Anything else that differs from the last commit would be in the setup without
# being in the commit that CI tests, so it stops here.
$bump = @('Cargo.toml', 'Cargo.lock', 'installer/sky-island.iss', 'THIRD-PARTY-LICENSES.txt')
if (Get-Command git -ErrorAction SilentlyContinue) {
    $other = @(git status --porcelain --untracked-files=all | ForEach-Object { $_.Substring(3).Trim('"') } | Where-Object { $_ -and ($bump -notcontains $_) })
    if ($other.Count -gt 0) {
        $list = ($other | ForEach-Object { "  $_" }) -join "`n"
        if ($AllowDirty) {
            Write-Warning "These files differ from the last commit (building anyway, because of -AllowDirty):`n$list"
        } else {
            throw "These files differ from the last commit, so the setup would not be what CI tested:`n$list`nCommit or undo them, or use -AllowDirty for a test build."
        }
    }
} else {
    Write-Warning 'git was not found, so the working tree was not checked.'
}

if ($CheckOnly) {
    Write-Host 'Checks passed (-CheckOnly: nothing was built).'
    return
}

cargo test --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo test failed' }

if (Get-Command cargo-deny -ErrorAction SilentlyContinue) {
    cargo deny check
    if ($LASTEXITCODE -ne 0) { throw 'cargo deny check failed' }
} else {
    Write-Warning 'cargo-deny is not installed, so the crate check was skipped (CI runs it).'
}

cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

# The licences of the crates built into the exe travel with it.
& (Join-Path $PSScriptRoot 'third-party.ps1')
if ($LASTEXITCODE -ne 0 -or -not $?) { throw 'third-party licence list failed' }

& $iscc "/DAppVersion=$version" 'installer\sky-island.iss'
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }
Write-Host "Created dist\Sky-Island-Setup-$version.exe"
