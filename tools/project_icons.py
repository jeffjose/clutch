"""Builds the font of pictures that a project with no icon of its own is
given one of: Solar's bold duotone drawings, each as two letters, since a
font's letter has one tone and these have two.

    uv run --no-project --with fonttools --with picosvg \
        tools/project_icons.py ICONS_JSON assets/solar-duotone.ttf

ICONS_JSON is the set as Iconify publishes it:
https://cdn.jsdelivr.net/npm/@iconify-json/solar/icons.json

The drawing at place N in PICKS has its full tone at code point
E000 + 2N and the tone Solar draws at half strength at the one after.
Add to the end of PICKS, never the middle: which drawing a project gets
is counted from its name, and a drawing put in the middle would change
them all. `projects::DRAWINGS` is how many there are.
"""

import json
import re
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.svgLib.path import parse_path
from picosvg.svg import SVG

# Things, each plain at a glance and unlike the others.
PICKS = """
rocket planet atom bolt flame leaf ghost cat paw crown star compass
telescope satellite ufo asteroid black-hole magnet lightbulb key gamepad
gameboy dumbbell cup-hot donut balloon bomb bone confetti magic-stick-3
palette paint-roller cassette vinyl boombox headphones-round camera
clapperboard reel tv radio map globus signpost umbrella snowflake sun
moon-stars cloud-bolt tornado waterdrop fire ferris-wheel skateboard
football basketball bowling golf tennis volleyball target medal-star
cup-star gift box suitcase backpack t-shirt glasses wineglass tea-cup
chef-hat whisk bottle test-tube dna bacteria virus pill stethoscope cpu
database server flash-drive sd-card usb socket wheel bus tram scooter
plain delivery garage home-smile buildings city shop sofa armchair lamp
chandelier bath bell flag bookmark book notebook diploma
square-academic-cap calculator ruler-pen scissors eraser pen paperclip
pin binoculars flashlight hourglass stopwatch alarm wallet money-bag
ticket tag cart masks incognito bot bug code layers widget structure
heart shield lock sledgehammer toolbox broom corkscrew ladle perfume
""".split()

EM = 1000
GRID = 24
FIRST = 0xE000


def glyph(shapes):
    """These shapes as one letter: Solar's 24 by 24 square taken to the
    font's em, the right way up."""
    pen = TTGlyphPen(None)
    if not shapes:
        return pen.glyph()
    body = "".join(shapes).replace("currentColor", "#000")
    svg = SVG.fromstring(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {GRID} {GRID}">{body}</svg>')
    scale = EM / GRID
    to_em = TransformPen(Cu2QuPen(pen, max_err=1.0, reverse_direction=False), (scale, 0, 0, -scale, 0, EM))
    for shape in svg.topicosvg().shapes():
        parse_path(shape.as_path().d, to_em)
    return pen.glyph()


def main():
    icons_json, out = sys.argv[1:3]
    solar = json.load(open(icons_json))["icons"]

    glyphs, cmap, missing = {".notdef": TTGlyphPen(None).glyph()}, {}, []
    for at, pick in enumerate(PICKS):
        drawing = solar.get(pick + "-bold-duotone")
        if not drawing:
            missing.append(pick)
            continue
        shapes = re.findall(r"<(?:path|circle|ellipse|rect)\b[^>]*/>", drawing["body"])
        # The faint tone is whatever Solar draws less than solid.
        faint = [re.sub(r' opacity="[^"]*"', "", shape) for shape in shapes if " opacity=" in shape]
        full = [shape for shape in shapes if " opacity=" not in shape]
        for tone, (name, shapes) in enumerate([(pick, full), (pick + ".faint", faint)]):
            glyphs[name] = glyph(shapes)
            cmap[FIRST + 2 * at + tone] = name
    if missing:
        sys.exit(f"Solar has no drawing for: {', '.join(missing)}")

    font = FontBuilder(EM, isTTF=True)
    font.setupGlyphOrder(list(glyphs))
    font.setupCharacterMap(cmap)
    font.setupGlyf(glyphs)
    font.setupHorizontalMetrics({name: (EM, 0) for name in glyphs})
    font.setupHorizontalHeader(ascent=EM, descent=0)
    font.setupNameTable({"familyName": "clutch project icons", "styleName": "Regular"})
    font.setupOS2(sTypoAscender=EM, sTypoDescender=0, usWinAscent=EM, usWinDescent=0)
    font.setupPost()
    font.save(out)
    print(f"{len(PICKS)} drawings")


if __name__ == "__main__":
    main()
