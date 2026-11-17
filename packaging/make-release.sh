#!/bin/sh
# Build the release assets from a built binary: tarball, install.sh,
# uninstall.sh, and SHA256SUMS. Usage: make-release.sh <binary> <outdir>
set -eu

if [ $# -ne 2 ]; then
    echo "usage: make-release.sh <binary> <outdir>" >&2
    exit 1
fi
BIN=$1
OUT=$2
SRC=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
[ -x "$BIN" ] || { echo "error: binary not found at $BIN" >&2; exit 1; }

VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$SRC/Cargo.toml" | head -n 1)
[ -n "$VERSION" ] || { echo "error: workspace version not found" >&2; exit 1; }

mkdir -p "$OUT"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT HUP INT TERM
TOP="$STAGE/skjalf-tv"
mkdir -p "$TOP"
cp "$BIN" "$TOP/skjalf"
chmod 755 "$TOP/skjalf"
cp -R "$SRC/packaging" "$TOP/packaging"
rm -rf "$TOP/packaging/tests"
find "$TOP" -name __pycache__ -type d -exec rm -rf {} +
cp "$SRC/README.md" "$SRC/LICENSE" "$TOP/"
printf '%s\n' "$VERSION" > "$TOP/VERSION"

tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 -C "$STAGE" -cf - skjalf-tv \
    | gzip -n > "$OUT/skjalf-tv-x86_64.tar.gz"
cp "$SRC/packaging/bootstrap-install.sh" "$OUT/install.sh"
cp "$SRC/packaging/uninstall-steamos.sh" "$OUT/uninstall.sh"
(cd "$OUT" && sha256sum skjalf-tv-x86_64.tar.gz install.sh uninstall.sh > SHA256SUMS)
echo "Release assets for $VERSION in $OUT"
