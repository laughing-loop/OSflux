<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/svg/osflux-lockup-white.svg">
    <img src="assets/brand/svg/osflux-lockup-blue.svg" alt="OSflux" width="360">
  </picture>
</h1>

OSflux is a next-generation bootable media and OS deployment platform.

## Platform Layout

- OSflux Core: low-level device access, Windows I/O bindings, and write pipeline primitives.
- OSflux Engine: orchestration layer for image planning, write execution, and verification.
- OSflux CLI: command-line interface for local development and operator workflows.
- OSflux Studio: the desktop app, built with Slint on the same engine. In progress: choosing an image, choosing a drive, the safety check, Windows options and confirming the drive run on the engine; writing comes next.

## Current Status

The repository is currently a Rust workspace with the first platform crates in place:

- `crates/osflux-core`
- `crates/osflux-engine`
- `crates/osflux-cli`
- `crates/osflux-studio`

For a beginner-friendly explanation of the project direction, read:

- `DOCUMENTS/PROJECT_OVERVIEW.md`

For continuous integration, releases and code signing, read:

- `DOCUMENTS/RELEASING.md`

## Development

To contribute, read [CONTRIBUTING.md](CONTRIBUTING.md).

Requirements:

- Rust, through rustup. The version is pinned in `rust-toolchain.toml`, and rustup installs it the first time `cargo` runs here.
- Visual Studio C++ build tools
- Windows SDK

The program is `osflux` (`osflux.exe` on Windows). From the source folder, `cargo run --` runs it, and `cargo build --release` puts it in `target\release\osflux.exe`.

Run the CLI:

```powershell
cargo run
cargo run -- --version
```

Run OSflux Studio, or preview a view. The views are `shell`, `settings`, `about`, `licences` and `gallery` (the component sheet), plus the steps: `image`, `drives`, `drives-none`, `safety`, `safety-blocked`, `windows-options`, `windows-options-set`, `confirm-code`, `confirm-code-wrong`, `confirm-code-match`, `confirm-files`, `confirm-files-ticked`, `confirm-phrase` and `confirm-expired`. The steps read the image given with `--image` through the real engine; the drives in a preview are samples, so none is read or written.

```powershell
cargo run -p osflux-studio
cargo run -p osflux-studio --features preview -- --preview gallery --theme dark
cargo run -p osflux-studio --features preview -- --preview safety --image D:\Images\Win11.iso
```

On Linux, building Studio needs fontconfig's development files (`sudo apt install libfontconfig-dev` on Debian and Ubuntu). Studio's design reference, the screen frames and `osflux-studio-ui-tokens.md`, is in `DOCUMENTS/studio/`. On Windows, `osflux-studio.exe` asks for Administrator when it opens. On Linux, Browse uses zenity or kdialog, and dropping files onto the window is not supported yet. Studio itself is distributed under GPL-3.0 only, because it includes Slint, which is licensed that way.

Every pull request is checked on Windows and Linux: formatting, lints as errors, all tests, the Windows build, and that every dependency's licence is compatible with GPL-3.0. `DOCUMENTS/RELEASING.md` explains CI, how to cut a release, and code signing. Notable changes are listed in `CHANGELOG.md`.

List visible physical drives:

```powershell
cargo run -- --list-devices
```

Assess one target drive without writing:

```powershell
cargo run -- --assess-target 2
```

Print the target assessment as JSON:

```powershell
cargo run -- --assess-target 2 --json
```

Run a read-only write preflight for an image and target:

```powershell
cargo run -- --preflight --image "C:\path\to\image.iso" --target 2
```

Print the preflight as JSON:

```powershell
cargo run -- --preflight --image "C:\path\to\image.iso" --target 2 --json
```

Create a read-only write intent and confirmation phrase:

```powershell
cargo run -- --intent --image "C:\path\to\image.iso" --target 2
```

Print the write intent as JSON:

```powershell
cargo run -- --intent --image "C:\path\to\image.iso" --target 2 --json
```

Validate a confirmed write command as a dry-run skeleton:

```powershell
cargo run -- --write --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

Print the write command plan as JSON:

```powershell
cargo run -- --write --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --json
```

Write a raw disk image, a hybrid ISO, or a Windows installer ISO to a USB stick. **This erases the stick.** Run it from an Administrator PowerShell, after `--intent` has printed the phrase for that stick. The same command works for every kind of image:

```powershell
cargo run -- --write --image "C:\path\to\image.img" --target 3 --confirm "OSFLUX ERASE TARGET 3 7F3A" --execute
```

Without Administrator rights OSflux stops before the prompt, leaves the confirmation unused, and exits with code 3. Before erasing, OSflux shows the drive's model, size, and path and asks you to type `yes`; anything else cancels without writing. Scripts and automation can pass `--yes` to skip the prompt. Without a terminal and without `--yes`, OSflux refuses rather than assume the answer.

When the write finishes, OSflux prints the SHA-256 of the image file, so you can compare it with the checksum the image's publisher lists. For a Windows ISO it is the ISO's own hash, read before its files are copied.

Every confirmation and write attempt is appended to an audit log, `audit.log` in the OSflux state folder, as one JSON object per line (JSON Lines). Each entry records the time, the Windows user and computer name, the image path, size, and SHA-256, the target drive's path, model, and identity code, and the event: `authorized`, `write-refused`, `write-cancelled`, `write-started`, `write-succeeded`, or `write-failed`. If the `write-started` entry cannot be saved, OSflux does not touch the drive.

Prepare a locked dry-run write session:

```powershell
cargo run -- --session --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

Print the write session as JSON:

```powershell
cargo run -- --session --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --json
```

Prepare a dry-run chunk plan:

```powershell
cargo run -- --chunk-plan --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

Use a custom chunk size:

```powershell
cargo run -- --chunk-plan --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --chunk-size 4194304 --json
```

Preview progress events from the chunk plan:

```powershell
cargo run -- --progress-preview --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

Print the progress preview as JSON:

```powershell
cargo run -- --progress-preview --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --chunk-size 4194304 --json
```

Select the read-only write mode:

```powershell
cargo run -- --select-mode --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

Print the write mode selection as JSON:

```powershell
cargo run -- --select-mode --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --chunk-size 4194304 --json
```

Plan an image without writing anything:

```powershell
cargo run -- --plan-image "C:\path\to\image.iso"
```

Print the same plan as JSON:

```powershell
cargo run -- --plan-image "C:\path\to\image.iso" --json
```

Detect the image family without writing anything:

```powershell
cargo run -- --detect-family --image "C:\path\to\image.iso"
```

Print the image family report as JSON:

```powershell
cargo run -- --detect-family --image "C:\path\to\image.iso" --json
```

Build a read-only ISO filesystem plan:

```powershell
cargo run -- --filesystem-plan --image "C:\path\to\image.iso"
```

Print the ISO filesystem plan as JSON:

```powershell
cargo run -- --filesystem-plan --image "C:\path\to\image.iso" --json
```

The planner reads known file signatures first, then falls back to the extension when no signature is found.
For ISO images, it also checks for an El Torito boot catalog and reports boot metadata, including BIOS, UEFI, or hybrid boot style when present. It validates each boot entry's byte range so OSflux can reject unsafe or incomplete image plans before write support is enabled.
OSflux also reads ISO directory records from the Primary Volume Descriptor and reports boot-related paths such as boot managers, UEFI loaders, installer images, Linux kernels, initrd files, live filesystems, and GRUB paths when they are present.
The planner reports the selected write strategy, such as ISO filesystem boot setup, deeper ISO inspection, or raw block writing.
Every image plan has a status: ready, needs-inspection, or blocked.
The engine builds a structured plan report so the CLI and future Studio UI can render the same planning result without duplicating logic.
The image family command is still read-only. It scans the early image bytes for family markers and classifies images as Windows installer, Linux installer, multi-family image, raw disk image, or unknown.
The filesystem plan command is still read-only. It combines image family evidence, ISO boot catalog validity, and directory boot assets into one filesystem-aware plan.
Target assessment only accepts drives that report removable media, such as USB sticks and SD cards. External hard disks and SSDs are always blocked, even when they connect over USB, because they usually hold data. Some high-end USB sticks report themselves as fixed disks and are blocked for the same reason.
For a drive that passes those checks, OSflux also reads what it holds, without changing anything and without locking its volumes. `--assess-target` and `--list-devices` show each partition's filesystem, label, size, and used space. A drive is `eligible` when it is blank (its first and last MiB are uniformly empty, apart from at most an empty MBR such as `diskpart clean` leaves) or holds exactly one FAT32 or exFAT partition whose root folder is empty apart from the volume label and a `System Volume Information` folder. Deleted files do not count; anything else in the root folder, a second partition, an extended partition, NTFS or FAT16, a damaged partition table, or a filesystem written without a partition table means `needs-confirmation`. Reading a drive needs Administrator rights, so without them every drive needs confirmation.
The preflight blocks any image that is larger than the target drive, so an oversized image never reaches the confirmation step.
The write intent command never touches a drive. It creates the confirmation phrase that `--write --execute` requires, and records a one-time confirmation for that drive and that image. For an `eligible` drive it also offers a short confirmation: the drive's four-character identity code, such as `--confirm 2E85`. The full phrase is always accepted too.
The phrase ends with the target drive's identity code, built from its model, serial number, and Windows device ID. `7F3A` in these examples is only a placeholder: run `--intent` to get the real phrase for your drive. If a different drive takes the same index, the code changes and the old phrase stops matching.
The recorded confirmation works for one write, within 10 minutes, with the same image file. Running `--write --execute` a second time, starting more than 10 minutes later, or after replacing or editing the image, is refused until `--intent` is run again. Confirmations are kept in `%LOCALAPPDATA%\OSflux\authorizations` on Windows, next to the audit log (`$XDG_STATE_HOME/osflux` or `~/.local/state/osflux` elsewhere), or under the folder named by the `OSFLUX_STATE_DIR` environment variable when it is set.
The write command skeleton is also read-only. It checks the exact confirmation phrase and reports whether a future write session would be allowed to proceed, while keeping `writeAccessAttempted` set to `false`.
Adding `--execute` to `--write` is the only way OSflux writes to a drive. It only writes raw disk images, hybrid ISOs and UDF ISOs such as Windows installers, only to a removable target, and only with the exact phrase from `--intent` while its confirmation is still valid. It re-runs the safety chain and re-checks the drive, uses up the confirmation, then opens the drive, which locks and dismounts the drive's volumes. A write confirmed with the identity code needs the drive to be empty both when `--intent` ran and when the write starts, and OSflux checks once more after locking the drive, before writing anything; if anything was added in between, it refuses and asks for the full phrase. It refuses to continue if the opened drive's size does not match the confirmed one, writes the image in 4 MiB chunks, reads every byte back to verify it, and then releases the drive. Locking a volume makes Windows ask the programs holding it to let go; File Explorer, antivirus scanners and indexers do, and OSflux retries for five seconds while they finish. If a program still keeps the drive open, as some backup, sync and accounting services do, OSflux asks before forcing a dismount, the way `chkdsk` and `format` do: programs using the drive lose access to it and anything unsaved on it is lost, although the drive is about to be erased anyway. Answering anything but yes stops with nothing written. With `--yes`, OSflux never forces a dismount unless `--force-dismount` is given too, and the audit log records every forced dismount. `--no-verify` skips the read-back, which roughly halves the time on a slow stick, but a faulty or fake-capacity drive then goes unnoticed; the write plan says so, and the audit log records that the write was not verified.
A hybrid ISO also carries a partition table in its first sector, which is how most Linux installers (Ubuntu, Fedora, Debian, Arch, Mint) are built. OSflux copies those byte for byte, the same way it writes a `.img`, and `--plan-image` reports them with `hybrid: yes` and the `hybrid-iso-raw-write` strategy. Windows installer ISOs are not hybrid: their files live in a UDF filesystem, which OSflux reads. OSflux copies those files onto a new MBR partition table with one FAT32 partition, from 1 MiB to the end of the stick, labelled from the ISO (cut to 11 characters, such as `CCCOMA_X64F`), and starts it in UEFI mode only: a PC that tries the legacy BIOS entry is told to choose the `UEFI:` entry instead, so Windows is never installed in legacy mode by accident. Microsoft's signed boot files are copied unchanged, so Secure Boot keeps working. The stick image is generated as it is written, nothing is extracted to a temporary folder, only the space the files need is written, and the last MiB of the stick is cleared so an old backup partition table cannot linger. Every written byte is read back and compared. The write plan shows the label, the number and size of the files, and the largest file. Use a stick of 8 GB or more for Windows.

FAT32 cannot hold a file of 4 GiB or more, and the `sources/install.wim` of most Windows 11 ISOs is larger. OSflux writes such an `install.wim` as split parts, `install.swm`, `install2.swm` and so on, of at most 3800 MiB each, the same form `DISM /Split-Image` makes and Windows Setup reads in place of `install.wim`. The parts are generated from the ISO as the stick is written, so nothing is extracted to a temporary folder. The edition metadata stays in the first part, and every other resource is copied unchanged into exactly one part, so no data is recompressed. `--plan-image` shows the split as `fits FAT32: yes, after splitting sources/install.wim (5.12 GiB) into 2 parts`, and the write plan lists the part names. Two cases are refused before anything is written: an `install.esd` or other image with solid compression, which cannot be split this way (convert it to a WIM first, for example with `DISM /Export-Image`), and any other file of 4 GiB or more. If a stick made this way ever fails to start Setup, the fallback is to split the image yourself with `DISM /Split-Image /ImageFile:install.wim /SWMFile:install.swm /FileSize:3800` and copy the parts onto a FAT32 stick.

### Windows Setup options

A Windows stick is a stock Microsoft installer unless you ask for more. With `--write --execute` and a Windows installer ISO, these options add settings to an answer file, `autounattend.xml`, at the root of the stick. Windows Setup reads it by itself on any PC started from that stick. Each option is off unless given:

| Option | What Windows Setup does |
|---|---|
| `--bypass-requirements` | Skips its TPM 2.0, Secure Boot and 4 GB memory checks, for PCs that fail them. Microsoft does not support Windows 11 on such PCs. |
| `--local-account <NAME>` | Creates this local administrator account instead of asking for a Microsoft account, and lets the first-start screens go on without a network. |
| `--skip-privacy-questions` | Answers the privacy questions with the choices that send the least data to Microsoft. |
| `--no-auto-bitlocker` | Stops Windows from turning on BitLocker device encryption by itself. BitLocker can still be turned on later. |
| `--copy-region` | Uses this PC's formats, system locale and keyboard instead of asking for them. |

The answer file never holds a password. The local account starts with a blank password, signs in once by itself so Setup can finish, and Windows asks for a new password at the next sign-in; passwords are then set never to expire. Account names are checked before anything is written: at most 20 letters, digits, spaces, periods, hyphens or underscores, and not a name Windows keeps for itself, such as `Administrator`.

The write plan lists every option, the audit log records them, and the answer file is written and read back like every other byte. OSflux refuses the options for raw images and hybrid ISOs, which it copies byte for byte, and for an ISO that already has its own answer file. The answer file only applies when a PC starts from the stick, not when `setup.exe` is run from inside Windows. Microsoft keeps changing how Setup treats some of these settings, so each option is tested with a real install before a release.
The write session command is still read-only. It locks the confirmed image and target details into a session record, assigns a session ID, and keeps write access disabled.
The chunk plan command is still read-only. It calculates image size, chunk size, chunk count, final chunk size, and planned bytes for future progress reporting.
The progress preview command is still read-only. It turns the chunk plan into sampled progress events with integer percentage basis points.
The write mode selection command is still read-only. It chooses the future execution path, such as ISO filesystem boot setup or raw block writing, only after the safety chain has passed.

Every command sets an exit code so scripts can check the result without parsing the output:

- `0`: the command ran and its result passed.
- `1`: the command could not run, for example because the image could not be read.
- `2`: an argument was unknown, missing, or invalid.
- `3`: the command ran, but its result is blocked or needs inspection, the confirmation was refused, the write needs Administrator rights, or the final prompt before erasing was declined. The report is still printed.

A hardware test for the drive contents check and the short confirmation runs on a real USB stick from an Administrator PowerShell. **It erases the stick it selects**, after checking that the stick is the only USB disk matching its name and asking you to type `yes`:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-drive-contents.ps1
```

Run with demo pipeline options:

```powershell
cargo run -- --blocks=16 --block-size=1048576
```

## Licence

Copyright (C) 2026 HOD Technologies Limited

OSflux is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version (SPDX: `GPL-3.0-or-later`).

OSflux is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See [LICENSE](LICENSE) for the full text of the GNU General Public License.

The licence covers OSflux's source code. It does not cover the OSflux name or logo, including the files in `assets/brand/`. HOD Technologies Limited keeps all rights to them, so using them, for example on a modified version, needs its permission. The logo's wordmark is drawn from IBM Plex Sans, under the SIL Open Font License 1.1.

Every dependency's licence is checked for compatibility with GPL-3.0 in CI (`deny.toml`).
