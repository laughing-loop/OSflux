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

Write a raw disk image or a hybrid ISO to a USB stick. **This erases the stick.** Run it from an Administrator PowerShell, after `--intent` has printed the phrase for that stick. The same command works for a `.iso` that OSflux reports as hybrid:

```powershell
cargo run -- --write --image "C:\path\to\image.img" --target 3 --confirm "OSFLUX ERASE TARGET 3 7F3A" --execute
```

Without Administrator rights OSflux stops before the prompt, leaves the confirmation unused, and exits with code 3. Before erasing, OSflux shows the drive's model, size, and path and asks you to type `yes`; anything else cancels without writing. Scripts and automation can pass `--yes` to skip the prompt. Without a terminal and without `--yes`, OSflux refuses rather than assume the answer.

When the write finishes, OSflux prints the SHA-256 of the image it wrote, so you can compare it with the checksum the image's publisher lists.

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
For a drive that passes those checks, OSflux also reads what it holds, without changing anything and without locking its volumes. `--assess-target` and `--list-devices` show each partition's filesystem, label, size, and used space. A drive is `eligible` when it is blank (its first and last MiB are uniformly empty) or holds exactly one FAT32 or exFAT partition whose root folder is empty apart from the volume label and a `System Volume Information` folder. Deleted files do not count; anything else in the root folder, a second partition, an extended partition, NTFS or FAT16, a damaged partition table, or a filesystem written without a partition table means `needs-confirmation`. Reading a drive needs Administrator rights, so without them every drive needs confirmation.
The preflight blocks any image that is larger than the target drive, so an oversized image never reaches the confirmation step.
The write intent command never touches a drive. It creates the confirmation phrase that `--write --execute` requires, and records a one-time confirmation for that drive and that image. For an `eligible` drive it also offers a short confirmation: the drive's four-character identity code, such as `--confirm 2E85`. The full phrase is always accepted too.
The phrase ends with the target drive's identity code, built from its model, serial number, and Windows device ID. `7F3A` in these examples is only a placeholder: run `--intent` to get the real phrase for your drive. If a different drive takes the same index, the code changes and the old phrase stops matching.
The recorded confirmation works for one write, within 10 minutes, with the same image file. Running `--write --execute` a second time, starting more than 10 minutes later, or after replacing or editing the image, is refused until `--intent` is run again. Confirmations are kept in `%LOCALAPPDATA%\OSflux\authorizations` on Windows, next to the audit log (`$XDG_STATE_HOME/osflux` or `~/.local/state/osflux` elsewhere), or under the folder named by the `OSFLUX_STATE_DIR` environment variable when it is set.
The write command skeleton is also read-only. It checks the exact confirmation phrase and reports whether a future write session would be allowed to proceed, while keeping `writeAccessAttempted` set to `false`.
Adding `--execute` to `--write` is the only way OSflux writes to a drive. It only writes raw disk images and hybrid ISOs, only to a removable target, and only with the exact phrase from `--intent` while its confirmation is still valid. It re-runs the safety chain and re-checks the drive, uses up the confirmation, then opens the drive, which locks and dismounts the drive's volumes. A write confirmed with the identity code needs the drive to be empty both when `--intent` ran and when the write starts, and OSflux checks once more after locking the drive, before writing anything; if anything was added in between, it refuses and asks for the full phrase. It refuses to continue if the opened drive's size does not match the confirmed one, writes the image in 4 MiB chunks, reads every byte back to verify it, and then releases the drive.
A hybrid ISO also carries a partition table in its first sector, which is how most Linux installers (Ubuntu, Fedora, Debian, Arch, Mint) are built. OSflux copies those byte for byte, the same way it writes a `.img`, and `--plan-image` reports them with `hybrid: yes` and the `hybrid-iso-raw-write` strategy. ISOs that need their files extracted, such as Windows installers, are still refused.
The write session command is still read-only. It locks the confirmed image and target details into a session record, assigns a session ID, and keeps write access disabled.
The chunk plan command is still read-only. It calculates image size, chunk size, chunk count, final chunk size, and planned bytes for future progress reporting.
The progress preview command is still read-only. It turns the chunk plan into sampled progress events with integer percentage basis points.
The write mode selection command is still read-only. It chooses the future execution path, such as ISO filesystem boot setup or raw block writing, only after the safety chain has passed.

Every command sets an exit code so scripts can check the result without parsing the output:

- `0`: the command ran and its result passed.
- `1`: the command could not run, for example because the image could not be read.
- `2`: an argument was unknown, missing, or invalid.
- `3`: the command ran, but its result is blocked or needs inspection, the confirmation was refused, the write needs Administrator rights, or the final prompt before erasing was declined. The report is still printed.

Run with demo pipeline options:

```powershell
cargo run -- --blocks=16 --block-size=1048576
```
