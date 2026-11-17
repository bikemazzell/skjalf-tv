#!/bin/sh
# Skjalf TV launcher (bundled libmpv, no root needed).
DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -d "$DIR/lib" ]; then
    export LD_LIBRARY_PATH="$DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
exec "$DIR/skjalf" "$@"
