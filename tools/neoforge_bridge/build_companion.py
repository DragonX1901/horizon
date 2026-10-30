#!/usr/bin/env python3
"""Build the Horizon bridge companion mod (mods/horizon-bridge-neoforge) into a jar.

It compiles with javac against an installed NeoForge 21.1 server's libraries, so no Gradle
or Minecraft download is needed beyond the server you already run:

    python3 tools/neoforge_bridge/build_companion.py --server-dir path/to/neoforge-server

Put the jar in that server's mods/ folder. Clients never need it.
"""

import argparse
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MOD = ROOT / "mods/horizon-bridge-neoforge"


def classpath(server_dir):
    libraries = server_dir / "libraries"
    patched = sorted(libraries.glob("net/neoforged/neoforge/*/neoforge-*-server.jar"))
    universal = sorted(libraries.glob("net/neoforged/neoforge/*/neoforge-*-universal.jar"))
    if not patched or not universal:
        raise SystemExit(f"No installed NeoForge server under {server_dir} (run the installer with --installServer)")
    # NeoForge's patched classes first, then the rest of Minecraft, Mojang-named (the
    # installer's -srg jar). The obfuscated vanilla jar beside it is left out.
    vanilla = sorted(libraries.glob("net/minecraft/server/*/server-*-srg.jar"))
    rest = [p for p in sorted(libraries.rglob("*.jar")) if "net/minecraft/server" not in p.as_posix()]
    jars = [patched[-1], *vanilla[-1:], universal[-1]] + [p for p in rest if p not in (patched[-1], universal[-1])]
    return ":".join(str(p) for p in jars)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--server-dir", type=Path, required=True)
    parser.add_argument("--out", type=Path, default=ROOT / "target/horizon-bridge-neoforge-0.1.0.jar")
    args = parser.parse_args()
    sources = sorted(str(p) for p in (MOD / "src/main/java").rglob("*.java"))
    with tempfile.TemporaryDirectory() as tmp:
        classes = Path(tmp) / "classes"
        subprocess.run(
            ["javac", "--release", "21", "-nowarn", "-proc:none", "-cp", classpath(args.server_dir.resolve()),
             "-d", str(classes), *sources],
            check=True,
        )
        shutil.copytree(MOD / "src/main/resources", classes, dirs_exist_ok=True)
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.unlink(missing_ok=True)
        subprocess.run(["jar", "--create", "--file", str(args.out), "-C", str(classes), "."], check=True)
    print(args.out)


if __name__ == "__main__":
    main()
