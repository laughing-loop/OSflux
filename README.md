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

Plan an image without writing anything:

```powershell
cargo run -- --plan-image "C:\path\to\image.iso"
```

Print the same plan as JSON:

```powershell
cargo run -- --plan-image "C:\path\to\image.iso" --json
```

The planner reads known file signatures first, then falls back to the extension when no signature is found.
For ISO images, it also checks for an El Torito boot catalog and reports boot metadata, including BIOS, UEFI, or hybrid boot style when present. It validates each boot entry's byte range so OSflux can reject unsafe or incomplete image plans before write support is enabled. The planner also reports the selected write strategy, such as ISO filesystem boot setup, deeper ISO inspection, or raw block writing.
Every image plan has a status: ready, needs-inspection, or blocked.
The engine builds a structured plan report so the CLI and future Studio UI can render the same planning result without duplicating logic.

Run with demo pipeline options:

```powershell
cargo run -- --blocks=16 --block-size=1048576
```
