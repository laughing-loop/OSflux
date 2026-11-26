# OSflux Studio — UI tokens and design decisions

Design canvas: https://claude.ai/artifact/LcNGtozL23SK2bvSjfXcdU
- Page "v1 · Brand": 14 frames + component sheet (rounded, brand gradient, Inter). Reference only; still shows the old dry-run flow.
- Page "v2 · Minimal": the chosen direction. 26 screens + component sheet, each in dark and light (IBM-style corporate minimal, IBM Plex). Laid out by section, dark row above light row.

---

# v2 · Minimal (current direction)

Decisions (2026-11-19): IBM-style corporate minimal; brand gradient on the logo only; fonts IBM Plex Sans + IBM Plex Mono (OFL, embedded).

Update (2026-11-19, later): two confirmation levels (short drive code vs full phrase), no cut-off values, labels and identifiers match the osflux-engine source (see "Engine mapping").

Update (2026-11-23): writing is real for raw disk images, Linux ISOs and Windows 11 ISOs. Dry-run-only wording and the dry-run result screen are gone. New: Windows options step, writing/verifying screens, Done screens, expected SHA-256, Settings (verify after writing), four error states.

Update (2026-11-23, later): real OSflux logo in every v2 frame (full mark everywhere, including the header). Linux ISO write method and the Needs inspection frame corrected from the engine source (`image_plan.rs`).

Update (2026-11-24): confirmation levels corrected from the engine source. There is no 64 GB rule; eligibility depends only on what the drive holds. Studio passes the engine what the person typed (code or phrase), not the full phrase in place of a code.

Update (2026-11-26): a drive with files that the engine can name is confirmed in Studio with its 4-character code and a ticked box that says what will be lost, e.g. "Erase SMI USB DISK (14.8 GB) and the 1 partition with files on it". The full phrase stays for drives whose contents could not be read, and is the only confirmation the CLI takes for a drive with files. Frame 14 changes to match; the PNG still shows the full phrase until it is redrawn, and `--preview confirm-files` shows the new screen.

## Frames (v2 page)
- Image: 01 empty · 02 Windows 11 ISO, Ready (expected SHA-256 filled) · 03 Linux ISO, Ready (hybrid, byte-for-byte copy) · 04 Needs inspection (ISO with a missing or invalid boot catalog)
- Drive: 05 list (Not allowed open) · 06 no drives found
- Safety check: 07 Ready · 08 Blocked (8.41 GB image on a 7.45 GB stick)
- Windows options: 09 none selected · 10 options 1, 2 and 3 on
- Confirm: 11–13 drive code (empty, wrong, matching) · 14 SD card with files: drive code typed, box naming what will be lost not yet ticked, verification off (Studio previews `confirm-files` and `confirm-files-ticked`; the PNG still shows the earlier full-phrase screen, which is now only for a drive whose contents could not be read, preview `confirm-phrase`)
- Writing: 15 Writing · 16 Verifying · 17 Writing with verification off
- Done: 18 Written and verified (hash matches) · 19 Written, not verified · 20 Hash mismatch
- Errors: 21 Confirmation expired · 22 Drive changed or unplugged · 23 Not running as Administrator
- 24 Settings (verify on) · 25 Settings (verify off) · 26 About · 27 Component sheet

## Flow
- Windows 11 ISO: 5 steps — Image, Drive, Safety check, Windows options, Confirm. Then Writing → Verifying → Done.
- Linux ISO and raw disk image: 4 steps — Image, Drive, Safety check, Confirm. Then Writing (→ Verifying) → Done.
- Before an image is chosen the sidebar shows 4 steps.

## Logo
- Files in the repo: `assets/brand/svg/`. Dark theme uses `osflux-mark-white.svg`, light theme uses `osflux-mark-blue.svg` (#0068DB).
- Use the full mark (ring + three streaks) everywhere in Studio: 24 px in the header, 40 px in About and on the component sheet.
- Don't use the small mark (`osflux-mark-small-*`, heavy ring + two bars) in the Studio header: it reads as a different logo next to the full mark. Keep it for favicons and tray icons.

## Layout
- Header 48 (24 px logo, name, version · privilege status · 48×48 Settings / About).
- Left rail 240: vertical step list (current step = selected row + 3 px accent bar). "This session" at the bottom: Image, Drive, Windows options (Windows only), Verify after writing.
- Content padding 24 / 32. Title block: "Step n of 4/5" 12 · title 28/300 · description 14.
- Action bar 64 at the bottom of the content column: hint text left, square buttons flush right (label left, icon right).
- Status bar 28: "Verify after writing: On/Off" (or "Writing — keep the stick plugged in") · engine status · drives scanned time · Refresh.
- Settings and About are 520 px square dialogs over a dimmed window, with flush 64 px buttons.

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
| accent / primary button / checkbox on / switch on | #0068DB | #0068DB |
| link, selection ring, step indicator | #4D9FFF | #0068DB |
| secondary button | #393939 (white text) | #393939 (white text) |
| danger · "Write to drive" (white text) | #DC2626 | #DC2626 |
| info band | rgba(77,159,255,0.12) | #E5F0FF |
| warning band (red) | rgba(239,68,68,0.12) | #FDECEC |
| amber note / band | rgba(245,158,11,0.14) | #FEF3C7 |
| progress track (fill = accent) | #393939 | #E0E0E0 |

Accent #0068DB is the OSflux brand blue, the same blue as the logo; white text on it passes 5.3:1.

## Engine status
Shown as shape + colour + label (never colour alone): Ready = filled circle ✓, Needs confirmation = filled circle i, Needs inspection = filled triangle !, Blocked = filled octagon ×. Dark uses the exact hexes (#22C55E, #0AC4FB, #F59E0B, #EF4444) with a dark glyph; light uses #15803D, #0369A1, #B45309, #B91C1C with a white glyph.

A drive with engine status `eligible` uses the Ready style (label "Ready").

## Type (IBM Plex)
- Sans: title 28/300 · section 16/600 · body 14/400 (+0.16 px) · label/caption 12/400 (+0.32 px) · verdict / result title with 28 px status icon.
- Mono: phrase 28/500 (+0.08em) · drive code boxes 28/500 · progress values 16–20 · hashes 13 · input 16 · values 13–14 · paths 12.

## Shape
- Radius 0 everywhere except radio buttons, status shapes and the switch. No shadows; depth comes from layer colours and 1 px rules.
- Slint: Fluent's rounded controls won't match — build small custom components (Button, LineEdit, CodeBoxes, Checkbox, Switch, PhaseProgress, row, status) over Slint primitives.

## Text overflow
- Never cut off a value that the user needs to judge safety: sizes, device paths, identity codes, chunk sizes, hashes, engine values.
- Sizes always sit in their own right-aligned column, in both the eligible list and the "Not allowed" list.
- Technical details values wrap onto a second line instead of ending in "…".
- Only long file names in the "This session" rail and step subtitles may truncate, in the middle at a separator (ubuntu-24.04.1-…-amd64.iso, Windows11_Client_…_9457.iso), with the full name on hover.

## Image step
- Windows 11 ISO: family "Windows installer", boot style UEFI. Write method "Windows installer: files copied to a FAT32 stick (UEFI)". Notes: "install.wim (6.2 GB) will be split into 2 parts to fit FAT32. Windows Setup reads them as normal." and (amber) "Boots on UEFI PCs only. Legacy BIOS mode is not supported."
- Linux ISO (hybrid, e.g. Ubuntu): write method "Hybrid ISO: copied to the drive byte for byte" (engine `hybrid-iso-raw-write`). Note: "This ISO is hybrid: it already holds a partition table, so OSflux copies it byte for byte."
- Needs inspection happens only when (a) a non-hybrid ISO's El Torito boot catalog is missing or invalid (`iso-needs-deeper-inspection`, reason "ISO boot metadata needs deeper inspection"), or (b) the image was classified by file extension only (reason "image classification used extension fallback"). There is no "run deeper inspection" action: the frame offers "Choose another image" and Next stays locked. An unknown image kind or an empty file is Blocked, not Needs inspection.
- Every loaded image has an optional field "Expected SHA-256 (from the download page)" under the file name. The Done screen shows match or mismatch.

## Windows options (Windows ISOs only, between Safety check and Confirm)
- Banner: "These settings apply to every PC that boots from this stick."
- Five square checkboxes, all off by default, each with a one-line explanation:
  1. Skip the TPM 2.0, Secure Boot and 4 GB RAM checks → amber note "Microsoft doesn't support Windows 11 on PCs that fail these checks."
  2. Use a local account instead of a Microsoft account → Username field (no password field) + "Windows Setup will ask for the password. OSflux never stores it on the stick."
  3. Skip the privacy questions (Setup chooses the private answer to each)
  4. Turn off automatic BitLocker drive encryption
  5. Use this PC's region and keyboard settings
- Selected options appear in "This session" (short labels: Skip hardware checks · Local account (name) · Skip privacy questions · No automatic BitLocker · This PC's region and keyboard) and in the Confirm summary.
- Maps to CLI flags `--bypass-requirements`, `--local-account <NAME>`, `--skip-privacy-questions`, `--no-auto-bitlocker`, `--copy-region` (autounattend.xml at the stick root).

## Confirmation levels
The engine decides which confirmation a drive needs and checks it. Studio passes the engine exactly what the person typed, the drive code or the full phrase, plus, for a drive with files, the engine's own acknowledgement text when the box is ticked. It never turns a typed code into the phrase. That way the engine applies the checks that go with each confirmation, and the audit log records which one was used.

- For an `eligible` drive the engine accepts the 4-character drive code on its own. It then reads the drive again before the write and once more after locking it, and refuses the write if anything has appeared on it. The full phrase is accepted too.
- For a drive with files whose partitions the engine could read, Studio asks for the drive code and a ticked box. The box's text comes from the engine (`contents_acknowledgement` in `write_intent.rs`): "Erase {model} ({size}) and the {n} partition(s) with files on it", with the size written as Studio writes sizes. The engine records the text and a fingerprint of the drive's partition layout with the 10-minute confirmation. Before writing it builds the text again from a fresh look at the drive, and checks the layout again once the drive is locked; if a partition was added, removed or reformatted, it refuses and asks for a new confirmation. The audit log records the form (`identity-code-contents-acknowledged`) and the text. The full phrase is accepted too.
- For any other allowed drive (contents that could not be read, or data with no partition to name), only the full phrase `OSFLUX ERASE TARGET {index} {code}` confirms the write.
- The CLI keeps the full phrase for every drive with files; the code with a box exists only in Studio.
- The drive's size plays no part: a blank 128 GB stick is `eligible` as well (`target.rs`, `drive_contents.rs`).

| Drive's engine status | What the person does | Screen |
|---|---|---|
| `eligible` (empty drive: blank, or one FAT32 or exFAT partition with nothing in it) | Types the 4-character drive code, e.g. `7F3A` | Drive summary card (with Windows options + verification) + 4 square code boxes |
| `needs-confirmation`, with files the engine can name (one or more partitions read) | Types the drive code, e.g. `B21C`, and ticks "Erase SD card in USB card reader (14.8 GB) and the 1 partition with files on it" | Amber note "This drive has files on it. Type its code and tick the box to confirm." + red band (what will be erased) + the code shown in blue next to 4 square code boxes + the box: a check box with a red left edge and the engine's text in Plex Sans 14/600 + match message |
| `needs-confirmation`, contents not readable or nothing to name | Types the full phrase, e.g. `OSFLUX ERASE TARGET 4 B21C` | Full-phrase screen + amber note "OSflux couldn't check what's on this drive, so it asks for the full phrase." (or "This drive has files on it, so OSflux asks for the full phrase." when it holds data with no partition to name) |
| `blocked` | Nothing — cannot be selected | Listed under "Not allowed" |

- The drive code is 4 hex characters (0–9, A–F). It changes if a different drive takes the same slot.
- Code boxes: upper-case on input, auto-advance, paste fills all four, Backspace moves back. States: empty, wrong (red border, "Code does not match this drive"), matching (green, "Matches SanDisk Ultra 64GB").
- With the box (a drive with files): the right code with the box clear shows "Code matches. Tick the box to unlock Write to drive." (neutral); ticking the box does not make a wrong code right. Rail subtitle "Type the code and tick the box".
- "Write to drive" (red, #DC2626) unlocks only on an exact match (and, for a drive with files, a ticked box); before that it is disabled grey with a lock icon. Confirmations last 10 minutes.

## Writing and verifying
- Two phases in order: Writing, then Verifying ("Every byte is read back and compared"). Each phase: progress bar, MB/s, GB done of total, time left. Head shows total time left.
- Example: Windows11_Client_x64_en-gb_26300_9457.iso, 8.41 GB at ~20 MB/s: writing about 7 min, verifying about 7 min.
- Slow hint: "The stick sets the speed. A USB 3 stick in a USB 3 port is usually 3–4× faster."
- Cancel (secondary) with "If you cancel, the stick will need writing again before you can use it." The engine stops after the current chunk (`CancelToken` on the write plan); the head shows "Cancelling…" until it has. The result screen then reads "Writing cancelled" (amber) with the band "This stick won't start a PC. It needs writing again before you can use it." (or "Nothing was written to the drive." when it stopped while the ISO was still being read), the rail says "Needs writing again", and "Try again" confirms the drive again: the one-time confirmation was used by the write.
- Verification off: single Writing phase + amber band "Verification: off".
- Windows ISOs: the engine reads the whole ISO first to record its SHA-256 (`hashing`). The Writing card shows that read with "Reading the image first, to record its SHA-256.", and the head says "Reading the image", before writing starts.
- A drive that another program keeps open: Studio does not offer to force it free, as the CLI can; the result screen says "Another program is using the drive." Nothing is written.
- Errors at write time that leave the drive untouched go back to Confirm: an expired confirmation shows frame 21, a changed drive frame 22. Without Administrator rights (frame 23), Write to drive stays locked and "Restart as Administrator" starts Studio again through the UAC prompt.

## Settings
- Writing: "Verify after writing" switch, on by default. Off shows amber "Faster, but a fake or failing stick won't be caught." When off, Confirm (action bar hint), the progress screen, the status bar and "This session" say "Verification: off".
- Appearance: Follow Windows / Light / Dark. Audit log: Open audit log.

## Done
- "Written and verified" (green) or "Written, not verified" (amber triangle).
- Result list: Image SHA-256 · Expected SHA-256 with Matches / Mismatch / Not entered · Verification · Time taken (with writing and verifying split) · "Recorded in the audit log".
- Next step tile: "Next: plug the stick into the PC, open the boot menu (F12, F11 or Esc) and choose the UEFI: entry for the USB stick."
- Buttons: Open audit log (secondary), Write another stick (primary).

## Error states
- Confirmation expired (amber band, "Confirm the drive again").
- Drive changed or unplugged between Safety check and Confirm (red band, sidebar Drive step blocked, summary card dimmed with "No longer found", "Choose the drive again").
- Not running as Administrator (header "Not elevated — read-only", amber band "Restart OSflux as Administrator to write", "Restart as Administrator").
- Hash mismatch after writing (red octagon "Hash mismatch", expected vs actual hashes in full, "Download the image again before you use this stick.").

## Behaviour
- Next locked on Needs inspection / Blocked; safety check stops at the first failure.
- Inputs upper-case. Write unlocks only on an exact match (code, code with the box ticked, or full phrase).

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
| **Planned write method (image write strategy)** | |
| ISO filesystem boot setup (Windows ISOs) | `iso-filesystem-boot-setup` |
| Hybrid ISO: copied byte for byte (Linux ISOs) | `hybrid-iso-raw-write` (write mode selection: `raw-block-write`) |
| Needs deeper inspection | `iso-needs-deeper-inspection` |
| Raw block write | `raw-block-write` |
| Family-specific setup (shown in family report) | `windows-installer-boot-setup` / `linux-installer-boot-setup` / `multi-family-boot-setup` |
| **Drive (target) status** | |
| Ready / Needs confirmation / Blocked | `eligible` / `needs-confirmation` / `blocked` |
| Blocked reasons (plain-English label → engine reason) | "System drive — Windows is installed here" → `target contains the system volume` · "External SSD/HDD — usually holds data" → `target is an external hard disk or SSD` · "Size could not be read" → `target size is unknown` · "Drive identity could not be read" → `target identity could not be read` · "Reports itself as a fixed disk" → `target does not report removable media` |
| Not elevated (privilege pill) | access `metadata-only` → "Restart OSflux as Administrator to write" |
| **Safety check (preflight) verdict** | |
| Ready / Needs confirmation / Needs inspection / Blocked | `ready` / `needs-confirmation` / `needs-inspection` / `blocked` |
| **Write mode selection** | `selected` / `needs-inspection` / `blocked` |
| **Confirmation** | `confirmation-required` → `confirmation-missing` / `confirmation-mismatch` → match |
| Confirmed with (audit log, write plan) | `full-phrase` / `identity-code` / `identity-code-contents-acknowledged` |
| **Session** | `prepared` / `blocked` |
| **Write progress phases** | `planned` → `prepared` → `writing` → `verifying` (verify skipped with `--no-verify`) |

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
#0AC4FB → #0076F7 → #7038F7 on logo, active step, progress bars; primary button uses blue → violet only. Never on destructive buttons. (v1 still uses the old stand-in gradient mark.)

## Type and shape
Inter + JetBrains Mono. Radius 6 / 8 / 10 / 12 / 14. One card shadow 0 8 24 rgba(0,0,0,0.35). Solid 1 px borders.

## Placeholders still to fill
- About: copyright holder, licence, website / source / report links.
- Version badge still shows v0.1.0.
