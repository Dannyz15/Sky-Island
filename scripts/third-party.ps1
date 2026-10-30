# Writes THIRD-PARTY-LICENSES.txt: the licence of every crate that is built into sky-island.exe, as their
# licences ask (MIT, Apache and the others require the notice to travel with the program).
# The texts are read from the crates' own folders, so run `cargo build` once first.
#
# Run from anywhere:  powershell -ExecutionPolicy Bypass -File scripts\third-party.ps1
# scripts\package.ps1 runs it too, so the file is never older than the dependency list.

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$sources = @(Get-ChildItem (Join-Path $cargoHome 'registry\src') -Directory)
if ($sources.Count -eq 0) { throw "No crates found in $cargoHome\registry\src. Run cargo build first." }

$tree = cargo tree -e normal --target x86_64-pc-windows-msvc --prefix none -f '{p}|{l}|{r}'
if ($LASTEXITCODE -ne 0) { throw 'cargo tree failed' }

# One entry per crate and version.
$crates = @{}
foreach ($line in $tree) {
    $parts = $line -split '\|', 3
    if ($parts.Count -lt 3 -or $parts[0] -notmatch '^(?<name>\S+) v(?<version>\S+)') { continue }
    if ($Matches.name -eq 'sky-island') { continue }
    $crates["$($Matches.name) $($Matches.version)"] = [pscustomobject]@{
        Name    = $Matches.name
        Version = $Matches.version
        License = $parts[1].Trim()
        Repo    = ($parts[2] -replace '\s*\(\*\)\s*$', '').Trim()
    }
}

$missing = @()
$groups = [ordered]@{}   # licence text -> the crates that carry it
foreach ($crate in ($crates.Values | Sort-Object Name, Version)) {
    $folder = $sources | ForEach-Object { Join-Path $_.FullName "$($crate.Name)-$($crate.Version)" } | Where-Object { Test-Path $_ } | Select-Object -First 1
    $files = @()
    if ($folder) {
        $files += Get-ChildItem $folder -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)' }
        foreach ($dir in (Get-ChildItem $folder -Directory | Where-Object { $_.Name -match '^LICEN[CS]ES?$' })) {
            $files += Get-ChildItem $dir.FullName -File
        }
    }
    if ($files.Count -eq 0) { $missing += "$($crate.Name) $($crate.Version)"; continue }
    foreach ($file in $files) {
        $text = ([IO.File]::ReadAllText($file.FullName, [Text.Encoding]::UTF8) -replace "`r`n", "`n").Trim()
        if ($text.Length -eq 0) { continue }
        if (-not $groups.Contains($text)) { $groups[$text] = New-Object System.Collections.Generic.List[string] }
        $label = "$($crate.Name) $($crate.Version)"
        if (-not $groups[$text].Contains($label)) { $groups[$text].Add($label) }
    }
}

$out = New-Object System.Text.StringBuilder
[void]$out.AppendLine('Sky Island and its third-party software')
[void]$out.AppendLine('======================================')
[void]$out.AppendLine('')
[void]$out.AppendLine('Sky Island itself is released under the MIT License (see the file LICENSE).')
[void]$out.AppendLine('')
[void]$out.AppendLine('It is built with the open-source Rust crates listed below. Each one is used under the')
[void]$out.AppendLine('licence named next to it (where a crate offers a choice, any of the named licences may be')
[void]$out.AppendLine('used), and the licence texts follow the list. Sky Island does not bundle a font: it draws')
[void]$out.AppendLine('with a font that is already installed on the computer.')
[void]$out.AppendLine('')
[void]$out.AppendLine("Crates ($($crates.Count))")
[void]$out.AppendLine('------')
foreach ($crate in ($crates.Values | Sort-Object Name, Version)) {
    $repo = if ($crate.Repo) { "  $($crate.Repo)" } else { '' }
    [void]$out.AppendLine("$($crate.Name) $($crate.Version)  [$($crate.License)]$repo")
}
[void]$out.AppendLine('')
[void]$out.AppendLine('Licence texts')
[void]$out.AppendLine('-------------')
foreach ($text in $groups.Keys) {
    [void]$out.AppendLine('')
    [void]$out.AppendLine('================================================================================')
    [void]$out.AppendLine('Used by: ' + ($groups[$text] -join ', '))
    [void]$out.AppendLine('================================================================================')
    [void]$out.AppendLine('')
    [void]$out.AppendLine($text)
}

[IO.File]::WriteAllText((Join-Path (Get-Location) 'THIRD-PARTY-LICENSES.txt'), ($out.ToString() -replace "`r?`n", "`r`n"), (New-Object System.Text.UTF8Encoding($false)))
Write-Host "Wrote THIRD-PARTY-LICENSES.txt: $($crates.Count) crates, $($groups.Count) distinct licence texts."
if ($missing.Count -gt 0) {
    Write-Warning ("No licence file in the package of: " + ($missing -join ', ') + '. Their licence is named in the list, but add the text by hand.')
}
