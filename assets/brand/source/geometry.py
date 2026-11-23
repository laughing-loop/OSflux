"""OSflux mark geometry. All shapes are exact (arcs + rectangles).

Coordinate system: ring centre at (0, 0), y grows downwards (SVG).
One unit = one pixel when the full mark is drawn on a 32 x 32 canvas,
so bars land on whole pixel rows at 32 px.
"""
import math

BLUE = "#0068DB"
CYAN = "#0AC4FB"
WHITE = "#FFFFFF"
BLACK = "#000000"
INK = "#161616"


def _f(v):
    s = f"{v:.4f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def c_ring_path(R, r, h):
    """Ring (outer radius R, inner radius r) opened on the left by the band |y| < h."""
    xo = -math.sqrt(R * R - h * h)
    xi = -math.sqrt(max(r * r - h * h, 0.0))
    return (
        f"M{_f(xo)} {_f(-h)}"
        f"A{_f(R)} {_f(R)} 0 1 1 {_f(xo)} {_f(h)}"
        f"L{_f(xi)} {_f(h)}"
        f"A{_f(r)} {_f(r)} 0 1 0 {_f(xi)} {_f(-h)}Z"
    )


def bar_path(x0, x1, yc, t, rounded=False):
    y0 = yc - t / 2
    if not rounded:
        return f"M{_f(x0)} {_f(y0)}H{_f(x1)}V{_f(y0 + t)}H{_f(x0)}Z"
    rad = t / 2
    return (
        f"M{_f(x0 + rad)} {_f(y0)}H{_f(x1 - rad)}"
        f"A{_f(rad)} {_f(rad)} 0 0 1 {_f(x1 - rad)} {_f(y0 + t)}"
        f"H{_f(x0 + rad)}A{_f(rad)} {_f(rad)} 0 0 1 {_f(x0 + rad)} {_f(y0)}Z"
    )


# Full mark (32 px and up)
# bar thickness t, ring thickness 2t, gap g between bars and to the ring's cut ends
FULL = dict(R=12.0, r=8.0, t=2.0, g=2.0)
FULL_BARS = [(-15, 2, -1), (-18, 0, 0), (-15, -2, 1)]  # x0, x1, row
FULL_BBOX = (-18.0, -12.0, 12.0, 12.0)  # x0, y0, x1, y1
FULL_SQUARE_VIEWBOX = (-19.0, -16.0, 32.0, 32.0)  # bbox centred, 1-unit side margins


def full_mark(rounded=False):
    R, r, t, g = FULL["R"], FULL["r"], FULL["t"], FULL["g"]
    pitch = t + g
    h = pitch + t / 2 + g
    ring = c_ring_path(R, r, h)
    bars = [bar_path(x0, x1, row * pitch, t, rounded) for x0, x1, row in FULL_BARS]
    return ring, bars


# Small mark (16-24 px): two thicker bars, heavier ring, on a 16 px grid.
SMALL = dict(cx=8.5, cy=8.0, R=7.0, r=4.0, h=4.0)  # 1 px gap between bars and the ring cut
SMALL_BARS = [(1, 10, 6.0), (3, 8, 10.0)]  # x0, x1, row centre (pixels)


def small_mark(rounded=False):
    cx, cy, R, r, h = (SMALL[k] for k in ("cx", "cy", "R", "r", "h"))
    ring = c_ring_path(R, r, h)
    bars = [bar_path(x0 - cx, x1 - cx, y - cy, 2.0, rounded) for x0, x1, y in SMALL_BARS]
    return ring, bars


# 20 px version of the small mark: 3 px bars so every edge lands on a whole pixel.
SMALL20 = dict(cx=10.5, cy=10.0, R=8.75, r=5.0, h=5.0)
SMALL20_BARS = [(1, 12, 7.5), (4, 9, 12.5)]


def small20_mark():
    cx, cy, R, r, h = (SMALL20[k] for k in ("cx", "cy", "R", "r", "h"))
    ring = c_ring_path(R, r, h)
    bars = [bar_path(x0 - cx, x1 - cx, y - cy, 3.0) for x0, x1, y in SMALL20_BARS]
    return ring, bars
