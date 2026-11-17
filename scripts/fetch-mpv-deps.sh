#!/bin/sh
# fetch-mpv-deps.sh — vendor libmpv headers + shared lib for no-sudo builds.
#
# Downloads the Ubuntu noble libmpv (0.37, matching this system's ffmpeg
# generation) via `apt download` (no root needed) and extracts it with
# dpkg-deb -x into .deps/mpv/. Then export the variables printed at the end
# before building/testing the app crate.
#
# rsmpv needs: pkg-config file (libmpv.pc) or MPV_LIBRARY_DIR override, plus
# libmpv.so at runtime (LD_LIBRARY_PATH).
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
OUT="$ROOT/.deps/mpv"
LIBDIR="$OUT/usr/lib/x86_64-linux-gnu"
PIN=0.37.0-1ubuntu4

if [ -f "$LIBDIR/libmpv.so.2.0.0" ] || ls "$LIBDIR"/libmpv.so.* >/dev/null 2>&1; then
    echo "libmpv already present in $LIBDIR (delete $OUT to refetch)"
else
    TMP=$(mktemp -d)
    trap 'rm -rf "$TMP"' EXIT
    echo "Downloading libmpv-dev/libmpv2 $PIN (+ libxpresent1 runtime dep) ..."
    (cd "$TMP" && apt download "libmpv-dev=$PIN" "libmpv2=$PIN" "libxpresent1")
    mkdir -p "$OUT"
    for deb in "$TMP"/*.deb; do
        echo "Extracting $(basename "$deb")"
        dpkg-deb -x "$deb" "$OUT"
    done
fi

echo
echo "Add to your environment before cargo build/test of the app crate:"
echo "  export PKG_CONFIG_PATH=\"\${PKG_CONFIG_PATH:+\$PKG_CONFIG_PATH:}$LIBDIR/pkgconfig\""
echo "  export MPV_LIBRARY_DIR=\"$LIBDIR\""
echo "  export LD_LIBRARY_PATH=\"\${LD_LIBRARY_PATH:+\$LD_LIBRARY_PATH:}$LIBDIR\""
