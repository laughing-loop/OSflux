# Checks that a file carries a valid, timestamped Authenticode signature,
# with signtool from the Windows SDK, and prints who signed it.
param(
    [Parameter(Mandatory)] [string] $Path
)
$ErrorActionPreference = 'Stop'

$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $signtool) { throw 'signtool.exe was not found in the Windows SDK' }

& $signtool.FullName verify /pa /tw /v $Path
if ($LASTEXITCODE -ne 0) { throw "the signature on $Path did not verify" }

$signature = Get-AuthenticodeSignature -FilePath $Path
if ($signature.Status -ne 'Valid') { throw "the signature on $Path is $($signature.Status)" }
if (-not $signature.TimeStamperCertificate) { throw "the signature on $Path has no timestamp, so it would stop being valid when the certificate expires" }
"Signed by: $($signature.SignerCertificate.Subject)"
"Timestamped by: $($signature.TimeStamperCertificate.Subject)"
