# OSflux Project Overview

This document explains what OSflux does today, how it is put together, and what comes next. It was last brought up to date in November 2026.

## What OSflux Is

OSflux writes operating system images to USB sticks and SD cards, so a PC can start from the stick and install the system. The long-term goal is a bootable media and OS deployment platform: fast, careful with the drives it touches, and built so that every interface uses the same engine.

Today OSflux is a command-line program, `osflux` (`osflux.exe` on Windows), that writes Windows 11 installer ISOs, Linux installer ISOs and raw disk images on Windows. OSflux Studio, the desktop app, is being built on the same engine.

## What OSflux Can Do Today

### Write three kinds of image

| Image | How OSflux writes it |
|---|---|
| Raw disk image (`.img`, `.raw`) | Byte for byte. |
| Hybrid ISO: most Linux installers, such as Ubuntu, Fedora, Debian, Arch and Mint | Byte for byte. The ISO already holds a partition table, so it starts from the stick as it is. |
| Windows installer ISO | OSflux reads the ISO's UDF file tree and copies its files onto a new MBR drive with one FAT32 partition that starts in UEFI mode only. |

For a Windows ISO:
- **No temporary files.** The FAT32 stick image is generated from the ISO as it is written, so nothing is extracted to a temporary folder.
- **Secure Boot keeps working.** Microsoft's signed boot files are copied unchanged.
- **No accidental legacy installs.** A PC that tries the legacy BIOS entry is told to choose the `UEFI:` entry instead, so Windows is never installed in legacy mode by accident.
- **Stick size.** Use a stick of 8 GB or more.

### Split an install.wim that is too big for FAT32

FAT32 cannot hold a file of 4 GiB or more, and the `sources/install.wim` of most Windows 11 ISOs is larger. OSflux writes it as `install.swm`, `install2.swm` and so on, of at most 3800 MiB each.
- **Windows Setup reads the parts** in place of `install.wim`. They are the same form `DISM /Split-Image` makes.
- **Nothing is recompressed or extracted first.** The parts are generated as the stick is written.
- **Refused before anything is written:**
  - an `install.esd` or other image with solid compression, which cannot be split this way;
  - any other file of 4 GiB or more.
- **Fallback.** If a stick made this way ever fails to start Setup, the README shows how to split the image with DISM by hand.

### Change Windows Setup, if asked

A Windows stick is a stock Microsoft installer unless you ask for more. These options add settings to an answer file, `autounattend.xml`, at the root of the stick. Windows Setup reads it by itself on any PC started from that stick. Each option is off unless given:

| Option | What it does |
|---|---|
| `--bypass-requirements` | Skips Windows 11's TPM 2.0, Secure Boot and 4 GB memory checks, for PCs that fail them. Microsoft does not support Windows 11 on such PCs. |
| `--local-account <NAME>` | Creates this local administrator account instead of asking for a Microsoft account. |
| `--skip-privacy-questions` | Answers the privacy questions with the choices that send the least data to Microsoft. |
| `--no-auto-bitlocker` | Stops Windows from turning on BitLocker device encryption by itself. |
| `--copy-region` | Uses this PC's formats, system locale and keyboard. |

The answer file never holds a password. The local account signs in once by itself so Setup can finish, and Windows asks for a new password at the next sign-in.

### Verify every write

- **Read back.** After writing, OSflux reads every byte back and compares it, so a faulty or fake-capacity stick is caught.
- **`--no-verify`** skips the read-back for a quicker write. The write plan warns about it, and the audit log records that the write was not verified.
- **Checksum.** When the write finishes, OSflux prints the SHA-256 of the image, to compare with the one the image's publisher lists.

### Refuse to erase the wrong drive

Writing to a drive erases it, so every write goes through a safety chain:

- **Which drives can be written.** Only drives that report removable media, such as USB sticks and SD cards. OSflux blocks any drive that:
  - holds the system volume;
  - is an external hard disk or SSD;
  - has a size or identity it cannot read;
  - reports itself as a fixed disk.
- **What has to be typed.** That depends on what the drive holds:
  - A blank drive, or one with a single empty FAT32 or exFAT partition, is confirmed with its four-character identity code, such as `7F3A`.
  - Any other allowed drive needs the full phrase, such as `OSFLUX ERASE TARGET 2 7F3A`.
- **The identity code.** It is built from the drive's model, serial number and Windows device ID. If USB sticks are swapped, the code changes and an old confirmation no longer matches.
- **What a confirmation covers.** `--intent` records it, and it covers one write, within 10 minutes, of the same image file.
- **Writing.** `--write --execute` does the following, and only from an Administrator prompt:
  1. Checks the drive again.
  2. Asks you to type `yes`.
  3. Locks and dismounts the drive's volumes. It asks before forcing a dismount, the way `chkdsk` and `format` do.
  4. Writes in 4 MiB chunks.
  5. Verifies, then releases the drive.
- **Audit log.** Every confirmation and write attempt is recorded in `audit.log`, in the OSflux state folder, as one JSON object per line. If OSflux cannot record that a write is starting, it does not touch the drive.

### Look before writing

Each step of the chain can also be run on its own. None of these commands writes to a drive, and each can print JSON with `--json`:

| Command | What it shows |
|---|---|
| `--list-devices` | Every physical drive, with its size, model, interface, and whether OSflux would write to it. |
| `--assess-target <INDEX>` | One drive's verdict: eligible, needs confirmation, or blocked, and why. |
| `--plan-image <PATH>` | What the image is (ISO 9660, UDF, El Torito boot catalog, MBR, GPT), how OSflux would write it, and for a Windows ISO, what its files become on a FAT32 stick. |
| `--detect-family`, `--filesystem-plan` | Whether the image is a Windows installer, a Linux installer, both, or a raw disk image, and the boot setup that follows. |
| `--preflight` | One verdict for an image and drive together: ready, needs confirmation, needs inspection, or blocked. |
| `--intent` | The confirmation for that image and drive. |
| `--write` (without `--execute`), `--session`, `--chunk-plan`, `--progress-preview`, `--select-mode` | The write that would happen, step by step, as a dry run. |

The README lists every command with examples.

## How OSflux Is Put Together

OSflux is a Rust workspace with four crates. The engine makes every decision about safety, and the interfaces only ask it.

| Crate | What it is |
|---|---|
| `crates/osflux-core` | The low-level layer: listing drives, opening them, and reading and writing in large blocks with the Windows I/O APIs. Windows is implemented. The Linux and macOS modules are placeholders, so OSflux cannot list or write drives there yet. |
| `crates/osflux-engine` | The orchestration layer. See below. |
| `crates/osflux-cli` | The command-line program, `osflux`. |
| `crates/osflux-studio` | OSflux Studio, the desktop app. |

The engine's parts:
- image planning, family detection and the ISO 9660 and UDF readers;
- drive assessment, including what a drive holds;
- the confirmation chain;
- the FAT32 builder, the install.wim splitter and the answer file;
- the write and verify path;
- the audit log.

## OSflux Studio (in progress)

Studio is the desktop app. It is built with Slint and calls the engine directly, so it shows the same verdicts as the CLI and never makes a safety decision of its own.

- **Design.** It follows the v2 design in `DOCUMENTS/studio/`: the screen frames, dark and light, and `osflux-studio-ui-tokens.md`. It uses IBM Plex Sans and Mono, square custom components, and the OSflux icons.
- **Administrator.** On Windows it asks for Administrator when it opens, as tools that write drives do.
- **Done so far:**
  - The window: a header with the Administrator status, the step list with "This session", the action bar and the status bar.
  - Dark, light and follow-Windows themes.
  - Settings: the theme, and "Verify after writing", on by default.
  - About, with the licences.
  - **Step 1, Image.** Choose a file with the Open dialog, or drop it onto the window. Dropping works on Windows even though Studio runs as Administrator. Studio shows the engine's image plan and notes, and takes an optional expected SHA-256 from the download page.
  - **Step 2, Drive.** The drives the engine allows, the ones it will not erase with the reason in plain words, and Refresh.
  - **Step 3, Safety check.** The engine's verdict and its checks in order, with the technical details. A blocked verdict offers a way back.
  - Reading images, listing drives and the safety check run in the background, so the window never freezes.
- **Next:**
  - Windows options, the five Setup options above, for Windows ISOs only.
  - Confirm: the drive code or the full phrase, as the engine requires, with the 10-minute limit.
  - Writing and verifying, with speed, time left and Cancel.
  - The Done screens, with whether the hash matched and a link to the audit log.
  - The error screens.
- **On Linux,** Studio builds and runs for development. Browse uses zenity or kdialog, but no drives are listed until the core supports Linux.

## Licence

OSflux is licensed under the GNU General Public License, version 3 or later (`GPL-3.0-or-later`), Copyright (C) 2026 HOD Technologies Limited. The OSflux name and logo are not covered by the licence.

The Studio program is distributed under GPL-3.0 only, because it includes Slint, which is licensed that way.

## Continuous Integration and Releases

Every pull request to `main`, and every push to it, runs `.github/workflows/ci.yml`:

| Job | What it checks |
|---|---|
| Windows | Lints with warnings treated as errors, every test, and release builds of `osflux.exe` and `osflux-studio.exe`, kept as a download for 14 days. |
| Linux | Formatting, lints, every test (with independent FAT32, WIM and XML tools installed, so the cross-checks run), and the Windows build checked from Linux. |
| Dependency licences | That every dependency's licence is compatible with GPL-3.0 (`deny.toml`). |

Rust is pinned in `rust-toolchain.toml`, so CI and every developer machine use the same compiler.

Pushing a version tag, such as `v0.1.0`, runs `.github/workflows/release.yml`:
1. It checks that the tag matches the version in `Cargo.toml`, and runs the tests.
2. It builds `osflux.exe` and signs it, once a code signing certificate is set up. Until then it builds unsigned.
3. It packages a zip with `SHA256SUMS.txt`.
4. It publishes a GitHub release whose notes are the version's section of `CHANGELOG.md`.

No version has been released yet; 0.1.0 will be the first. `DOCUMENTS/RELEASING.md` explains CI, how to cut a release, and the code signing options.

## How To Run It Locally

From the project folder, in PowerShell:

```powershell
cargo run -- --version
cargo run -- --list-devices
cargo run -- --plan-image "C:\path\to\image.iso"
cargo run -p osflux-studio
```

Writing needs an Administrator PowerShell and the confirmation from `--intent`. The README shows the full sequence.

## What Comes Next

- **Studio:** Windows options and Confirm, then writing, verifying, Done and the error screens. Cancel needs the engine's write path to support stopping part-way, which it does not yet.
- **Before the first release:**
  - package `osflux-studio.exe` in the release workflow;
  - make its release build smaller;
  - include the third-party licence notices;
  - set up code signing.
- **Later:** Linux and macOS drive access in the core, so OSflux can write drives there too.

## Development Rule

We build the dangerous parts carefully. Writing to a physical drive erases it, so OSflux:

- lists drives clearly, with their size, model and identity;
- writes only to removable drives, and never to the system drive;
- asks for a confirmation tied to that exact drive and image, which expires;
- checks the drive again just before writing, and reads every byte back afterwards;
- records every attempt in the audit log.

New features keep to these rules. Interfaces ask the engine, and do not repeat or skip its checks.

## Short Version

- **The CLI** writes Windows 11 installers, Linux installers and raw disk images to USB sticks on Windows, and checks every byte.
- **The engine** makes every safety decision. The CLI and Studio only ask it.
- **Studio** is under way: choosing an image, choosing a drive and the safety check work. Confirming and writing come next.
- **Every change** is checked on Windows and Linux. The release workflow is ready for the first version.
