#!/bin/sh
# Print the GitHub release notes for a version: its section of CHANGELOG.md,
# then the install line. A version without a changelog section fails, so a
# release cannot ship without one.
#
# Usage: packaging/release-notes.sh VERSION [CHANGELOG]
set -eu

version="$1"
changelog="${2:-$(dirname "$0")/../CHANGELOG.md}"

changes=$(awk -v v="$version" '
    /^## / { found = ($2 == v); next }
    found { if (NF) started = 1; if (started) print }
' "$changelog")

if [ -z "$changes" ]; then
    echo "error: $changelog has no section for $version" >&2
    exit 1
fi

printf '%s\n\nInstall on a Steam Deck in Desktop Mode:\n\n    curl -fsSL https://github.com/bikemazzell/skjalf-tv/releases/latest/download/install.sh | sh\n\nVerify downloads against SHA256SUMS.\n' "$changes"
