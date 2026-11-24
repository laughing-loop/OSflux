# OSflux Studio — UI tokens and design decisions

Design canvas: https://claude.ai/artifact/LcNGtozL23SK2bvSjfXcdU
- Page "v1 · Brand": 14 frames + component sheet (rounded, brand gradient, Inter).
- Page "v2 · Minimal": 4 key screens in dark and light (IBM-style corporate minimal, IBM Plex). In progress — chosen direction to take forward.

---

# v2 · Minimal (current direction)

Decisions (2026-11-19): IBM-style corporate minimal; first pass = 4 key screens (Image loaded, Drive list, Safety check, Confirm) in dark + light; brand gradient on the logo only; fonts switched to IBM Plex Sans + IBM Plex Mono (OFL, embedded).

Update (2026-11-19, later): two confirmation levels (short drive code vs full phrase), no cut-off values, labels and identifiers now match the osflux-engine source (see "Engine mapping").

## Layout
- Header 48 (logo, name, version · privilege status · 48×48 Settings / About).
- Left rail 240: vertical step list (current step = selected row + 3 px accent bar), "This session" summary at the bottom (image, drive, mode).
- Content padding 24 / 32. Title block: "Step n of 4" 12 · title 28/300 · description 14.
- Action bar 64 at the bottom of the content column: hint text left, square buttons flush right (label left, icon right).
- Status bar 28: dry-run notice · engine status · drives scanned time · Refresh.

## Colour
| Token | Dark | Light |
|---|---|---|
| bg | #141414 | #FFFFFF |
| layer-01 (rail, tiles, inputs, list) | #1E1E1E | #F4F4F4 |
| layer-02 | #2A2A2A | #E8E8E8 |
| selected row | #333333 | #E0E0E0 |
| border-subtle | #2E2E2E | #E0E0E0 |
| border-strong (section rules, input underline) | #5A5A5A | #8C8C8C |
| text | #F2F2F2 | #161616 |
| text-secondary | #A8A8A8 | #525252 |
| text-disabled | #6B6B6B | #A8A8A8 |
| accent / primary button | #0068DB | #0068DB |
| link, selection ring, step indicator | #4D9FFF | #0068DB |
| secondary button | #393939 (white text) | #393939 (white text) |
| warning band | rgba(239,68,68,0.12) | #FDECEC |
| locked write button | bg rgba(239,68,68,0.16), text #FF9A9A | bg #FDECEC, text #A61B1B |

Accent #0068DB is the OSflux brand blue (#0076F7) one step darker so white text passes 5.3:1.

## Engine status
Shown as shape + colour + label (never colour alone): Ready = filled circle ✓, Needs confirmation = filled circle i, Needs inspection = filled triangle !, Blocked = filled octagon ×. Dark uses the exact hexes (#22C55E, #0AC4FB, #F59E0B, #EF4444) with a dark glyph; light uses #15803D, #0369A1, #B45309, #B91C1C with a white glyph.

A drive with engine status `eligible` uses the Ready style (label "Ready").

## Type (IBM Plex)
- Sans: title 28/300 · section 16/600 · body 14/400 (+0.16 px) · label/caption 12/400 (+0.32 px) · verdict title with 28 px status icon.
- Mono: phrase 28/500 (+0.08em) · drive code boxes 28/500 · input 16 · values 13–14 · paths 12.

## Shape
- Radius 0 everywhere except radio buttons and status shapes. No shadows; depth comes from layer colours and 1 px rules.
- Slint: Fluent's rounded controls won't match — build small custom components (Button, LineEdit, CodeBoxes, row, status) over Slint primitives.

## Text overflow
- Never cut off a value that the user needs to judge safety: sizes, device paths, identity codes, chunk sizes, engine values.
- Sizes always sit in their own right-aligned column, in both the eligible list and the "Not allowed" list.
- Technical details values wrap onto a second line instead of ending in "…".
- Only long file names in the "This session" rail and step subtitles may truncate, in the middle (ubuntu-24.04.1-…-amd64.iso), with the full name on hover.

## Confirmation levels
The engine always checks the full phrase `OSFLUX ERASE TARGET {index} {code}`. The UI only changes how much the person has to type; Studio passes the full phrase to the engine itself.

| Drive's engine status | What the person types | Screen |
|---|---|---|
| `eligible` (blank or new removable drive) | The 4-character drive code, e.g. `7F3A` | Drive summary card + 4 square code boxes |
| `needs-confirmation` (has partitions or files, or larger than 64 GB) | The full phrase, e.g. `OSFLUX ERASE TARGET 4 B21C` | Full-phrase screen + amber note "This drive has files on it, so OSflux asks for the full phrase." |
| `blocked` | Nothing — cannot be selected | Listed under "Not allowed" |

- The drive code is 4 hex characters (0–9, A–F) built from the drive's model, serial and PnP ID. It changes if a different drive takes the same slot.
- Code boxes: upper-case on input, auto-advance, paste fills all four, Backspace moves back. States: empty, wrong (red border, "Code does not match this drive"), matching (green, "Matches SanDisk Ultra 64GB").
- Copy: sidebar Confirm subtitle "Confirm the drive"; safety-check line "Everything checks out. Next, confirm the drive."

Engine work needed before this is real: today `classify_target` returns `needs-confirmation` for every allowed drive and never `eligible`, and the device layer does not read existing partitions. Add a partition / volume check (and the 64 GB threshold) so the engine can return `eligible` for blank drives.

## Behaviour
- Next locked on Needs inspection / Blocked; safety check stops at the first failure.
- Confirm: the dry run unlocks only on an exact match (code or full phrase, per the table above). Inputs upper-case.
- "Write to drive" stays locked in v0.1.0. Note: the engine now has a real raw-disk-image write command with read-back verify (`writing` → `verifying`), so this button can unlock first for raw images once Studio wires it up.

## Engine mapping (from the osflux-engine source)
Use these exact values in technical panels; UI labels on the left.

| UI label | Engine value |
|---|---|
| **Image plan status** | |
| Ready / Needs inspection / Blocked | `ready` / `needs-inspection` / `blocked` |
| **Image kind** | |
| ISO image / Raw disk image / Unknown | `iso` / `raw-disk-image` / `unknown` |
| Format evidence | "ISO 9660 volume descriptor" · "UDF volume descriptor" · "GPT header" · "MBR boot signature" (detected by `signature`, else `extension`) |
| **Image family** | |
| Windows installer / Linux installer / Multi-family / Raw disk / Unknown | `windows-installer` / `linux-installer` / `multi-family-image` / `raw-disk-image` / `unknown` |
| Boot files found (kinds) | `boot-manager`, `uefi-bootloader`, `linux-kernel`, `initrd`, `grub-boot-path`, `linux-bootloader`, `live-filesystem`, `installer-image` |
| **Planned write method** | |
| ISO filesystem boot setup | `iso-filesystem-boot-setup` |
| Needs deeper inspection | `iso-needs-deeper-inspection` |
| Raw block write | `raw-block-write` |
| Family-specific setup (shown in family report) | `windows-installer-boot-setup` / `linux-installer-boot-setup` / `multi-family-boot-setup` |
| **Drive (target) status** | |
| Ready / Needs confirmation / Blocked | `eligible` / `needs-confirmation` / `blocked` |
| Blocked reasons (plain-English label → engine reason) | "System drive — Windows is installed here" → `target contains the system volume` · "External SSD/HDD — usually holds data" → `target is an external hard disk or SSD` · "Size could not be read" → `target size is unknown` · "Drive identity could not be read" → `target identity could not be read` · "Reports itself as a fixed disk" → `target does not report removable media` |
| Not elevated (privilege pill) | access `metadata-only` → "Run as Administrator before any write workflow." |
| **Safety check (preflight) verdict** | |
| Ready / Needs confirmation / Needs inspection / Blocked | `ready` / `needs-confirmation` / `needs-inspection` / `blocked` |
| **Write mode selection** | `selected` / `needs-inspection` / `blocked` |
| **Confirmation** | `confirmation-required` → `confirmation-missing` / `confirmation-mismatch` → match |
| **Session** | `prepared` (dry run: `ready-dry-run`) / `blocked` |
| **Write progress phases** | `planned` → `prepared` → `writing` → `verifying` |

---

# v1 · Brand (reference)

## Window
- 1080 × 720 default, 960 × 640 minimum. Frames show the client area (native title bar not drawn).
- Header 52 · step bar 68 · footer 34 · content padding 16 / 24 · nav row 44.

## Colour (dark)
surface.bg #0A0B10 · chrome #0E1016 · well #0C0D13 · card #12141B · raised #1A1D26 · border #262B36 · border.strong #2F3542 · text #E8EAF0 · secondary #8A90A0 · disabled #5B6170 · select #0076F7 · danger.fill #DC2626 · danger.text #F5A3A3.
Light: bg #F4F5F9, card #FFFFFF, raised #F0F2F7, border #DDE1EA, border.strong #CBD1DC, text #12141B, secondary #5A6070.

## Engine status
| Status | Colour | Light-theme text | Light-theme fill |
|---|---|---|---|
| Ready | #22C55E | #15803D | #DCFCE7 |
| Needs confirmation | #0AC4FB | #0369A1 | #E0F2FE |
| Needs inspection | #F59E0B | #B45309 | #FEF3C7 |
| Blocked | #EF4444 | #B91C1C | #FEE2E2 |

## Brand gradient
#0AC4FB → #0076F7 → #7038F7 on logo, active step, progress bars; primary button uses blue → violet only. Never on destructive buttons.

## Type and shape
Inter + JetBrains Mono. Radius 6 / 8 / 10 / 12 / 14. One card shadow 0 8 24 rgba(0,0,0,0.35). Solid 1 px borders.

## Placeholders still to fill (both versions)
- Real OSflux logo: files exist in the repo root (`OSflux_logo.png` on black, `OSflux_logo_plain_bkg.png` on white, 1254 × 1254). Upload one to the canvas to replace the stand-in gradient mark. Tagline: "Flow. Flash. Deploy."
- About: copyright holder, licence, website / source / report links.
- Engine identifiers in the technical panels: now real — use the "Engine mapping" table (hyphenated, e.g. `iso-filesystem-boot-setup`, not `iso_filesystem_boot_setup`).
