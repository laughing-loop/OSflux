<#
.SYNOPSIS
Hardware test for the drive contents check and the short confirmation.

.DESCRIPTION
THIS ERASES THE USB STICK IT SELECTS.

It looks for exactly one USB disk whose name contains -Model, refuses a boot
or system disk or anything larger than -MaxSizeGB, checks that OSflux sees
the same drive, shows what it found and waits for you to type yes. Then:

  1. Wipes the stick (diskpart clean): OSflux must call it blank and eligible.
  2. Formats one FAT32 partition: eligible, one empty FAT32 partition.
  3. Runs --intent: the identity code must be offered.
  4. Copies a file onto the stick: a write confirmed with the code must be
     refused, naming the file.
  5. Deletes the file and runs --intent again: a write confirmed with the
     code must succeed and verify.
  6. Reads the audit log: it must record which confirmation was used.

Run it from an Administrator PowerShell in the OSflux folder:

  pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-drive-contents.ps1

Every OSflux and diskpart output is saved to a log in %TEMP%.
#>
#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    # Part of the stick's name as Windows shows it.
    [string]$Model = 'UDisk',
    # Image for step 5. By default a 4 MiB test image is made in %TEMP%.
    [string]$Image = '',
    [int]$MaxSizeGB = 64
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$script:Root = Split-Path -Parent $PSScriptRoot
$script:Exe = Join-Path $script:Root 'target\debug\osflux.exe'
$script:Log = Join-Path $env:TEMP 'osflux-drive-contents-test.log'
$script:Results = New-Object System.Collections.Generic.List[object]
$testFileName = 'Holiday Photo.txt'

function Write-Log([string]$Text) {
    Add-Content -Path $script:Log -Value $Text
}

# Runs osflux and returns its exit code, stdout and stderr.
function Invoke-OSflux([string[]]$Arguments) {
    $errorFile = New-TemporaryFile
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $stdout = & $script:Exe @Arguments 2> $errorFile.FullName | Out-String
        $code = $LASTEXITCODE
        $stderr = Get-Content -Raw -Path $errorFile.FullName
    }
    finally {
        $ErrorActionPreference = $previous
        Remove-Item -Path $errorFile.FullName -ErrorAction SilentlyContinue
    }
    if ($null -eq $stderr) { $stderr = '' }
    Write-Log ("> osflux {0}`n{1}{2}(exit {3})`n" -f ($Arguments -join ' '), $stdout, $stderr, $code)
    [pscustomobject]@{ Code = $code; Stdout = $stdout; Stderr = $stderr; Text = $stdout + $stderr }
}

function Get-Assessment([int]$Number) {
    $result = Invoke-OSflux @('--assess-target', "$Number", '--json')
    $result.Stdout | ConvertFrom-Json
}

function Get-Intent([int]$Number) {
    $result = Invoke-OSflux @('--intent', '--image', $Image, '--target', "$Number", '--json')
    $result.Stdout | ConvertFrom-Json
}

function Invoke-DiskPart([string[]]$Commands) {
    $file = New-TemporaryFile
    try {
        Set-Content -Path $file.FullName -Value $Commands -Encoding ASCII
        $output = & diskpart.exe /s $file.FullName | Out-String
        $code = $LASTEXITCODE
    }
    finally {
        Remove-Item -Path $file.FullName -ErrorAction SilentlyContinue
    }
    Write-Log ("> diskpart: {0}`n{1}(exit {2})`n" -f ($Commands -join '; '), $output, $code)
    if ($code -ne 0) {
        throw "diskpart failed (exit $code):`n$output"
    }
    $output
}

function Get-StickLetter([int]$Number) {
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Update-HostStorageCache -ErrorAction SilentlyContinue
        $partition = Get-Partition -DiskNumber $Number -ErrorAction SilentlyContinue |
            Where-Object { "$($_.DriveLetter)" -match '^[A-Za-z]$' } |
            Select-Object -First 1
        if ($partition) { return "$($partition.DriveLetter)" }
        Start-Sleep -Seconds 1
    }
    throw "The new partition on disk $Number did not get a drive letter."
}

function Check([string]$Name, [bool]$Passed, [string]$Detail) {
    $result = $(if ($Passed) { 'PASS' } else { 'FAIL' })
    $script:Results.Add([pscustomobject]@{ Result = $result; Test = $Name; Detail = $Detail })
    $color = $(if ($Passed) { 'Green' } else { 'Red' })
    Write-Host ("{0}  {1}" -f $result, $Name) -ForegroundColor $color
    Write-Host ("      {0}" -f $Detail)
}

Set-Content -Path $script:Log -Value ("OSflux drive contents test, {0}`n" -f (Get-Date -Format o))

# --- Build OSflux -----------------------------------------------------------

Push-Location $script:Root
try {
    Write-Host 'Building OSflux...'
    & cargo build --quiet
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed.' }
}
finally {
    Pop-Location
}

if (-not $Image) {
    $Image = Join-Path $env:TEMP 'osflux-short-confirm-test.img'
    # A raw disk image: a boot signature at the end of the first sector and
    # a byte in every 4 KiB block, so verification compares real data.
    $bytes = New-Object byte[] (4MB)
    $bytes[510] = 0x55
    $bytes[511] = 0xAA
    for ($offset = 4096; $offset -lt $bytes.Length; $offset += 4096) {
        $bytes[$offset] = [byte](($offset / 4096) % 251)
    }
    [System.IO.File]::WriteAllBytes($Image, $bytes)
}

# --- Find the stick, and make sure it is the only match -----------------------

Update-HostStorageCache -ErrorAction SilentlyContinue
$disks = @(Get-Disk | Where-Object { $_.BusType -eq 'USB' -and $_.FriendlyName -like "*$Model*" })
if ($disks.Count -ne 1) {
    throw "Expected exactly one USB disk named like '$Model', found $($disks.Count). Plug in only the test stick, or pass -Model."
}
$disk = $disks[0]
$number = [int]$disk.Number
if ($disk.IsBoot -or $disk.IsSystem) {
    throw "Disk $number is a boot or system disk. Stopping."
}
if ($disk.Size -gt ([int64]$MaxSizeGB * 1GB)) {
    throw "Disk $number holds more than $MaxSizeGB GB, which is not a test stick. Stopping."
}

$before = Get-Assessment $number
if ("$($before.model)" -notlike "*$Model*") {
    throw "OSflux sees '$($before.model)' at drive $number, not the '$Model' stick. Stopping."
}
if ($before.status -eq 'blocked') {
    throw "OSflux blocks drive ${number}: $($before.reason)"
}
$detail = Invoke-DiskPart @("select disk $number", 'detail disk')
if ($detail -notmatch [regex]::Escape($Model)) {
    throw "diskpart's disk $number is not the '$Model' stick. Stopping."
}
$code = "$($before.identityCode)"

Write-Host ''
Write-Host ("This test ERASES disk {0}: {1}, {2:N0} MB, identity code {3}." -f $number, $disk.FriendlyName, ($disk.Size / 1MB), $code) -ForegroundColor Yellow
Write-Host "Its current contents: $($before.contents.state) ($($before.contents.reason))"
$answer = Read-Host 'Type yes to erase it and run the test'
if ($answer -ne 'yes') {
    Write-Host 'Cancelled. Nothing was changed.'
    exit 1
}
Write-Host ''

# --- 1. Wiped stick is blank ---------------------------------------------------

Invoke-DiskPart @("select disk $number", 'clean') | Out-Null
Start-Sleep -Seconds 2
$wiped = Get-Assessment $number
Check '1. A wiped stick is blank and eligible' `
    ($wiped.status -eq 'eligible' -and $wiped.contents.state -eq 'blank') `
    "status $($wiped.status), contents $($wiped.contents.state): $($wiped.contents.reason)"

# --- 2. One empty FAT32 partition --------------------------------------------

Invoke-DiskPart @("select disk $number", 'create partition primary', 'format fs=fat32 quick label=TESTSTICK', 'assign') | Out-Null
$letter = Get-StickLetter $number
Start-Sleep -Seconds 2
$formatted = Get-Assessment $number
$partitions = @($formatted.contents.partitions)
Check '2. A freshly formatted stick has one empty FAT32 partition and is eligible' `
    ($formatted.status -eq 'eligible' -and $formatted.contents.state -eq 'empty-filesystem' -and
        $partitions.Count -eq 1 -and $partitions[0].filesystem -eq 'FAT32' -and $partitions[0].label -eq 'TESTSTICK') `
    "status $($formatted.status), contents $($formatted.contents.state): $($formatted.contents.reason); drive ${letter}:"

# --- 3. Intent offers the identity code --------------------------------------

$intent = Get-Intent $number
Check '3. --intent offers the identity code for the empty stick' `
    ($intent.shortConfirmation -eq $code -and $intent.confirmationLevel -eq 'short') `
    "confirmationLevel $($intent.confirmationLevel), shortConfirmation $($intent.shortConfirmation)"

# --- 4. A file on the stick: the code is refused -----------------------------

$testFile = "${letter}:\$testFileName"
Set-Content -Path $testFile -Value 'OSflux test file'
Write-VolumeCache -DriveLetter $letter -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
$refused = Invoke-OSflux @('--write', '--image', $Image, '--target', "$number", '--confirm', $code, '--execute', '--yes')
$firstLine = ($refused.Stderr -split "`n" | Where-Object { $_ -match 'identity code' } | Select-Object -First 1)
Check '4. With a file on the stick, a write confirmed with the code is refused' `
    ($refused.Code -eq 3 -and $refused.Text -match [regex]::Escape('Holiday Photo')) `
    "exit $($refused.Code): $firstLine"

# --- 5. File deleted: the code works ------------------------------------------

Remove-Item -Path $testFile
Write-VolumeCache -DriveLetter $letter -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2
$again = Get-Intent $number
$written = Invoke-OSflux @('--write', '--image', $Image, '--target', "$number", '--confirm', $code, '--execute', '--yes')
Check '5. After deleting the file, a write confirmed with the code succeeds and verifies' `
    ($again.shortConfirmation -eq $code -and $written.Code -eq 0 -and
        $written.Stdout -match 'confirmed with: the identity code' -and $written.Stdout -match 'status: written') `
    "intent shortConfirmation $($again.shortConfirmation); write exit $($written.Code)"

# --- 6. Audit log -----------------------------------------------------------

$stateDir = $(if ($env:OSFLUX_STATE_DIR) { $env:OSFLUX_STATE_DIR } else { Join-Path $env:LOCALAPPDATA 'OSflux' })
$events = @(Get-Content -Path (Join-Path $stateDir 'audit.log') -Tail 40 | ForEach-Object { $_ | ConvertFrom-Json })
$authorized = $events | Where-Object { $_.event -eq 'authorized' } | Select-Object -Last 1
$refusedEvent = $events | Where-Object { $_.event -in @('write-refused', 'write-failed') -and "$($_.detail)" -match 'Holiday Photo' } | Select-Object -Last 1
$started = $events | Where-Object { $_.event -eq 'write-started' } | Select-Object -Last 1
$succeeded = $events | Where-Object { $_.event -eq 'write-succeeded' } | Select-Object -Last 1
Check '6. The audit log records the short confirmation, the refusal and the write' `
    ("$($authorized.detail)" -match 'identity code' -and $null -ne $refusedEvent -and
        "$($started.detail)" -match 'identity code' -and $null -ne $succeeded -and
        $succeeded.sessionId -eq $started.sessionId -and $succeeded.targetIdentityCode -eq $code) `
    "write-started detail: $($started.detail)"

# --- Summary ------------------------------------------------------------------

$failed = @($script:Results | Where-Object { $_.Result -ne 'PASS' }).Count
Write-Host ''
Write-Host ("{0} of {1} checks passed. Full output: {2}" -f ($script:Results.Count - $failed), $script:Results.Count, $script:Log)
Write-Host 'The stick now holds the small test image. If Windows offers to format it, you can click Cancel.'
if ($failed -gt 0) { exit 1 }
exit 0
