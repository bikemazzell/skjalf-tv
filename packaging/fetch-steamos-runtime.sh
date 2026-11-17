#!/bin/sh
# Fetch the pinned SteamOS libmpv runtime into OUT_DIR/lib.
# SKJALF_PACKAGE_CACHE: optional directory of downloaded packages, matched by file name.
# SKJALF_RUNTIME_LOCK: optional lock file path (tests).
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 OUT_DIR" >&2
    exit 2
fi
OUT_DIR=$1
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
LOCK=${SKJALF_RUNTIME_LOCK:-$HERE/steamos-runtime.lock}

if [ -d "$OUT_DIR/lib" ] && [ -n "$(ls -A "$OUT_DIR/lib")" ]; then
    echo "error: $OUT_DIR/lib already exists and is not empty" >&2
    exit 1
fi
if [ ! -f "$LOCK" ]; then
    echo "error: lock file not found: $LOCK" >&2
    exit 1
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
mkdir "$WORK/download" "$WORK/extract" "$WORK/tools" "$WORK/lib"
: > "$WORK/runtime.txt"
PATCHELF_PKG=
PATCHELF=$WORK/tools/usr/bin/patchelf

while read -r sha url role; do
    case "$sha" in ''|'#'*) continue ;; esac
    file=${url##*/}
    if [ -n "${SKJALF_PACKAGE_CACHE:-}" ] && [ -f "$SKJALF_PACKAGE_CACHE/$file" ]; then
        pkg=$SKJALF_PACKAGE_CACHE/$file
    else
        pkg=$WORK/download/$file
        curl -fsSL -o "$pkg" "$url" || { echo "error: download failed for $file ($url)" >&2; exit 1; }
    fi
    actual=$(sha256sum "$pkg" | cut -d' ' -f1)
    if [ "$actual" != "$sha" ]; then
        echo "error: SHA-256 mismatch for $file: expected $sha, got $actual" >&2
        exit 1
    fi
    case "$role" in
        runtime)
            dir=$WORK/extract/$file
            mkdir "$dir"
            tar --zstd -xf "$pkg" -C "$dir" --wildcards 'usr/lib/*.so*'
            find "$dir/usr/lib" -maxdepth 1 \( -type f -o -type l \) -name '*.so*' -exec cp -P {} "$WORK/lib/" \;
            info=$(tar --zstd -xOf "$pkg" .PKGINFO)
            name=$(printf '%s\n' "$info" | sed -n 's/^pkgname = //p')
            version=$(printf '%s\n' "$info" | sed -n 's/^pkgver = //p')
            printf '%s %s %s runtime\n' "$name" "$version" "$sha" >> "$WORK/runtime.txt"
            ;;
        tool)
            tar --zstd -xf "$pkg" -C "$WORK/tools" usr/bin/patchelf
            PATCHELF_PKG=$file
            info=$(tar --zstd -xOf "$pkg" .PKGINFO)
            name=$(printf '%s\n' "$info" | sed -n 's/^pkgname = //p')
            version=$(printf '%s\n' "$info" | sed -n 's/^pkgver = //p')
            printf '%s %s %s tool\n' "$name" "$version" "$sha" >> "$WORK/runtime.txt"
            ;;
        *)
            echo "error: unknown role '$role' for $file in $LOCK" >&2
            exit 1
            ;;
    esac
done < "$LOCK"

if [ -z "$PATCHELF_PKG" ]; then
    echo "error: $LOCK has no patchelf tool entry" >&2
    exit 1
fi
if [ ! -e "$WORK/lib/libmpv.so.2" ]; then
    echo "error: libmpv.so.2 missing from the runtime packages" >&2
    exit 1
fi
"$PATCHELF" --replace-needed /usr/lib/libmujs.so libmujs.so "$(readlink -f "$WORK/lib/libmpv.so.2")"

for lib in "$WORK"/lib/*; do
    [ -L "$lib" ] && continue
    needed=$("$PATCHELF" --print-needed "$lib")
    if printf '%s\n' "$needed" | grep -q '^/'; then
        echo "error: $(basename "$lib") has an absolute NEEDED entry" >&2
        exit 1
    fi
done

mkdir -p "$OUT_DIR"
[ -d "$OUT_DIR/lib" ] && rmdir "$OUT_DIR/lib"
mv "$WORK/lib" "$OUT_DIR/lib"
cp "$WORK/runtime.txt" "$OUT_DIR/runtime.txt"
echo "SteamOS runtime written to $OUT_DIR/lib"
