#!/usr/bin/env python3
"""Add Skjalf TV to one Steam account's shortcuts.vdf without losing other entries."""

import argparse
import os
from pathlib import Path
import shutil
import struct
import tempfile
import zlib


def cstring(data, pos):
    end = data.index(b"\0", pos)
    return data[pos:end].decode("utf-8"), end + 1


def read_map(data, pos):
    result = {}
    while pos < len(data):
        kind = data[pos]
        pos += 1
        if kind == 8:
            return result, pos
        key, pos = cstring(data, pos)
        if kind == 0:
            value, pos = read_map(data, pos)
        elif kind == 1:
            value, pos = cstring(data, pos)
        elif kind == 2:
            value = struct.unpack_from("<i", data, pos)[0]
            pos += 4
        else:
            raise ValueError(f"unsupported VDF value type {kind}")
        result[key] = value
    raise ValueError("shortcuts.vdf is incomplete")


def read_vdf(data):
    if not data:
        return []
    if data[0] != 0:
        raise ValueError("shortcuts.vdf has an unknown root")
    name, pos = cstring(data, 1)
    if name != "shortcuts":
        raise ValueError("shortcuts.vdf has an unknown root")
    root, pos = read_map(data, pos)
    # Valve's binary VDF has one terminator for the root map and one for
    # the outer object. Accept an older one-terminator file too.
    if pos + 1 == len(data) and data[pos] == 8:
        pos += 1
    if pos != len(data):
        raise ValueError("shortcuts.vdf has trailing data")
    return [root[key] for key in sorted(root, key=int)]


def encode_map(mapping):
    out = bytearray()
    for key, value in mapping.items():
        key_bytes = key.encode() + b"\0"
        if isinstance(value, dict):
            out += b"\0" + key_bytes + encode_map(value) + b"\x08"
        elif isinstance(value, int):
            out += b"\x02" + key_bytes + struct.pack("<i", value)
        else:
            out += b"\x01" + key_bytes + value.encode() + b"\0"
    return out


def write_vdf(shortcuts):
    out = bytearray(b"\x00shortcuts\x00")
    for index, shortcut in enumerate(shortcuts):
        out += b"\x00" + str(index).encode() + b"\x00" + encode_map(shortcut) + b"\x08"
    return bytes(out + b"\x08\x08")


def atomic_write(path, data):
    fd, temp_path = tempfile.mkstemp(prefix=".shortcuts.", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp_path, path)
    finally:
        if os.path.exists(temp_path):
            os.unlink(temp_path)

ARTWORK_DIR = Path(__file__).resolve().parent / "icons"
GRID_FILES = {"capsule.png": "{}p.png", "header.png": "{}.png", "hero.png": "{}_hero.png", "logo.png": "{}_logo.png"}
APP_NAME = "Skjalf TV"
LEGACY_APP_NAME = "SteamTV"


def install_grid_artwork(grid, appid, artwork):
    grid.mkdir(parents=True, exist_ok=True)
    for source, pattern in GRID_FILES.items():
        shutil.copyfile(artwork / source, grid / pattern.format(appid & 0xFFFFFFFF))


def update(vdf, executable, install_dir, artwork=ARTWORK_DIR):
    old = vdf.read_bytes() if vdf.exists() else b""
    existing = read_vdf(old)
    quoted_exe = f'"{executable}"'
    checksum = zlib.crc32((quoted_exe + APP_NAME).encode()) | 0x80000000
    appid = checksum - (1 << 32) if checksum >= 1 << 31 else checksum
    entry = {
        "appid": appid, "AppName": APP_NAME, "Exe": quoted_exe,
        "StartDir": f'"{install_dir}"', "icon": "", "ShortcutPath": "",
        "LaunchOptions": "", "IsHidden": 0, "AllowDesktopConfig": 1,
        "AllowOverlay": 1, "openvr": 0, "DevKit": 0,
        "DevKitGameID": "", "DevKitOverrideAppID": 0,
        "LastPlayTime": 0, "tags": {},
    }
    owned = {
        "appid": appid, "AppName": APP_NAME, "Exe": quoted_exe,
        "StartDir": f'"{install_dir}"', "icon": str(Path(install_dir) / "skjalf.png"),
    }
    previous = next((item for item in existing if item.get("AppName") == APP_NAME), None)
    if previous is None:
        previous = next((item for item in existing if item.get("AppName") == LEGACY_APP_NAME), None)
    entry = {**(previous if previous is not None else entry), **owned}
    updated = [item for item in existing if item.get("AppName") not in (APP_NAME, LEGACY_APP_NAME)] + [entry]
    data = write_vdf(updated)
    if old:
        backup = vdf.with_name(vdf.name + ".bak")
        atomic_write(backup, old)
    atomic_write(vdf, data)
    install_grid_artwork(vdf.parent / "grid", appid, artwork)


def remove(vdf):
    if not vdf.exists():
        return
    old = vdf.read_bytes()
    existing = read_vdf(old)
    ours = [item for item in existing if item.get("AppName") == APP_NAME]
    if not ours:
        return
    atomic_write(vdf.with_name(vdf.name + ".bak"), old)
    atomic_write(vdf, write_vdf([item for item in existing if item.get("AppName") != APP_NAME]))
    grid = vdf.parent / "grid"
    for item in ours:
        appid = item.get("appid")
        if isinstance(appid, int):
            for pattern in GRID_FILES.values():
                (grid / pattern.format(appid & 0xFFFFFFFF)).unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--remove", action="store_true", help="remove the Skjalf TV entry and its artwork")
    parser.add_argument("vdf", type=Path)
    parser.add_argument("install_dir", type=Path, nargs="?")
    args = parser.parse_args()
    if args.remove:
        remove(args.vdf)
    elif args.install_dir is None:
        parser.error("install_dir is required")
    else:
        update(args.vdf, args.install_dir / "skjalf.sh", args.install_dir)


if __name__ == "__main__":
    main()
