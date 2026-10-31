#!/usr/bin/env python3
"""Give mods' signs and chests what 26.3 draws them with.

    mod_entity_blocks.py --client-jar 1.21.1.jar --resources RESOURCES --mods-dir GAME_HOME/horizon-mods

In 1.21.1 signs and chests are drawn by code from entity textures, so their
block models hold only a particle texture. Horizon (like 26.3) draws a sign
from block models (`template_sign_rot_N` ...) with a 32x32 texture in a new
layout, and a chest from `entity/chest/<name>` textures.

Signs: the old-to-new pixel layout is learnt from vanilla itself, by matching
each 26.3 `block/<wood>_sign.png` pixel to the one 1.21.1
`entity/signs/<wood>.png` pixel that agrees in every wood; each mod sign's
entity texture is then laid out the same way, and the sign gets vanilla's
blockstate and models with it.

Chests: the textures a mod keeps for a `*_chest` block (`<wood>/normal.png`,
`left`, `right`, or `entity/chest/...`) are copied to
`<ns>:entity/chest/<block>[_left|_right]`, the names the game looks for.
"""

import argparse
import io
import json
import re
import zipfile
from pathlib import Path

from PIL import Image

WOODS = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "bamboo", "crimson",
         "warped"]
SUFFIXES = ["_wall_hanging_sign", "_hanging_sign", "_wall_sign", "_sign"]


def learn(client_jar, resources, hanging):
    """{new (x, y): old (x, y)} for the sign texture, from vanilla's woods; old size."""
    old_dir = "entity/signs/hanging/" if hanging else "entity/signs/"
    pairs = []
    with zipfile.ZipFile(client_jar) as z:
        for wood in WOODS:
            new = resources / f"assets/minecraft/textures/block/{wood}_{'hanging_' if hanging else ''}sign.png"
            name = f"assets/minecraft/textures/{old_dir}{wood}.png"
            if new.exists() and name in z.namelist():
                pairs.append((Image.open(io.BytesIO(z.read(name))).convert("RGBA"),
                              Image.open(new).convert("RGBA")))
    if len(pairs) < 3:
        return None, None
    old0, new0 = pairs[0]
    # Index the old pixels by their colours across all woods.
    where = {}
    for y in range(old0.height):
        for x in range(old0.width):
            key = tuple(o.getpixel((x, y)) for o, _ in pairs)
            where.setdefault(key, (x, y))
    mapping = {}
    for y in range(new0.height):
        for x in range(new0.width):
            key = tuple(n.getpixel((x, y)) for _, n in pairs)
            if key in where:
                mapping[(x, y)] = where[key]
    return mapping, (new0.width, new0.height, old0.width, old0.height)


def convert(image, mapping, sizes):
    nw, nh, ow, oh = sizes
    image = image.convert("RGBA")
    # A mod's texture at another resolution scales with the layout.
    k = image.width / ow
    out = Image.new("RGBA", (int(nw * k), int(nh * k)))
    for (x, y), (sx, sy) in mapping.items():
        for dy in range(int(k)):
            for dx in range(int(k)):
                out.putpixel((int(x * k) + dx, int(y * k) + dy), image.getpixel((int(sx * k) + dx, int(sy * k) + dy)))
    return out


def sign_parts(name):
    for suffix in SUFFIXES:
        if name.endswith(suffix):
            return name[: -len(suffix)], suffix
    return None, None


def words(s):
    return frozenset(w for w in re.split(r"[_/]", s) if w)


def find_texture(textures, hanging, wood):
    """The mod's entity sign texture for this wood: the same words, in any order."""
    folder = textures / "entity" / "signs"
    if hanging:
        folder = folder / "hanging"
    if not folder.is_dir():
        return None
    want = words(wood)
    for png in sorted(folder.glob("*.png")):
        if words(png.stem) == want:
            return png
    for png in sorted(folder.glob("*.png")):
        if want <= words(png.stem) or words(png.stem) <= want:
            return png
    return None


def signs(mod, ns, client_jar, resources, maps):
    root = mod / "assets" / ns
    states = root / "blockstates"
    count = 0
    for file in sorted(states.glob("*_sign.json")) if states.is_dir() else []:
        name = file.stem
        wood, suffix = sign_parts(name)
        if wood is None:
            continue
        hanging = "hanging" in suffix
        mapping, sizes = maps[hanging]
        source = find_texture(root / "textures", hanging, wood)
        vanilla_state = resources / f"assets/minecraft/blockstates/oak{suffix}.json"
        if mapping is None or source is None or not vanilla_state.exists():
            continue
        texture = f"{ns}:block/{name}_horizon"
        out = root / "textures" / "block" / f"{name}_horizon.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        convert(Image.open(source), mapping, sizes).save(out)
        particle = None
        try:
            old = json.loads(file.read_text())
            first = next(iter(old.get("variants", {}).values()), {})
            first = first[0] if isinstance(first, list) else first
            model = first.get("model", "")
            mns, _, mpath = model.partition(":") if ":" in model else ("minecraft", "", model)
            particle = json.loads((mod / f"assets/{mns}/models/{mpath}.json").read_text())["textures"]["particle"]
        except (OSError, ValueError, KeyError, StopIteration, AttributeError):
            pass
        text = vanilla_state.read_text().replace(f"minecraft:block/oak{suffix}", f"{ns}:block/{name}")
        file.write_text(text)
        for ref in set(re.findall(rf'"{ns}:block/({re.escape(name)}[a-z0-9_]*)"', text)):
            vanilla_model = resources / f"assets/minecraft/models/block/{ref.replace(name, 'oak' + suffix, 1)}.json"
            if not vanilla_model.exists():
                continue
            v = json.loads(vanilla_model.read_text())
            v.setdefault("textures", {})["all"] = texture
            if particle:
                v["textures"]["particle"] = particle
            target = root / "models" / "block" / f"{ref}.json"
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(json.dumps(v, indent=2))
        count += 1
    return count


def chests(mod, ns):
    root = mod / "assets" / ns
    states = root / "blockstates"
    textures = root / "textures"
    pngs = [p for p in textures.rglob("*.png")] if textures.is_dir() else []
    count = 0
    for file in sorted(states.glob("*chest.json")) if states.is_dir() else []:
        name = file.stem
        trapped = "trapped" in name
        wood = name.replace("trapped_", "").replace("_chest", "")
        want = words(wood)
        found = {}
        for part, stems in [("", ["trap" if trapped else "normal", wood]), ("_left", ["trap_left" if trapped else "left", f"{wood}_left"]),
                            ("_right", ["trap_right" if trapped else "right", f"{wood}_right"])]:
            for p in pngs:
                if p.stem in stems and (want <= words(p.parent.relative_to(textures).as_posix()) or p.stem.startswith(wood)):
                    found[part] = p
                    break
        if "" not in found:
            continue
        for part, p in found.items():
            out = textures / "entity" / "chest" / f"{name}{part}.png"
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(p.read_bytes())
        count += 1
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client-jar", type=Path, required=True, help="Minecraft 1.21.1's client jar")
    parser.add_argument("--resources", type=Path, required=True, help="Horizon's prepared resources")
    parser.add_argument("--mods-dir", type=Path, required=True, help="GAME_HOME/horizon-mods")
    args = parser.parse_args()
    maps = {h: learn(args.client_jar, args.resources, h) for h in (False, True)}
    for h, (m, _) in maps.items():
        print(f"{'hanging ' if h else ''}sign layout: {len(m or {})} pixels learnt")
    for mod in sorted(p for p in args.mods_dir.iterdir() if (p / "assets").is_dir()):
        for ns_dir in sorted((mod / "assets").iterdir()):
            ns = ns_dir.name
            s, c = signs(mod, ns, args.client_jar, args.resources, maps), chests(mod, ns)
            if s or c:
                print(f"{mod.name}: {s} signs, {c} chests")


if __name__ == "__main__":
    main()
