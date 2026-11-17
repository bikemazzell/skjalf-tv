#!/bin/sh
# Skjalf TV one-line installer, published as install.sh with each release:
#   curl -fsSL https://github.com/bikemazzell/skjalf-tv/releases/latest/download/install.sh | sh
set -eu

BASE_URL=${SKJALF_RELEASE_URL:-https://github.com/bikemazzell/skjalf-tv/releases/latest/download}
TARBALL=skjalf-tv-x86_64.tar.gz
WAIT_SECONDS=${SKJALF_STEAM_WAIT:-60}

die() {
    echo "error: $*" >&2
    exit 1
}

if [ "$(id -u)" -eq 0 ]; then
    die "do not run this as root; run it as your normal user"
fi

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        die "curl or wget is required"
    fi
}

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
    echo "Steam is running and will close so the shortcut can be added."
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

# The shell reads this whole function before main runs, so a truncated download cannot run partially.
main() {
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

echo "Downloading Skjalf TV..."
fetch "$BASE_URL/$TARBALL" "$TMP/$TARBALL"
fetch "$BASE_URL/SHA256SUMS" "$TMP/SHA256SUMS"
grep " $TARBALL\$" "$TMP/SHA256SUMS" > "$TMP/tarball.sha256" || die "no checksum listed for $TARBALL"
(cd "$TMP" && sha256sum -c tarball.sha256 >/dev/null 2>&1) || die "checksum mismatch for $TARBALL; nothing was installed"

close_steam

tar -xzf "$TMP/$TARBALL" -C "$TMP"
sh "$TMP/skjalf-tv/packaging/install-steamos.sh" "$TMP/skjalf-tv/skjalf"

cat <<'MSG'

Skjalf TV is installed.
Next steps:
  1. Start Steam, or return to Game Mode.
  2. Open the Library and find Skjalf TV under Non-Steam.
  3. On a TV, set Game resolution to Native in the Steam Deck display settings.
MSG
}

main "$@"
