"""Builds the palettes the box shows colors from: Tailwind's, Radix's,
Material's and Chakra's, as the icons project has gathered them.

    python3 tools/palettes.py ~/scripts/icons/src/lib/palettes.ts assets/palettes.json

That file is made from the palettes' own packages by the icons project's
scripts/gen-palettes.mjs. Each palette here is its id, its name, the
names of its shades from light to dark, which of them is its main one,
and each hue's colors in that order.
"""

import json
import re
import sys


def palettes(source):
    found = []
    for block in re.split(r"\n\t\{\n", source)[1:]:
        field = lambda name: re.search(rf'\b{name}: "([^"]*)"', block).group(1)
        shades = re.findall(r'"([^"]+)"', re.search(r"shades: \[([^\]]*)\]", block).group(1))
        main = int(re.search(r"defaultShade: (\d+)", block).group(1))
        hues = [[name, colors.replace('"', "").replace(" ", "").split(",")] for name, colors in re.findall(r'\n\t\t\t"?([a-z-]+)"?: \[([^\]]*)\]', block)]
        for name, colors in hues:
            assert len(colors) == len(shades), f"{field('id')} {name}: {len(colors)} colors for {len(shades)} shades"
        found.append({"id": field("id"), "name": field("name"), "shades": shades, "main": main, "hues": hues})
    return found


if __name__ == "__main__":
    source, out = sys.argv[1:3]
    with open(source) as file:
        made = palettes(file.read())
    with open(out, "w") as file:
        json.dump(made, file, separators=(",", ":"))
        file.write("\n")
    print(", ".join(f"{p['name']} {len(p['hues'])}x{len(p['shades'])}" for p in made))
