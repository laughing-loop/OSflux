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

This is still read-only. It classifies the selected drive as blocked or needing confirmation before any future write workflow can touch it.

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
OSFLUX ERASE TARGET 2
```

That phrase is the safety gate a future write command must require before opening a target drive for write access.

OSflux can now validate a confirmed write command as a dry-run skeleton:

```powershell
cargo run -- --write --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2"
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
cargo run -- --session --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2"
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
cargo run -- --chunk-plan --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2"
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
cargo run -- --progress-preview --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2"
```

This still does not write to any drive. It turns a chunk plan into progress snapshots such as:

- start
- first completed chunk
- midpoint checkpoint
- final completed chunk

The preview reports progress as integer basis points. In plain terms, `10000` means `100.00%`, `5000` means `50.00%`, and `4000` means `40.00%`.

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
cargo run -- --write --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2" --json
```

The locked write session can also be printed as JSON:

```powershell
cargo run -- --session --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2" --json
```

The chunk plan can also be printed as JSON:

```powershell
cargo run -- --chunk-plan --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2" --chunk-size 4194304 --json
```

The progress preview can also be printed as JSON:

```powershell
cargo run -- --progress-preview --image "C:\path\to\image.iso" --target 2 --confirm "OSFLUX ERASE TARGET 2" --chunk-size 4194304 --json
```

After that, we build:

- write mode selection
- image family detection
- richer boot metadata inspection
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
