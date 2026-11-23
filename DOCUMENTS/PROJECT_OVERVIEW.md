# OSflux Project Overview

This document explains what we have done so far, why we did it, and what comes next.

## What OSflux Is

OSflux is planned as a next-generation bootable media and OS deployment platform.

In simple terms, OSflux will help prepare USB drives for operating system installation and deployment. The long-term goal is not just to copy files to a USB drive, but to build a fast, intelligent, extensible platform.

## Why There Is No Full UI Yet

Right now, we are building the engine first.

That means we are starting with the low-level foundation that will later power every interface:

- command-line interface
- desktop GUI
- automation workflows
- future extensions

A nice UI can come later, but it needs a reliable engine underneath it. If we build the UI first without the engine, the buttons would exist but would not have real power behind them yet.

## The Platform Structure

OSflux is structured as a platform, not as one big script.

## OSflux Core

Location:

```text
crates/osflux-core
```

This is the low-level system layer.

It will handle things like:

- finding storage devices
- opening physical drives safely
- writing data in large blocks
- using Windows I/O APIs
- managing the write pipeline

This layer is where performance matters most.

The device layer now has a shared `BlockDevice` trait. Windows has the first implementation, while Linux and macOS have placeholder modules ready for future work.

The writer layer now has a `WriteStrategy` trait. This lets Windows, Linux, and macOS use different high-performance write methods later without forcing the engine to care about operating-system details.

## OSflux Engine

Location:

```text
crates/osflux-engine
```

This is the orchestration layer.

It will decide what should happen and in what order.

Example future flow:

```text
Input image -> Image plan -> Device preparation -> Write -> Verify
```

The engine should not be tied to only one user interface. The CLI and future GUI should both call into this layer.

## OSflux CLI

Location:

```text
crates/osflux-cli
```

This is the command-line interface.

Right now it is the first user-facing entrypoint because it is faster to build, easier to test, and useful while the engine is still young.

Later commands may look like:

```powershell
osflux-cli --list-devices
osflux-cli write --image Windows.iso --target PhysicalDrive2
```

## OSflux Studio

This is the planned graphical interface.

It does not exist yet. When the engine is stronger, OSflux Studio can become the visual app with buttons, progress bars, drive selection, image selection, and settings.

## What We Have Done So Far

We installed and verified the Rust MSVC toolchain.

We fixed the first Windows API compile issues.

We converted the project into a Rust workspace.

We created the first platform crates:

- `osflux-core`
- `osflux-engine`
- `osflux-cli`

We pushed the first commit to GitHub:

```text
https://github.com/laughing-loop/OSflux
```

We prepared a GitHub Actions workflow template:

```text
DOCUMENTS/CI_WORKFLOW_TEMPLATE.md
```

The active workflow is disabled for now because GitHub is failing before any job starts. Local Rust checks still work, and we can restore the workflow when the GitHub-side setting is fixed.

We added the first read-only device command:

```powershell
cargo run -- --list-devices
```

This is the first step toward safe drive selection.

The device scan now tries to show:

- disk path
- size
- model
- interface type
- media type
- safety label
- system-drive warning

OSflux can now assess a target drive by index:

```powershell
cargo run -- --assess-target 2
```

This is still read-only. It classifies the selected drive as blocked, needing confirmation, or eligible. Eligible means the drive is blank or holds one empty FAT32 or exFAT partition, so its four-character identity code is enough to confirm a write.

OSflux can now combine image planning and target assessment into a write preflight:

```powershell
cargo run -- --preflight --image "C:\path\to\image.iso" --target 2
```

This is also read-only. It gives one verdict for the image and target pair: blocked, needs-inspection, needs-confirmation, or ready.

OSflux can now create a write intent:

```powershell
cargo run -- --intent --image "C:\path\to\image.iso" --target 2
```

This is still read-only. It turns a passing preflight into a deliberate confirmation phrase, such as:

```text
OSFLUX ERASE TARGET 2 7F3A
```

That phrase is the safety gate a future write command must require before opening a target drive for write access.

The last part, `7F3A` in this example, is the drive's identity code. OSflux builds it from the drive's model, serial number, and Windows device ID. If a different drive ends up at the same index, for example after USB sticks are swapped, the code changes and the old phrase no longer matches.

OSflux can now write Windows installer ISOs. Their files live in a UDF filesystem rather than a hybrid partition table, so OSflux reads the UDF tree and copies it onto a new MBR drive with one FAT32 partition that starts in UEFI mode only. The FAT32 image is generated as it is written, from the ISO, and read back and compared like any other write. Most Windows 11 ISOs carry a `sources/install.wim` larger than the 4 GiB FAT32 limit; OSflux writes it as `install.swm`, `install2.swm` and so on, of at most 3800 MiB each, which Windows Setup reads in its place. `DISM /Split-Image` stays documented in the README as the fallback.

OSflux can now validate a confirmed write command as a dry-run skeleton:

```powershell
cargo run -- --write --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

This still does not write to any drive. It checks:

- the image plan
- the target assessment
- the write preflight
- the write intent
- the exact confirmation phrase

If everything matches, the command reports `ready-dry-run` and keeps `write access attempted` as `no`.

OSflux can now prepare a locked write session:

```powershell
cargo run -- --session --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

This still does not write to any drive. It turns the confirmed dry-run write command into a session record with:

- a session ID
- the locked image path
- the locked target index and path
- confirmation status
- dry-run status
- write access status

For now, a prepared session means the safety chain is complete, not that OSflux is allowed to write yet.

OSflux can now prepare a dry-run chunk plan:

```powershell
cargo run -- --chunk-plan --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

The chunk plan reads the image size and calculates:

- chunk size
- chunk count
- final chunk size
- planned bytes
- dry-run status
- write access status

This is the foundation for progress reporting. If an image is 10 MiB and the chunk size is 4 MiB, OSflux plans 3 chunks: 4 MiB, 4 MiB, and 2 MiB.

OSflux can now preview progress events:

```powershell
cargo run -- --progress-preview --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

This still does not write to any drive. It turns a chunk plan into progress snapshots such as:

- start
- first completed chunk
- midpoint checkpoint
- final completed chunk

The preview reports progress as integer basis points. In plain terms, `10000` means `100.00%`, `5000` means `50.00%`, and `4000` means `40.00%`.

OSflux can now select a write mode:

```powershell
cargo run -- --select-mode --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A"
```

This still does not write to any drive. It decides which future execution path is appropriate:

- ISO filesystem boot setup when ISO boot metadata is trusted
- raw block writing when the image is a raw disk image
- needs-inspection when the image is recognized but not trusted enough
- blocked when the safety chain or image plan does not pass

OSflux can now detect an image family:

```powershell
cargo run -- --detect-family --image "C:\path\to\image.iso"
```

This still does not write to any drive. It uses the image planner first, then scans the early image bytes for family markers.

The family detector can currently classify:

- Windows installer images
- Linux installer images
- multi-family images
- raw disk images
- unknown images that need deeper inspection

This matters because the future writer should not treat every image the same way. A raw disk image can follow a raw block path, while an installer ISO may need filesystem-aware boot setup.

OSflux can now inspect ISO directory records:

```powershell
cargo run -- --plan-image "C:\path\to\image.iso"
```

This still does not write to any drive. The image planner now reads the ISO Primary Volume Descriptor, finds the root directory record, walks real ISO directory entries with safety limits, and reports boot-related paths when they are present.

The directory inspector can currently identify paths such as:

- boot managers
- UEFI bootloaders
- installer image files
- Linux bootloaders
- Linux kernel files
- initrd files
- live filesystem images
- GRUB boot paths

This is more reliable than only scanning raw bytes because OSflux is now reading the ISO's actual filesystem records.

OSflux can now build a filesystem-aware ISO plan:

```powershell
cargo run -- --filesystem-plan --image "C:\path\to\image.iso"
```

This still does not write to any drive. It combines:

- image family evidence
- ISO boot catalog validity
- boot image range validation
- ISO directory boot assets

The result is a higher-level plan that can choose paths such as:

- Windows installer boot setup
- Linux installer boot setup
- multi-family boot setup
- deeper inspection
- unsupported

This matters because the future write engine needs a clean decision before it can safely choose filesystem setup behavior.

We also added the first read-only image planning command:

```powershell
cargo run -- --plan-image "C:\path\to\image.iso"
```

This command classifies the image, reads its size, chooses an initial write mode, and prints notes/warnings. It reads known file signatures first, then falls back to the extension when no signature is found. It does not open or write to any drive.

## GitHub Actions Status

GitHub Actions is the robot that can build and check the project on GitHub.

For now, the active workflow is disabled because GitHub starts every run with a startup failure before any job is created.

The workflow template is stored here:

```text
DOCUMENTS/CI_WORKFLOW_TEMPLATE.md
```

When the GitHub-side issue is fixed, that workflow can:

- install/configure Rust
- check formatting
- compile the full workspace
- run tests

Because OSflux uses Windows APIs, the workflow runs on:

```text
windows-latest
```

That is intentional.

## How To Run The Project Locally

From the project folder:

```powershell
cd C:\Users\jolyn\OSflux
cargo run
```

Current output:

```text
OSflux CLI
OSflux Engine starting...
OSflux Core pipeline processed 8 block(s).
```

That output means the CLI is calling the engine, and the engine is calling the core pipeline.

## What Comes Next

The next useful milestone is improving image planning.

We now have this command:

```powershell
cargo run -- --plan-image "C:\path\to\image.iso"
```

The deeper image inspection milestone has started. OSflux can now detect early signatures such as ISO volume descriptors, UDF descriptors, GPT headers, and MBR boot signatures.

OSflux can also inspect ISO boot metadata for an El Torito boot catalog. That is how many bootable ISO images describe their boot entries, platform, emulation mode, and boot image location.

The planner now turns that catalog data into a boot style:

- BIOS
- UEFI
- BIOS + UEFI hybrid
- unknown or not bootable

It also validates the byte range for each boot entry. In plain terms, OSflux checks whether the boot catalog points to real readable bytes inside the ISO instead of blindly trusting the catalog.

OSflux now also reads ISO directory records. In plain terms, it can inspect the ISO's real file table and report boot asset paths instead of relying only on extension checks or raw marker searches.

The planner now selects an initial write strategy:

- ISO filesystem boot setup when boot metadata is complete and valid
- deeper ISO inspection when boot metadata is missing, unknown, or unsafe
- raw block writing for raw disk images
- unknown strategy for files OSflux cannot classify yet

The planner now also assigns a status:

- ready: the image plan is trusted for the next safety phase
- needs-inspection: OSflux recognizes the image, but needs stronger metadata before trusting it
- blocked: OSflux cannot safely use the image plan yet

The engine now builds a structured plan report. The CLI renders that report today, and the future Studio interface can use the same report instead of rebuilding planning logic in the GUI.

The CLI can now print that plan report as JSON:

```powershell
cargo run -- --plan-image "C:\path\to\image.iso" --json
```

That gives OSflux a machine-readable output path for future automation, testing, and GUI integration.

The preflight can also be printed as JSON:

```powershell
cargo run -- --preflight --image "C:\path\to\image.iso" --target 2 --json
```

The write intent can also be printed as JSON:

```powershell
cargo run -- --intent --image "C:\path\to\image.iso" --target 2 --json
```

The write command dry-run can also be printed as JSON:

```powershell
cargo run -- --write --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --json
```

The locked write session can also be printed as JSON:

```powershell
cargo run -- --session --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --json
```

The chunk plan can also be printed as JSON:

```powershell
cargo run -- --chunk-plan --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --chunk-size 4194304 --json
```

The progress preview can also be printed as JSON:

```powershell
cargo run -- --progress-preview --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --chunk-size 4194304 --json
```

The write mode selection can also be printed as JSON:

```powershell
cargo run -- --select-mode --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2 7F3A" --chunk-size 4194304 --json
```

The image family report can also be printed as JSON:

```powershell
cargo run -- --detect-family --image "C:\path\to\image.iso" --json
```

The filesystem-aware ISO plan can also be printed as JSON:

```powershell
cargo run -- --filesystem-plan --image "C:\path\to\image.iso" --json
```

After that, we build:

- filesystem staging plan
- real write pipeline
- progress reporting
- verification
- GUI later

## Development Rule

We build the dangerous parts carefully.

Writing to a physical drive can erase data. So before OSflux writes anything, it needs strong safety checks:

- clearly list target devices
- detect removable drives
- show disk size and identity
- require explicit confirmation before writing
- avoid touching system drives by accident

## Short Version

We are currently building the foundation.

The CLI is the first control panel.

The engine will become the brain.

The core will become the power layer.

The GUI comes after the engine can do real work.
