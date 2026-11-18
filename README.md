# OSflux

OSflux is a next-generation bootable media and OS deployment platform.

## Platform Layout

- OSflux Core: low-level device access, Windows I/O bindings, and write pipeline primitives.
- OSflux Engine: orchestration layer for image planning, write execution, and verification.
- OSflux CLI: command-line interface for local development and operator workflows.
- OSflux Studio: planned graphical interface built on top of the same engine.

## Current Status

The repository is currently a Rust workspace with the first platform crates in place:

- `crates/osflux-core`
- `crates/osflux-engine`
- `crates/osflux-cli`

For a beginner-friendly explanation of the project direction, read:

- `DOCUMENTS/PROJECT_OVERVIEW.md`

The GitHub Actions workflow is stored as a template until repository Actions startup is fixed:

- `DOCUMENTS/CI_WORKFLOW_TEMPLATE.md`

## Development

Requirements:

- Rust stable MSVC toolchain
- Visual Studio C++ build tools
- Windows SDK

Run the CLI:

```powershell
cargo run
```

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
The preflight blocks any image that is larger than the target drive, so an oversized image never reaches the confirmation step.
The write intent command is still read-only. It creates the confirmation phrase that a future destructive write command must require before opening a target drive for write access.
The phrase ends with the target drive's identity code, built from its model, serial number, and Windows device ID. `7F3A` in these examples is only a placeholder: run `--intent` to get the real phrase for your drive. If a different drive takes the same index, the code changes and the old phrase stops matching.
The write command skeleton is also read-only. It checks the exact confirmation phrase and reports whether a future write session would be allowed to proceed, while keeping `writeAccessAttempted` set to `false`.
The write session command is still read-only. It locks the confirmed image and target details into a session record, assigns a session ID, and keeps write access disabled.
The chunk plan command is still read-only. It calculates image size, chunk size, chunk count, final chunk size, and planned bytes for future progress reporting.
The progress preview command is still read-only. It turns the chunk plan into sampled progress events with integer percentage basis points.
The write mode selection command is still read-only. It chooses the future execution path, such as ISO filesystem boot setup or raw block writing, only after the safety chain has passed.

Every command sets an exit code so scripts can check the result without parsing the output:

- `0`: the command ran and its result passed.
- `1`: the command could not run, for example because the image could not be read.
- `2`: an argument was unknown, missing, or invalid.
- `3`: the command ran, but its result is blocked or needs inspection. The report is still printed.

Run with demo pipeline options:

```powershell
cargo run -- --blocks=16 --block-size=1048576
```
