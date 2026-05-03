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

We also added a GitHub Actions workflow:

```text
.github/workflows/rust.yml
```

That workflow checks the project automatically whenever code is pushed.

We added the first read-only device command:

```powershell
cargo run -- --list-devices
```

This is the first step toward safe drive selection.

## What GitHub Actions Is Doing

GitHub Actions is the robot that builds and checks the project on GitHub.

When we push code, it will:

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

The next useful milestone is device listing.

We want this command:

```powershell
cargo run -- --list-devices
```

That should show available drives in a safe way. This milestone has started, and the next improvement is to add richer device identity such as model, bus type, removable status, and system-drive warnings.

After that, we build:

- image input planning
- write mode selection
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
