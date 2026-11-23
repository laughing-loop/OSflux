"""Outline the OSflux wordmark in IBM Plex Sans so the SVGs need no font."""
import uharfbuzz as hb
from fontTools.ttLib import TTFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen

CAP_HEIGHT = 698  # IBM Plex Sans, units per em = 1000


class _Font:
    def __init__(self, path):
        self.tt = TTFont(path)
        self.glyphset = self.tt.getGlyphSet()
        blob = hb.Blob.from_file_path(path)
        self.hb = hb.Font(hb.Face(blob))
        self.order = self.tt.getGlyphOrder()


def _shape(font, text):
    buf = hb.Buffer()
    buf.add_str(text)
    buf.guess_segment_properties()
    hb.shape(font.hb, buf, {"kern": True, "liga": False})
    return [(font.order[i.codepoint], p.x_advance, p.x_offset, p.y_offset)
            for i, p in zip(buf.glyph_infos, buf.glyph_positions)]


def wordmark_paths(parts, tracking=-6):
    """parts: [(font_path, text), ...]. Returns (list of SVG path strings per part, total advance).
    Coordinates: font units, baseline at y = 0, y down (SVG)."""
    x = 0.0
    out = []
    for font_path, text in parts:
        font = _Font(font_path)
        pen = SVGPathPen(font.glyphset)
        for name, adv, xo, yo in _shape(font, text):
            # flip y: font units are y-up
            tpen = TransformPen(pen, (1, 0, 0, -1, x + xo, -yo))
            font.glyphset[name].draw(tpen)
            x += adv + tracking
        out.append(pen.getCommands())
    return out, x - tracking
