# OSflux logo pack

Flat, geometric version of the OSflux ring-and-streaks mark. Built from exact shapes (arcs and rectangles), so it stays sharp at every size. `preview.png` shows everything at a glance.

## What to use where

| Use | File |
|---|---|
| Website, README, documents (light background) | `svg/osflux-lockup-blue.svg` |
| App header and documents that use black text | `svg/osflux-lockup-ink.svg` (blue mark, #161616 wordmark) |
| Dark backgrounds, dark theme | `svg/osflux-lockup-white.svg`, `svg/osflux-mark-white.svg` |
| Single-colour print, stamps, engraving | `svg/osflux-lockup-black.svg`, `svg/osflux-mark-black.svg` |
| Marketing only (large sizes, 48 px and up) | `svg/osflux-lockup-two-tone.svg`, `svg/osflux-mark-two-tone.svg` |
| Mark on its own, 32 px and up | `svg/osflux-mark-blue.svg` |
| Mark at 16–24 px (favicons, tray, small UI) | `svg/osflux-mark-small3-16.svg`, `-20.svg`, `-24.svg`: three pixel-aligned bars, one file per size |
| Alternative small mark, 16–24 px | `svg/osflux-mark-small-blue.svg` (two thicker bars), with `-black` and `-white` versions |
| Windows app / .exe icon | `icons/windows/osflux.ico` (16, 20, 24, 32, 40, 48, 64, 96, 128, 256; 16–24 use the three-bar small mark) |
| macOS app icon | `icons/macos/osflux.icns`, PNGs 16–1024 |
| Avatars, social, store tiles | `icons/tile/osflux-tile-512.png` / `-1024.png`, `svg/osflux-tile.svg` |

PNG exports of every mark and lockup are in `png/`. `svg/osflux-mark-blue-rounded-alt.svg` is the rounded-end version, kept for comparison. The square ends are the primary style because they match the Studio v2 design.

## Colours

| Name | Hex | Use |
|---|---|---|
| Brand blue | `#0068DB` | Mark, wordmark, app icon tile (white text on it passes 5.3:1) |
| Cyan | `#0AC4FB` | Streaks in the two-tone variant only |
| Ink | `#161616` | Wordmark in the ink lockup |
| White / Black | `#FFFFFF` / `#000000` | Single-colour versions |

## Rules

- **Clear space:** keep empty space around the logo equal to half the ring's height.
- **Minimum size:** lockup at least 96 px wide on screen (25 mm in print). Below 32 px, use the small mark, not the full one.
- Don't stretch, rotate, add shadows, glows or outlines, or recolour outside the palette above.
- Don't use the two-tone version below 48 px: cyan on white is too faint at small sizes.
- Don't retype the wordmark. The SVGs already contain it as outlines (IBM Plex Sans, "OS" SemiBold + "flux" Regular), so no font is needed.

## In the OSflux repo

Suggested location: `assets/brand/`.

- **Slint window icon:** `Window { icon: @image-url("../assets/brand/svg/osflux-mark-blue.svg"); }`
- **Studio header:** `osflux-mark-blue.svg` next to the "OSflux Studio" text, or `osflux-lockup-ink.svg` / `osflux-lockup-white.svg` by theme.
- **Windows .exe icon:** in `build.rs`, with the `winresource` crate:
  ```rust
  let mut res = winresource::WindowsResource::new();
  res.set_icon("assets/brand/icons/windows/osflux.ico");
  res.compile().unwrap();
  ```
- **macOS bundle:** point `CFBundleIconFile` (or your bundler's icon setting) at `osflux.icns`.

## Geometry (for anyone redrawing or extending it)

- Full mark on a 32-unit grid (1 unit = 1 px at 32 px): ring outer radius 12, inner radius 8; three bars 2 units thick with 2-unit gaps; the ring opens on the left where the bars enter. Bars run from x −15 to 2, −18 to 0 and −15 to −2 (ring centre at 0, 0), so their right ends step back as they go down.
- Small mark, 16–24 px: three bars, like the full mark, each drawn for its size so every bar edge lands on a whole pixel. The bars are 1 px thick with 1 px gaps at 16 px, and 2 px thick with 1 px gaps at 20 and 24 px. The Windows icon's 16, 20 and 24 px frames and `icons/windows/osflux-16.png`, `-20.png` and `-24.png` use it.
- Alternative small mark, on a 16 px grid: two 2 px bars, heavier ring, 1 px gaps. A 20 px version uses 3 px bars. Every edge lands on a whole pixel at 16, 20, 24, 32, 48, 64, 96, 128 and 256 px.
- `source/` contains the scripts that generate the whole pack (needs Python with fontTools and uharfbuzz, IBM Plex Sans TTFs and `rsvg-convert`).

Wordmark font: IBM Plex Sans, SIL Open Font License 1.1.

## Licence

The OSflux name and logo are not covered by OSflux's GPL-3.0-or-later licence; Joly Doh keeps all rights to them. See the Licence section of the main README.
