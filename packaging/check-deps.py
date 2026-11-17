#!/usr/bin/env python3
"""Stage bundled libmpv libraries and list the host libraries still required."""

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

REQUIRED = "libmpv.so.2"
RUNTIME = {"libc.so.6", "libm.so.6", "libgcc_s.so.1", "ld-linux-x86-64.so.2"}
DRIVER_PREFIXES = ("libGL", "libEGL", "libvulkan", "libdrm", "libgbm", "libva", "libvdpau")


def inventory(binary: Path, library_dir: Path):
    env = os.environ.copy()
    env["LD_LIBRARY_PATH"] = str(library_dir)
    proc = subprocess.run(["ldd", str(binary)], env=env, capture_output=True, text=True, check=False)
    if proc.returncode:
        raise RuntimeError(f"ldd failed for {binary}: {proc.stderr.strip()}")
    resolved = {}
    missing = []
    for line in proc.stdout.splitlines():
        match = re.match(r"\s*(\S+)\s+=>\s+(\S+)", line)
        if match:
            name, path = match.groups()
            if path == "not":
                missing.append(name)
            else:
                resolved[name] = path
    return resolved, missing


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("vendor", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if not args.binary.is_file() or not args.vendor.is_dir():
        parser.error("binary or vendored libmpv directory is missing")
    library_dir = args.output / "lib"
    library_dir.mkdir(parents=True, exist_ok=True)
    sources = sorted(p for p in args.vendor.glob("*.so*") if p.is_file())
    if not any(p.name == REQUIRED for p in sources):
        raise RuntimeError(f"required bundled library missing: {args.vendor / REQUIRED}")
    for source in sources:
        target = library_dir / source.name
        if target.is_symlink() or target.exists():
            target.unlink()
        if source.is_symlink():
            target.symlink_to(os.readlink(source))
        else:
            shutil.copyfile(source, target)
    bundled = [p.name for p in sources]
    resolved, missing = inventory(args.binary, library_dir)
    for name, path in resolved.items():
        if name in bundled and Path(path).parent != library_dir:
            missing.append(f"{name} (did not resolve from bundle)")
    if REQUIRED not in resolved:
        missing.append(f"{REQUIRED} (not needed by binary)")
    if missing:
        raise RuntimeError("missing runtime dependencies:\n  " + "\n  ".join(sorted(set(missing))))
    lines = ["Bundled libraries:"]
    lines.extend(f"  {name}" for name in bundled)
    lines.append("Host libraries required on the target system:")
    for name, path in sorted(resolved.items()):
        if name in bundled:
            continue
        category = "runtime" if name in RUNTIME else "driver" if name.startswith(DRIVER_PREFIXES) else "system"
        lines.append(f"  {name} [{category}] ({path})")
    lines.append("This inventory describes the build host. Recheck dependencies on SteamOS before launch.")
    report = "\n".join(lines) + "\n"
    (args.output / "dependencies.txt").write_text(report)
    print(report, end="")


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)
