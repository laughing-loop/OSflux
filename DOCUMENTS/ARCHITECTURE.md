# Project Architecture — Next-Gen Bootable Media Engine

## Vision
A high-performance, extensible, and intelligent USB bootable media and OS deployment platform focused on top-tier write speed, automation, and extensibility.

---

## Core Principles

- I/O-bound optimization first
- Pipeline-based architecture (no blocking)
- Adaptive performance tuning
- Modular + plugin-driven design
- Cross-platform core (Windows-first optimization)

---

## System Layers

### 1. Device Layer
Handles raw disk interaction.

Responsibilities:
- Enumerate physical drives
- Lock/unmount volumes
- Provide raw read/write access
- Detect sector size & alignment

Design:
- Shared trait boundary lives in `osflux-core::device`.
- Windows, Linux, and macOS implementations stay behind that boundary.
- The engine depends on `BlockDevice` behavior, not operating-system APIs.

---

### 2. Image Layer
Parses and classifies input images.

Supports:
- ISO (El Torito, hybrid)
- IMG/raw disk images
- Windows install ISOs
- Linux distributions

Outputs:
- ImagePlan (structured execution plan)

---

### 3. Write Engine (Core Performance Layer)

Responsibilities:
- High-throughput writing
- Overlapped I/O (Windows)
- Adaptive buffering
- Queue-based pipeline

Key Features:
- Multi-threaded pipeline
- Dynamic buffer sizing
- Queue depth tuning

Design:
- Platform-agnostic pipeline code owns orchestration and buffering.
- Platform write strategies handle the actual I/O style.
- Windows uses the Windows strategy boundary for overlapped I/O.
- Unix-like systems will use a Unix strategy boundary for positional writes and later async upgrades.

---

### 4. Filesystem Layer

Responsibilities:
- Create FAT32/NTFS/exFAT
- Extract files from ISO
- Bootloader setup

---

### 5. Intelligence Layer (Differentiator)

Responsibilities:
- Detect OS type
- Recommend optimal settings
- Auto-configure installs

Examples:
- Windows → TPM bypass, unattended install
- Linux → persistence setup

---

### 6. Orchestration Layer

Pipeline:

ISO → ImagePlan → Filesystem Prep → Write Engine → Verify

---

### 7. Plugin System

Trait-based extension system.

Supports:
- OS handlers
- Bootloaders
- Deployment modules

---

## Execution Pipeline

[ISO Reader] → [Buffer Queue] → [Write Workers] → [USB Device]

Goal:
- Keep USB saturated at all times
- Zero idle write cycles

---

## Performance Strategy

- Large block writes (1MB–8MB)
- Overlapped I/O (async)
- Device-specific tuning
- Double buffering

---

## Future Extensions

- Cloud ISO repository
- Automated OS deployment profiles
- Multi-boot management system
- Telemetry-based optimization
