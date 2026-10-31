#!/usr/bin/env python3
"""Make a modded single-player world from a folder of NeoForge 1.21.1 mods.

    setup_world.py --mods DIR --home GAME_HOME --world NAME [--java PATH] [--neoforge 21.1.N]
    setup_world.py --mods DIR --home GAME_HOME --join NAME   # a server's mods, for multiplayer

The mods run in Java on a NeoForge server that the game starts for the world
(horizon-neoforge-host, docs/neoforge-bridge.md); Horizon draws them. This:

1. installs a NeoForge 21.1 server for the world (`GAME_HOME/neoforge/NAME`),
   with the mods and the bridge's companion mod in its `mods/`;
2. runs the NeoForge oracle on the mods and carries each one over into a
   content mod that keeps all of its blocks (HORIZON_PORT_SERVER_BLOCKS=1);
3. installs and enables those content mods in GAME_HOME, and creates
   `saves/NAME` with its `horizon-neoforge.txt`.

The jars are yours; nothing here is shipped or committed. Run it from the
repository with the fast-profile tools built (`cargo build --profile fast
-p horizon-port-pack -p horizon-client -p horizon-neoforge-bridge`) and
Horizon's resources prepared (`.local/horizon-client/26.3`).

With --join, the mods are a NeoForge 1.21.1 server's: steps 1 and 2 run (the
server folder only serves the oracle and the companion build), the content
mods are enabled, and no world is made. Put the printed companion jar in the
server's mods/ folder; then join the server from the Multiplayer screen.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MAVEN = "https://maven.neoforged.net/releases/net/neoforged/neoforge"
BIN = ROOT / "target" / "fast"


def latest_neoforge():
    with urllib.request.urlopen(f"{MAVEN}/maven-metadata.xml", timeout=60) as r:
        text = r.read().decode()
    versions = [v for v in text.split("<version>")[1:]]
    versions = [v.split("</version>")[0] for v in versions]
    versions = [v for v in versions if v.startswith("21.1.") and v.split(".")[2].isdigit()]
    return max(versions, key=lambda v: int(v.split(".")[2]))


def install_server(server, version, java):
    args = server / f"libraries/net/neoforged/neoforge/{version}/unix_args.txt"
    if args.exists():
        return
    server.mkdir(parents=True, exist_ok=True)
    installer = server / f"neoforge-{version}-installer.jar"
    if not installer.exists():
        print(f"downloading NeoForge {version}")
        urllib.request.urlretrieve(f"{MAVEN}/{version}/neoforge-{version}-installer.jar", installer)
    subprocess.run([java, "-jar", installer, "--installServer", server], cwd=server, check=True,
                   stdout=subprocess.DEVNULL)


def run(*command, env=None):
    print("+", " ".join(str(c) for c in command))
    subprocess.run([str(c) for c in command], check=True, env=env)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--mods", type=Path, required=True, help="a folder of NeoForge 1.21.1 mod jars")
    parser.add_argument("--home", type=Path, required=True, help="the game's home (HORIZON_HOME)")
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--world", help="the world's folder name")
    target.add_argument("--join", help="a name for a server's mods: content mods only, no world")
    parser.add_argument("--java", default="java", help="the Java to run the server with (21 for Cobblemon)")
    parser.add_argument("--neoforge", help="the NeoForge 21.1 version (default: the newest)")
    parser.add_argument("--resources", type=Path, default=ROOT / ".local/horizon-client/26.3")
    parser.add_argument("--client-dir", type=Path,
                        help="a launcher folder with NeoForge's 1.21.1 client installed (--installClient): "
                             "bakes the entity models mods define in Java")
    args = parser.parse_args()

    mods = args.mods.resolve()
    home = args.home.resolve()
    jars = sorted(mods.glob("*.jar"))
    if not jars:
        sys.exit(f"no jars in {mods}")
    version = args.neoforge or latest_neoforge()
    name = args.world or f"server-{args.join}"
    server = home / "neoforge" / name
    work = home / "neoforge" / f"{name}-port"

    # 1. The server, its mods and the companion.
    install_server(server, version, args.java)
    (server / "mods").mkdir(exist_ok=True)
    for jar in jars:
        shutil.copy2(jar, server / "mods" / jar.name)
    companion = server / "mods" / "horizon-bridge-neoforge.jar"
    run(sys.executable, ROOT / "tools/neoforge_bridge/build_companion.py", "--server-dir", server,
        "--out", companion)

    # 2. The oracle's facts and the content mods.
    oracle_home = ROOT / ".local" / "neoforge-oracle" / version
    if not oracle_home.exists():
        oracle_home.parent.mkdir(parents=True, exist_ok=True)
        oracle_home.symlink_to(server)
    shutil.rmtree(work, ignore_errors=True)
    run(BIN / "horizon-port-pack", "import", mods, work)
    run(BIN / "horizon-port-pack", "inventory", work)
    env = dict(os.environ, HORIZON_NEOFORGE=version)
    run(sys.executable, ROOT / "tools/neoforge_oracle.py", "export", "--jars", mods,
        "--out", work / "oracle" / "facts.json", env=env)
    records = json.loads((work / "inventory.json").read_text())
    keys = [r["key"] for r in records if r.get("key")]
    run(BIN / "horizon-port-pack", "fetch", work, "--from", mods, *keys)
    env = dict(os.environ, HORIZON_PORT_SERVER_BLOCKS="1")
    run(BIN / "horizon-port-pack", "carry", work, "--resources", args.resources, *keys, env=env)

    # 3. The game: content mods on, and the world.
    installed = []
    for carried in sorted((work / "mods").glob("*/carry")):
        manifest = (carried / "horizon-mod.toml").read_text()
        mod_id = next(line.split("=", 1)[1].strip().strip('"') for line in manifest.splitlines()
                      if line.startswith("id ="))
        target = home / "horizon-mods" / mod_id
        shutil.rmtree(target, ignore_errors=True)
        shutil.copytree(carried, target)
        installed.append(mod_id)
    listing = home / "horizon-mods.txt"
    lines = listing.read_text().splitlines() if listing.exists() else []
    lines = [l for l in lines if l.split(":")[0] not in {f"mod.{m}" for m in installed}]
    lines += [f"mod.{m}:true" for m in installed]
    listing.write_text("\n".join(lines) + "\n")
    if args.client_dir:
        run(sys.executable, ROOT / "tools/neoforge_bridge/export_mod_models.py", "--client-dir", args.client_dir,
            "--server-dir", server, "--mods", mods, "--home", home)
    if args.join:
        print(f"\n{args.join}: NeoForge {version}, {len(jars)} jars, content mods {', '.join(installed) or 'none'}.")
        print(f"Put {companion} in the server's mods/ folder, then join it from the Multiplayer screen.")
        return
    world = home / "saves" / args.world
    world.mkdir(parents=True, exist_ok=True)
    marker = f"{server}\n" + (f"java={args.java}\n" if args.java != "java" else "")
    (world / "horizon-neoforge.txt").write_text(marker)
    print(f"\n{args.world}: NeoForge {version}, {len(jars)} jars, content mods {', '.join(installed) or 'none'}.")
    print(f"Open it from Horizon's Singleplayer screen (game home {home}).")


if __name__ == "__main__":
    main()
