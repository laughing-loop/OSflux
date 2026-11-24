# Packages a release build: dist/osflux-<version>-windows-x64.zip with
# osflux.exe, README.md and CHANGELOG.md; dist/SHA256SUMS.txt; and
# dist/release-notes.md, taken from the version's section of CHANGELOG.md.
param(
    [Parameter(Mandatory)] [string] $Version,
    [switch] $Signed
)
$ErrorActionPreference = 'Stop'

$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$exe = Join-Path $root 'target\release\osflux.exe'
if (-not (Test-Path $exe)) { throw "$exe was not found; build it with cargo build --release -p osflux-cli" }

$dist = Join-Path $root 'dist'
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
$name = "osflux-$Version-windows-x64"
$stage = Join-Path $dist $name
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item $exe, (Join-Path $root 'README.md'), (Join-Path $root 'CHANGELOG.md') $stage
$zip = Join-Path $dist "$name.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
Remove-Item -Recurse -Force $stage

$zipHash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
$exeHash = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLowerInvariant()
"$zipHash  $name.zip`n$exeHash  osflux.exe`n" |
    Set-Content -Path (Join-Path $dist 'SHA256SUMS.txt') -Encoding ascii -NoNewline

# The release notes are the version's CHANGELOG section. A tag build must
# have one; a test build from the Actions tab falls back to Unreleased.
$changelog = Get-Content (Join-Path $root 'CHANGELOG.md')
function Get-Section([string] $Heading) {
    $lines = [System.Collections.Generic.List[string]]::new()
    $inside = $false
    foreach ($line in $changelog) {
        if ($line.StartsWith('## ')) {
            if ($inside) { break }
            $inside = $line.StartsWith("## $Heading")
        } elseif ($inside) {
            $lines.Add($line)
        }
    }
    return ($lines -join "`n").Trim()
}
$notes = Get-Section "[$Version]"
if (-not $notes) {
    if ($env:GITHUB_REF_TYPE -eq 'tag') { throw "CHANGELOG.md has no '## [$Version]' section" }
    $notes = "Test build of commit $($env:GITHUB_SHA), not a release.`n`n" + (Get-Section '[Unreleased]')
}
if ($Signed) {
    $notes += "`n`nosflux.exe is code-signed."
} else {
    $notes += "`n`nThis build is not code-signed yet, so Windows SmartScreen may warn before it runs. Check the download against SHA256SUMS.txt."
}
$notes | Set-Content -Path (Join-Path $dist 'release-notes.md') -Encoding utf8

Get-ChildItem $dist | Format-Table Name, Length -AutoSize
