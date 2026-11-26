# Changelog

All notable changes to OSflux are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Until 1.0.0, a
minor version may change command-line options or output.

## [Unreleased]

The first release, 0.1.0, will contain everything below.

### Added

- **Drive listing.** `--list-devices` shows each physical drive with its size, model, serial number and interface. It says whether a drive is a possible target, or blocked because it holds Windows, is internal, or is an external disk rather than removable media.
- **Image planning.** `--plan-image` reads an image's signatures, including ISO 9660, UDF, El Torito boot catalogs, MBR and GPT. It then picks how to write it, and shows what a Windows ISO's files would become on a FAT32 stick. Text or JSON.
- **Write safety chain.**
  - `--intent` records a one-time confirmation, tied to the drive's identity code, the image and a 10-minute limit.
  - `--write --execute` re-checks the drive, asks a final yes, locks and dismounts the drive's volumes, writes in 4 MiB chunks, and reads every byte back to verify it.
  - Each write is recorded in a JSON Lines audit log.
- **Raw images and hybrid ISOs** (most Linux installers) are written byte for byte.
- **Drive contents check.** A blank drive, or one holding a single empty FAT32 or exFAT partition, can be confirmed with its four-character identity code instead of the full phrase. This is checked again before writing.
- **Windows installer ISOs.**
  - OSflux reads the ISO's UDF file tree and writes its files onto a new MBR drive with one FAT32 partition that starts in UEFI mode. The image is generated as it is written.
  - An `install.wim` over 4 GiB is written as `install.swm`, `install2.swm` and so on, the form DISM makes and Windows Setup reads.
- **Windows Setup options**, written as an `autounattend.xml` answer file:
  - `--bypass-requirements`
  - `--local-account <NAME>`, which never writes a password
  - `--skip-privacy-questions`
  - `--no-auto-bitlocker`
  - `--copy-region`
- **`--no-verify`** skips the read-back for a quicker write, with a warning in the plan and a note in the audit log.
- **A drive another program keeps open** can be freed with a forced dismount, the way `chkdsk` and `format` do. OSflux asks first. With `--yes`, it forces only when `--force-dismount` is also given.
- **The Windows program** has the OSflux icon and version details. `--version` prints the version.
- **Continuous integration** on Windows and Linux: formatting, lints as errors, tests, and the Windows build.
- **A release workflow** that packages `osflux.exe` with checksums and signs it once a code signing certificate is set up.
- **Licence.** OSflux is licensed under the GNU General Public License, version 3 or later (`GPL-3.0-or-later`), Copyright (C) 2026 HOD Technologies Limited. The OSflux name and logo are not covered by the licence. CI checks that every dependency's licence is compatible with GPL-3.0.

- **OSflux Studio, in progress** (`osflux-studio.exe`): the desktop app, built with Slint on the same engine, to the v2 design in `DOCUMENTS/studio/`.
  - The window shell (header with Administrator status, step rail with "This session", action bar and status bar), dark, light and follow-Windows themes, Settings (theme, verify after writing) and About with the licences.
  - The steps run on the engine:
    - **Image:** choose a file with the Open dialog, or drop it onto the window. Dropping works on Windows even though Studio runs as Administrator. Studio shows the engine's image plan, the notes that go with it, and an optional expected SHA-256.
    - **Drive:** the drives the engine allows, the ones it won't erase with the reason in plain words, and Refresh.
    - **Safety check:** the engine's verdict and its checks in order, plus Verification, with the technical details. A blocked verdict offers a way back.
    - **Windows options,** for Windows installers only: the five Setup options, all off by default, with a username field for the local account and no password field. The name is checked by the engine's rules.
    - **Confirm:**
      - An empty drive is confirmed with its four-character code, typed into four boxes.
      - A drive with files is confirmed with the same code and a ticked box that says what will be lost, such as "Erase SMI USB DISK (14.8 GB) and the 1 partition with files on it". A drive whose contents could not be read still needs the full phrase.
      - The engine records a one-time confirmation that lasts 10 minutes, as `--intent` does. When it expires, writing locks until the drive is confirmed again.
      - Studio keeps exactly what was typed, and the box's text as the engine wrote it, for the engine.
      - "Write to drive" unlocks only on a match. Without Administrator rights it stays locked, and "Restart as Administrator" starts Studio again through the UAC prompt.
    - **Writing and verifying:** the engine checks the confirmation and the drive once more, then writes and reads back, with each phase's progress, speed, amount done and time left. Cancel stops after the current chunk; the stick then needs writing again. Studio never forces a busy drive free: it says another program is using the drive.
    - **Done:** "Written and verified", "Written, not verified" or "Hash mismatch" when the image's SHA-256 differs from the one entered, with the result list and the next step. A write that was cancelled or failed says so and offers to try again. "Write another stick" goes back to the drives with the same image.
  - Reading images, listing drives, the safety check and the confirmation run off the UI thread.
  - The header, action bar and status bar stay in view at any window size down to the 960 × 640 minimum; the page and the rail's "This session" scroll when there is not room.
  - Studio keeps a log, `studio.log` in OSflux's state folder, with startup times, the renderer and each step of setting up dropped files.
  - On Windows Studio draws with Slint's software renderer, which opens the window in about 0.2 s instead of about 1.2 s with OpenGL. `SLINT_BACKEND=winit-femtovg` selects OpenGL.
  - `osflux-studio` is licensed GPL-3.0-only in its package metadata, as it includes Slint under that licence.
- **Engine additions for Studio:**
  - `assess_all_targets` assesses every drive in one call.
  - `run_safety_check` splits the preflight verdict into its checks, in the same order and with the same outcome. It also notices when a different drive has taken the chosen drive's place.
  - `Sha256Hex` parses an expected SHA-256.
  - `detect_plan_family` detects an already-planned image's family without reading the image again.
  - A write can be stopped between chunks through the plan's `CancelToken`. The audit log records it as `write-cancelled`, and the error says whether anything reached the drive.
  - A third confirmation form, the identity code with an acknowledgement that names what will be lost (`identity-code-contents-acknowledged`), for drives with files whose partitions could be read. The engine writes the acknowledgement, records it with a fingerprint of the drive's partition layout in the one-time confirmation, checks both again before writing and once more after locking the drive, and records the form and the text in the audit log. The CLI keeps the full phrase.
  - IBM Plex Sans and Mono are embedded under the SIL Open Font License. `osflux-studio.exe` carries the OSflux icon and asks for Administrator when it opens.

### Changed

- **Drives are listed without PowerShell.** OSflux asks Windows' storage stack directly, as Rufus does: `IOCTL_STORAGE_QUERY_PROPERTY` for the bus, removable media, model and serial, Plug and Play for the device-instance ID and the name Device Manager shows, the disk geometry for the size when the drive can't be read, and the system volume's disk extents to find the system disk. Listing works where scripts are blocked, is faster, and never opens a console window. Drive codes stay the same; the bus is named more precisely (SATA or NVMe where Windows' old inventory said IDE or SCSI). If the system volume can't be located, every disk is treated as holding it.
- **The program is now `osflux`** (`osflux.exe` on Windows); it was `osflux-cli`.
- **Rust is pinned** to 1.97.0 in `rust-toolchain.toml`, so every machine builds and lints the same way.
