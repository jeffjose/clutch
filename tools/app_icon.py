"""Makes clutch's own icon, the one a list of programs and the box's window
show: Solar's gear-change pattern in a square, from its bold duotone set,
with the two tones made a color and white as hotshot's icon is, so that
it reads on a dark panel and on a light one alike.

    python3 tools/app_icon.py ICONS_JSON assets/clutch.svg
    convert -background none -density 1024 assets/clutch.svg \
        -resize 256x256 assets/clutch.png

ICONS_JSON is the set as Iconify publishes it:
https://cdn.jsdelivr.net/npm/@iconify-json/solar/icons.json

The PNG is what clutch carries, for the window of its box; the SVG is
for `~/.local/share/icons/hicolor/scalable/apps/`, where `clutch setup`
puts it.
"""

import json
import re
import sys

# The square, and the gear pattern drawn on it. The color is the one the
# box picks things out in.
BODY = "#3E63DD"
MARKS = "#FFFFFF"


def main(icons_json, out):
    with open(icons_json) as file:
        body = json.load(file)["icons"]["transmission-square-bold-duotone"]["body"]
    paths = re.findall(r"<path[^>]*/>", body)
    drawn = []
    for path in paths:
        # The tone Solar draws at half strength is the body; the rest
        # is what stands on it.
        faint = 'opacity=".5"' in path
        path = path.replace(' opacity=".5"', "")
        drawn.append(path.replace("<path", f'<path fill="{BODY if faint else MARKS}"', 1))
    # A little room round it, as hotshot's has, so that it is not hard
    # against the edge of whatever shows it.
    svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="1 1 22 22">{"".join(drawn)}</svg>\n'
    with open(out, "w") as file:
        file.write(svg)
    print(f"{len(drawn)} paths")


if __name__ == "__main__":
    main(*sys.argv[1:3])
