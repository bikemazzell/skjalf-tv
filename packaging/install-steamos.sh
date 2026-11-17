#!/bin/sh
# Install Skjalf TV for one user. Run from an unpacked source/build directory.
set -eu

SRC=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BIN=${1:-$SRC/target/release/skjalf}
INSTALL_HOME=${SKJALF_INSTALL_HOME:-$HOME}
case "$INSTALL_HOME" in
    /*) ;;
    *) echo "error: install home must be an absolute path" >&2; exit 1 ;;
esac
if [ "$INSTALL_HOME" = / ]; then
    echo "error: install home cannot be /" >&2
    exit 1
fi
if [ ! -x "$BIN" ]; then
    echo "error: release binary not found at $BIN" >&2
    exit 1
fi

# Editing a live Steam account risks Steam replacing shortcuts.vdf on exit.
if [ "$INSTALL_HOME" = "$HOME" ] && command -v pgrep >/dev/null 2>&1; then
    if pgrep -u "$(id -u)" -x steam >/dev/null || pgrep -u "$(id -u)" -x steamwebhelper >/dev/null; then
        echo "error: exit Steam before installing its shortcut" >&2
        exit 1
    fi
fi

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT HUP INT TERM
if [ -n "${SKJALF_MPV_LIB_DIR:-}" ]; then
    VENDORED=$SKJALF_MPV_LIB_DIR
else
    sh "$SRC/packaging/fetch-steamos-runtime.sh" "$STAGE/runtime"
    VENDORED=$STAGE/runtime/lib
fi
python3 "$SRC/packaging/check-deps.py" "$BIN" "$VENDORED" "$STAGE" > "$STAGE/dependency-check.log"
DEST="$INSTALL_HOME/.local/share/skjalf"
APPLICATIONS="$INSTALL_HOME/.local/share/applications"
mkdir -p "$DEST/lib" "$APPLICATIONS"
cp "$BIN" "$DEST/skjalf"
cp "$SRC/packaging/skjalf-wrapper.sh" "$DEST/skjalf.sh"
chmod +x "$DEST/skjalf.sh"
cp "$SRC/packaging/update-shortcuts.py" "$DEST/update-shortcuts.py"
cp "$SRC/packaging/uninstall-steamos.sh" "$DEST/uninstall.sh"
chmod +x "$DEST/uninstall.sh"
cp -P "$STAGE"/lib/* "$DEST/lib/"
cp "$STAGE/dependencies.txt" "$DEST/dependencies.txt"

ICONS="$SRC/packaging/icons"
THEME="$INSTALL_HOME/.local/share/icons/hicolor"
for size in 32 48 64 128 256 512; do
    mkdir -p "$THEME/${size}x${size}/apps"
    cp "$ICONS/skjalf-$size.png" "$THEME/${size}x${size}/apps/skjalf.png"
done
mkdir -p "$THEME/scalable/apps"
cp "$ICONS/skjalf.svg" "$THEME/scalable/apps/skjalf.svg"
cp "$ICONS/skjalf-256.png" "$DEST/skjalf.png"

python3 - "$SRC/packaging/skjalf.desktop" "$DEST" "$APPLICATIONS/skjalf.desktop" <<'PY'
from pathlib import Path
import sys
template = Path(sys.argv[1]).read_text()
install_dir = sys.argv[2].replace('\\', '\\\\').replace('"', '\\"')
Path(sys.argv[3]).write_text(template.replace('@INSTALL_DIR@', install_dir))
PY

STEAM_USERDATA="$INSTALL_HOME/.local/share/Steam/userdata"
if [ -d "$STEAM_USERDATA" ]; then
    for cfg in "$STEAM_USERDATA"/*/config; do
        [ -d "$cfg" ] || continue
        [ "$(basename "$(dirname "$cfg")")" = 0 ] && continue
        python3 "$SRC/packaging/update-shortcuts.py" "$cfg/shortcuts.vdf" "$DEST"
        echo "Steam shortcut updated: $cfg/shortcuts.vdf"
    done
else
    echo "Steam userdata not found; add Skjalf TV as a non-Steam game after install."
fi

echo "Installed to $DEST"
echo "Runtime inventory: $DEST/dependencies.txt"
