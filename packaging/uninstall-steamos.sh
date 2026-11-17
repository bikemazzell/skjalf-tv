#!/bin/sh
# Remove Skjalf TV for one user. Usage: uninstall-steamos.sh [--purge]
# --purge also removes the settings directory, which holds the source credentials.
set -eu

PURGE=0
for arg in "$@"; do
    case "$arg" in
        --purge) PURGE=1 ;;
        *) echo "usage: uninstall-steamos.sh [--purge]" >&2; exit 1 ;;
    esac
done

WAIT_SECONDS=${SKJALF_STEAM_WAIT:-60}

die() {
    echo "error: $*" >&2
    exit 1
}

if [ "$(id -u)" -eq 0 ]; then
    die "do not run this as root; run it as your normal user"
fi

INSTALL_HOME=${SKJALF_INSTALL_HOME:-$HOME}
case "$INSTALL_HOME" in
    /*) ;;
    *) die "install home must be an absolute path" ;;
esac
[ "$INSTALL_HOME" != / ] || die "install home cannot be /"

steam_running() {
    pgrep -u "$(id -u)" -x steam >/dev/null 2>&1 || pgrep -u "$(id -u)" -x steamwebhelper >/dev/null 2>&1
}

game_mode() {
    pgrep -u "$(id -u)" -x gamescope >/dev/null 2>&1 \
        || systemctl --user is-active --quiet 'gamescope-session*' >/dev/null 2>&1
}

close_steam() {
    steam_running || return 0
    if game_mode; then
        die "Steam is running in Game Mode. Switch to Desktop Mode (Steam menu, Power, Switch to Desktop) and run this again."
    fi
    echo "Steam is running and will close so the shortcut can be removed."
    steam -shutdown >/dev/null 2>&1 || true
    waited=0
    while steam_running; do
        if [ "$waited" -ge "$WAIT_SECONDS" ]; then
            die "Steam did not close within $WAIT_SECONDS seconds. Close it and run this again."
        fi
        sleep 1
        waited=$((waited + 1))
    done
}

DEST="$INSTALL_HOME/.local/share/skjalf"
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd) || HERE=
SHORTCUTS=
for candidate in "$HERE/update-shortcuts.py" "$DEST/update-shortcuts.py"; do
    if [ -f "$candidate" ]; then
        SHORTCUTS=$candidate
        break
    fi
done

close_steam

STEAM_USERDATA="$INSTALL_HOME/.local/share/Steam/userdata"
if [ -d "$STEAM_USERDATA" ]; then
    for user in "$STEAM_USERDATA"/*; do
        [ -d "$user/config" ] || continue
        [ "$(basename "$user")" != 0 ] || continue
        [ -f "$user/config/shortcuts.vdf" ] || continue
        if [ -n "$SHORTCUTS" ]; then
            python3 "$SHORTCUTS" --remove "$user/config/shortcuts.vdf"
        else
            echo "warning: update-shortcuts.py not found; the Steam shortcut was not removed" >&2
        fi
    done
fi

ICONS="$INSTALL_HOME/.local/share/icons/hicolor"
for size in 32 48 64 128 256 512; do
    rm -f "$ICONS/${size}x${size}/apps/skjalf.png"
done
rm -f "$ICONS/scalable/apps/skjalf.svg"
rm -f "$INSTALL_HOME/.local/share/applications/skjalf.desktop"
rm -rf "$DEST"

CONFIG="${XDG_CONFIG_HOME:-$INSTALL_HOME/.config}/skjalf"
if [ "$PURGE" -eq 1 ]; then
    rm -rf "$CONFIG"
    echo "Removed $CONFIG. It held your source credentials."
elif [ -d "$CONFIG" ]; then
    echo "Kept $CONFIG. It holds your source credentials; run with --purge to remove it."
fi
echo "Skjalf TV is removed."
