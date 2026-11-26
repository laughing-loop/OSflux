# Packages a release build: dist/osflux-<version>-windows-x64.zip with
# osflux.exe, osflux-studio.exe, THIRD-PARTY-NOTICES.html, the IBM Plex font
# licence, LICENSE, README.md and CHANGELOG.md; dist/SHA256SUMS.txt; and
# dist/release-notes.md, taken from the version's section of CHANGELOG.md.
param(
    [Parameter(Mandatory)] [string] $Version,
    [switch] $Signed
)
$ErrorActionPreference = 'Stop'

$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
# Built with the dist profile (see Cargo.toml).
$exe = Join-Path $root 'target\dist\osflux.exe'
$studio = Join-Path $root 'target\dist\osflux-studio.exe'
$notices = Join-Path $root 'target\dist\THIRD-PARTY-NOTICES.html'
foreach ($file in $exe, $studio) {
    if (-not (Test-Path $file)) { throw "$file was not found; build it with cargo build --profile dist -p osflux-cli -p osflux-studio" }
}
# Code signing (SignPath) needs every program in a release to name the
# project and carry the release's version.
foreach ($file in $exe, $studio) {
    $info = (Get-Item $file).VersionInfo
    if ($info.ProductName -ne 'OSflux') { throw "$file names the product '$($info.ProductName)', not 'OSflux'" }
    if ($info.ProductVersion -ne $Version -or $info.FileVersion -ne $Version) {
        throw "$file carries version $($info.ProductVersion) / $($info.FileVersion), not $Version"
    }
}
if (-not (Test-Path $notices)) { throw "$notices was not found; write it with cargo about generate (see about.toml)" }

$dist = Join-Path $root 'dist'
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
$name = "osflux-$Version-windows-x64"
$stage = Join-Path $dist $name
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item $exe, $studio, $notices, (Join-Path $root 'LICENSE'), (Join-Path $root 'README.md'), (Join-Path $root 'CHANGELOG.md') $stage
Copy-Item (Join-Path $root 'crates\osflux-studio\ui\fonts\OFL.txt') (Join-Path $stage 'IBM-PLEX-OFL.txt')
$zip = Join-Path $dist "$name.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
Remove-Item -Recurse -Force $stage

$zipHash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
$exeHash = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLowerInvariant()
$studioHash = (Get-FileHash $studio -Algorithm SHA256).Hash.ToLowerInvariant()
"$zipHash  $name.zip`n$exeHash  osflux.exe`n$studioHash  osflux-studio.exe`n" |
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
$notes += "`n`nOSflux is free software under the GNU General Public License, version 3 or later (OSflux Studio under version 3 only, because it includes Slint); see LICENSE in the zip, and THIRD-PARTY-NOTICES.html for the licences of the crates built into it. Its source code is in this release's Source code archives (commit $($env:GITHUB_SHA))."
if ($Signed) {
    $notes += "`n`nosflux.exe and osflux-studio.exe are code-signed."
} else {
    $notes += "`n`nThis build is not code-signed yet, so Windows SmartScreen may say it protected your PC. Choose More info, then Run anyway. Check the download against SHA256SUMS.txt first."
}
$notes | Set-Content -Path (Join-Path $dist 'release-notes.md') -Encoding utf8

Get-ChildItem $dist | Format-Table Name, Length -AutoSize
