# Reads OSflux's version from Cargo.toml and, for a tag build, checks that
# the tag names the same version. Writes `version=<x.y.z>` to the step's
# outputs when run in GitHub Actions.
$ErrorActionPreference = 'Stop'

$metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
$version = ($metadata.packages | Where-Object name -eq 'osflux-cli').version
if (-not $version) { throw 'could not find the osflux-cli package version' }

if ($env:GITHUB_REF_TYPE -eq 'tag' -and $env:GITHUB_REF_NAME -ne "v$version") {
    throw "the tag $($env:GITHUB_REF_NAME) does not match version $version in Cargo.toml; tag the release v$version or change the version"
}

"OSflux version $version"
if ($env:GITHUB_OUTPUT) {
    "version=$version" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
}
