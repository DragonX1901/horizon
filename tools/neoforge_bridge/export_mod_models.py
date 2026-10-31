#!/usr/bin/env python3
"""Bake the entity models mods define in Java into a modded world's content mods.

    export_mod_models.py --client-dir CLIENT --server-dir SERVER --mods DIR --home GAME_HOME

Mobs whose models are Java code (`createBodyLayer` returning a LayerDefinition)
have no model file Horizon could read. This runs ModModelExporter on NeoForge's
Mojang-named 1.21.1 client classes (CLIENT: a launcher folder where the NeoForge
installer ran with --installClient), matches each layer to the entity it draws
by name (`BlackBearModel` -> `black_bear`), and writes them per mod to
`GAME_HOME/horizon-mods/<mod>/assets/<ns>/horizon/entity_models.json`, which the
game reads for mobs with no GeckoLib model (crates/horizon-client/src/mobs/bedrock.rs).
"""

import argparse
import json
import re
import subprocess
import sys
import tempfile
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "mods/horizon-bridge-neoforge/src/exporter/java/dev/horizonlab/bridge/ModModelExporter.java"


def version_key(v):
    return [int(x) if x.isdigit() else 0 for x in re.split(r"[.\-+]", v)]


def classpath(client, server):
    """NeoForge's Mojang-named client, NeoForge, then each library once, newest version."""
    libs = client / "libraries"
    mc = sorted(libs.glob("net/minecraft/client/*/client-*-srg.jar"))
    universal = sorted(libs.glob("net/neoforged/neoforge/*/neoforge-*-universal.jar"))
    if not mc or not universal:
        sys.exit(f"no NeoForge client installed in {client} (run the installer with --installClient)")
    best = {}
    for root in (libs, server / "libraries"):
        for jar in sorted(root.rglob("*.jar")):
            posix = jar.as_posix()
            if "net/minecraft/" in posix or "net/neoforged/neoforge/" in posix:
                continue
            artifact, version = jar.parent.parent, jar.parent.name
            key = artifact.relative_to(root).as_posix()
            if key not in best or version_key(version) > version_key(best[key][0]):
                best[key] = (version, jar)
    return [mc[-1], universal[-1]] + [jar for _, jar in best.values()]


def snake(name):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def entities_of(jar):
    """(mod id, the entity names its language file names)."""
    with zipfile.ZipFile(jar) as z:
        names = z.namelist()
        toml = next((n for n in ("META-INF/neoforge.mods.toml", "META-INF/mods.toml") if n in names), None)
        if toml is None:
            return None, set()
        mods = tomllib.loads(z.read(toml).decode("utf-8", "replace")).get("mods", [])
        mod_id = mods[0].get("modId", "") if mods else ""
        lang = f"assets/{mod_id}/lang/en_us.json"
        entities = set()
        if lang in names:
            for key in json.loads(z.read(lang)):
                parts = key.split(".")
                if len(parts) == 3 and parts[0] == "entity" and parts[1] == mod_id:
                    entities.add(parts[2])
        return mod_id, entities


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client-dir", type=Path, required=True)
    parser.add_argument("--server-dir", type=Path, required=True)
    parser.add_argument("--mods", type=Path, required=True)
    parser.add_argument("--home", type=Path, required=True)
    args = parser.parse_args()
    jars = sorted(args.mods.resolve().glob("*.jar"))
    cp = classpath(args.client_dir.resolve(), args.server_dir.resolve())
    with tempfile.TemporaryDirectory() as tmp:
        classes = Path(tmp) / "classes"
        joined = ":".join(str(p) for p in cp)
        subprocess.run(["javac", "--release", "21", "-nowarn", "-proc:none", "-cp", joined, "-d", classes, SOURCE],
                       check=True)
        out = Path(tmp) / "layers.json"
        subprocess.run(["java", "-cp", f"{classes}:{joined}", "dev.horizonlab.bridge.ModModelExporter", out,
                        *jars], check=True)
        layers = json.loads(out.read_text())
    total = 0
    for jar in jars:
        mod_id, entities = entities_of(jar)
        if not mod_id or not entities:
            continue
        with zipfile.ZipFile(jar) as z:
            classes_in = {n[:-6].replace("/", ".") for n in z.namelist() if n.endswith(".class")}
        models = {}
        for key, part in layers.items():
            cls, method = key.split("#")
            if cls not in classes_in or "baby" in cls.lower():
                continue
            base = re.sub(r"(Model|Renderer|Layer)$", "", cls.rsplit(".", 1)[-1].split("$")[-1])
            name = snake(base)
            if name in entities and (name not in models or method == "createBodyLayer"):
                models[name] = part
        target = args.home / "horizon-mods" / mod_id
        if not models or not target.exists():
            continue
        path = target / "assets" / mod_id / "horizon" / "entity_models.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({f"{mod_id}:{n}": p for n, p in models.items()}))
        total += len(models)
        print(f"{mod_id}: {len(models)} entity models ({', '.join(sorted(models)[:8])}...)")
    print(f"{total} entity models from {len(layers)} layers")


if __name__ == "__main__":
    main()
