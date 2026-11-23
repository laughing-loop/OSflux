"""Build the OSflux logo pack: SVG masters, PNG exports, Windows .ico, macOS icons."""
import os, sys, subprocess, shutil
from PIL import Image

sys.path.insert(0, os.path.dirname(__file__))
from geometry import *  # noqa
from wordmark import wordmark_paths, CAP_HEIGHT

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
FONTS = ROOT
OUT = os.path.join(ROOT, "osflux-logo-pack")
if os.path.exists(OUT):
    shutil.rmtree(OUT)
for d in ["svg", "png/mark", "png/lockup", "icons/windows", "icons/macos", "icons/tile"]:
    os.makedirs(os.path.join(OUT, d), exist_ok=True)

VARIANTS = {
    # name: (bar colour, ring colour, text colour)
    "blue": (BLUE, BLUE, BLUE),
    "ink": (BLUE, BLUE, INK),
    "white": (WHITE, WHITE, WHITE),
    "black": (BLACK, BLACK, BLACK),
    "two-tone": (CYAN, BLUE, BLUE),
}


def svg_doc(viewbox, body, title):
    vb = " ".join(f"{v:g}" for v in viewbox)
    w, h = viewbox[2], viewbox[3]
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb}" width="{w:g}" height="{h:g}" '
            f'role="img" aria-label="{title}"><title>{title}</title>{body}</svg>\n')


def mark_group(bar_c, ring_c, transform="", rounded=False, small=False):
    ring, bars = (small_mark if small else full_mark)(rounded)
    t = f' transform="{transform}"' if transform else ""
    if bar_c == ring_c:
        return f'<g fill="{ring_c}"{t}><path d="{ring}"/>' + "".join(f'<path d="{b}"/>' for b in bars) + "</g>"
    return (f'<g{t}><path fill="{ring_c}" d="{ring}"/>'
            + "".join(f'<path fill="{bar_c}" d="{b}"/>' for b in bars) + "</g>")


def write(path, text):
    with open(os.path.join(OUT, path), "w") as f:
        f.write(text)


def render(svg_rel, png_rel, width, height=None):
    cmd = ["rsvg-convert", "-w", str(width)]
    if height:
        cmd += ["-h", str(height)]
    subprocess.run(cmd + [os.path.join(OUT, svg_rel), "-o", os.path.join(OUT, png_rel)], check=True)


# ---------------------------------------------------------------- marks
for name in ["blue", "white", "black", "two-tone"]:
    bar_c, ring_c, _ = VARIANTS[name]
    write(f"svg/osflux-mark-{name}.svg",
          svg_doc(FULL_SQUARE_VIEWBOX, mark_group(bar_c, ring_c), "OSflux"))
write("svg/osflux-mark-blue-rounded-alt.svg",
      svg_doc(FULL_SQUARE_VIEWBOX, mark_group(BLUE, BLUE, rounded=True), "OSflux"))
for name in ["blue", "white", "black"]:
    bar_c, ring_c, _ = VARIANTS[name]
    write(f"svg/osflux-mark-small-{name}.svg",
          svg_doc((0, 0, 16, 16), mark_group(bar_c, ring_c, f"translate({SMALL['cx']:g} {SMALL['cy']:g})", small=True),
                  "OSflux"))

# ---------------------------------------------------------------- lockups
S = 41.0                       # font units per mark unit: ring diameter = 24 * 41 = 1.41 x cap height
RING_CX = 18 * S               # bars start at x = 0
RING_CY = -CAP_HEIGHT / 2      # ring centred on the capitals
GAP = 0.55 * 12 * S            # ring edge to text
TEXT_X = RING_CX + 12 * S + GAP
(os_path, flux_path), adv = wordmark_paths([
    (os.path.join(FONTS, "IBMPlexSans-SemiBold.ttf"), "OS"),
    (os.path.join(FONTS, "IBMPlexSans-Regular.ttf"), "flux"),
])
top = RING_CY - 12 * S
LOCKUP_VB = (0, round(top), round(TEXT_X + adv + 10), round(24 * S))
for name, (bar_c, ring_c, text_c) in VARIANTS.items():
    body = (mark_group(bar_c, ring_c, f"translate({RING_CX:g} {RING_CY:g}) scale({S:g})")
            + f'<g fill="{text_c}" transform="translate({TEXT_X:g} 0)"><path d="{os_path}"/><path d="{flux_path}"/></g>')
    write(f"svg/osflux-lockup-{name}.svg", svg_doc(LOCKUP_VB, body, "OSflux"))

# ---------------------------------------------------------------- app icons
# macOS: 1024 canvas, 824 tile, Big Sur grid; white mark ~62% of tile width, bbox centred
MS = 18.0
bx0, by0, bx1, by1 = FULL_BBOX
mac_tx = 512 - (bx0 + bx1) / 2 * MS + 6   # +6 px optical nudge to the right
mac_ty = 512 - (by0 + by1) / 2 * MS
write("svg/osflux-app-icon-macos.svg", svg_doc(
    (0, 0, 1024, 1024),
    f'<rect x="100" y="100" width="824" height="824" rx="185" fill="{BLUE}"/>'
    + mark_group(WHITE, WHITE, f"translate({mac_tx:g} {mac_ty:g}) scale({MS:g})"),
    "OSflux"))
# square tile with sharp corners (avatars, social, store tiles)
TS = 10.5
write("svg/osflux-tile.svg", svg_doc(
    (0, 0, 512, 512),
    f'<rect width="512" height="512" fill="{BLUE}"/>'
    + mark_group(WHITE, WHITE, f"translate({256 - (bx0 + bx1) / 2 * TS + 4:g} {256:g}) scale({TS:g})"),
    "OSflux"))

# ---------------------------------------------------------------- PNG exports
for name in ["blue", "white", "black", "two-tone"]:
    for s in (256, 512, 1024):
        render(f"svg/osflux-mark-{name}.svg", f"png/mark/osflux-mark-{name}-{s}.png", s, s)
for name in VARIANTS:
    for w in (1200, 2400):
        render(f"svg/osflux-lockup-{name}.svg", f"png/lockup/osflux-lockup-{name}-{w}w.png", w)

# Windows: blue mark on transparent; 16-24 px use the small mark
WIN_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
win_imgs = []
# y nudge (in pixels at that size) so the small mark's bars land on whole pixel rows
SMALL_NUDGE = {16: 0.0, 24: 0.5}
FULL_NUDGE = {40: 0.25, 48: 0.5}
small_body = mark_group(BLUE, BLUE, f"translate({SMALL['cx']:g} {SMALL['cy']:g})", small=True)
for s in WIN_SIZES:
    rel = f"icons/windows/osflux-{s}.png"
    if s == 20:
        ring20, bars20 = small20_mark()
        body20 = (f'<g fill="{BLUE}" transform="translate({SMALL20['cx']:g} {SMALL20['cy']:g})">'
                  f'<path d="{ring20}"/>' + "".join(f'<path d="{b}"/>' for b in bars20) + "</g>")
        tmp = "icons/windows/_tmp20.svg"
        write(tmp, svg_doc((0, 0, 20, 20), body20, "OSflux"))
        render(tmp, rel, 20, 20)
        os.remove(os.path.join(OUT, tmp))
    elif s <= 24:
        k = s / 16
        tmp = f"icons/windows/_tmp{s}.svg"
        write(tmp, svg_doc((0, -SMALL_NUDGE[s] / k, 16, 16), small_body, "OSflux"))
        render(tmp, rel, s, s)
        os.remove(os.path.join(OUT, tmp))
    elif s in FULL_NUDGE:
        k = s / 32
        vx, vy, vw, vh = FULL_SQUARE_VIEWBOX
        tmp = f"icons/windows/_tmp{s}.svg"
        write(tmp, svg_doc((vx, vy - FULL_NUDGE[s] / k, vw, vh), mark_group(BLUE, BLUE), "OSflux"))
        render(tmp, rel, s, s)
        os.remove(os.path.join(OUT, tmp))
    else:
        render("svg/osflux-mark-blue.svg", rel, s, s)
    win_imgs.append(Image.open(os.path.join(OUT, rel)).convert("RGBA"))
big = win_imgs[-1]
big.save(os.path.join(OUT, "icons/windows/osflux.ico"), format="ICO",
         sizes=[(s, s) for s in WIN_SIZES], append_images=win_imgs[:-1])

# macOS: tile PNGs + .icns
MAC_SIZES = [16, 32, 64, 128, 256, 512, 1024]
for s in MAC_SIZES:
    render("svg/osflux-app-icon-macos.svg", f"icons/macos/osflux-app-icon-{s}.png", s, s)
mac1024 = Image.open(os.path.join(OUT, "icons/macos/osflux-app-icon-1024.png")).convert("RGBA")
try:
    mac1024.save(os.path.join(OUT, "icons/macos/osflux.icns"), format="ICNS",
                 append_images=[Image.open(os.path.join(OUT, f"icons/macos/osflux-app-icon-{s}.png")).convert("RGBA")
                                for s in MAC_SIZES[:-1]])
    print("icns ok")
except Exception as e:
    print("icns skipped:", e)

for s in (512, 1024):
    render("svg/osflux-tile.svg", f"icons/tile/osflux-tile-{s}.png", s, s)

print("built", OUT)
