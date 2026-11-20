#!/bin/sh
# Take the README screenshots against a fake Xtream server with made-up
# titles and generated artwork, at 1280x800.
#
# Everything runs on an invisible Xvfb display: the app never opens a window
# on the desktop, and the simulated key presses reach only that display.
# Scratch files go to target/screenshots; the images go to docs/screenshots.
#
# Usage: scripts/screenshots/shoot.sh [path-to-release-binary]
#
# Needs: Xvfb, xdotool, ImageMagick (import), pngquant, ffmpeg, python3 with
# Pillow, the DejaVu fonts, and libmpv (MPV_LIBRARY_DIR, default .deps/mpv).
# SKJALF_SHOTS_PORT and SKJALF_SHOTS_DISPLAY change the port (8899) and the
# display number (99).
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
KIT="$ROOT/scripts/screenshots"
BIN="${1:-$ROOT/target/release/skjalf}"
WORK="$ROOT/target/screenshots"
OUT="$ROOT/docs/screenshots"
PORT="${SKJALF_SHOTS_PORT:-8899}"
DISPLAY_NUM="${SKJALF_SHOTS_DISPLAY:-99}"
MPV_DIR="${MPV_LIBRARY_DIR:-$ROOT/.deps/mpv/usr/lib/x86_64-linux-gnu}"

fail() {
    echo "error: $*" >&2
    exit 1
}

for tool in Xvfb xdotool import convert pngquant ffmpeg python3; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done
python3 -c "import PIL" 2>/dev/null || fail "python3 Pillow is not installed"
[ -x "$BIN" ] || fail "no release binary at $BIN (cargo build --release -p skjalf)"
[ -d "$MPV_DIR" ] || fail "no libmpv at $MPV_DIR (set MPV_LIBRARY_DIR)"
[ ! -e "/tmp/.X11-unix/X$DISPLAY_NUM" ] || fail "display :$DISPLAY_NUM is in use (set SKJALF_SHOTS_DISPLAY)"

rm -rf "$WORK"
mkdir -p "$WORK/www" "$WORK/cache" "$WORK/shots" "$WORK/bin"

echo "Drawing artwork and seeding the config ..."
python3 -I "$KIT/prepare.py" "$WORK" "$PORT"

echo "Making the test clips ..."
printf '1\n00:00:00,000 --> 00:10:00,000\nThe signal is clear tonight.\n' > "$WORK/subs.srt"
ffmpeg -y -loglevel error \
    -f lavfi -i "smptehdbars=size=1280x720:rate=25:duration=600" \
    -f lavfi -i "sine=frequency=440:duration=600" \
    -f lavfi -i "sine=frequency=660:duration=600" \
    -i "$WORK/subs.srt" \
    -map 0:v -map 1:a -map 2:a -map 3:s \
    -c:v libx264 -preset ultrafast -tune stillimage -crf 30 -g 25 -pix_fmt yuv420p \
    -c:a aac -ac:a:0 6 -ac:a:1 2 -c:s srt \
    -metadata:s:a:0 language=eng -metadata:s:a:1 language=ger -metadata:s:s:0 language=eng \
    -disposition:a:0 default -disposition:s:0 0 "$WORK/www/demo.mkv"
# A keyframe every second: a decoder that starts mid-stream recovers at once.
# Live URLs end in .ts, so live channels get real MPEG-TS.
ffmpeg -y -loglevel error -i "$WORK/www/demo.mkv" -map 0:v -map 0:a:0 -c copy -f mpegts "$WORK/www/demo.ts"

# The app must never reach a real Steam client.
printf '#!/bin/sh\nexit 0\n' > "$WORK/bin/steam"
chmod +x "$WORK/bin/steam"

PIDS=""
cleanup() {
    for pid in $PIDS; do
        kill "$pid" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

python3 -I "$KIT/server.py" "$WORK" "$PORT" > "$WORK/server.log" 2>&1 &
PIDS="$PIDS $!"
Xvfb ":$DISPLAY_NUM" -screen 0 1280x800x24 -nolisten tcp > "$WORK/xvfb.log" 2>&1 &
PIDS="$PIDS $!"
sleep 2

export DISPLAY=":$DISPLAY_NUM"
env -u WAYLAND_DISPLAY \
    XDG_CONFIG_HOME="$WORK/config" XDG_CACHE_HOME="$WORK/cache" \
    PATH="$WORK/bin:$PATH" LD_LIBRARY_PATH="$MPV_DIR" LIBGL_ALWAYS_SOFTWARE=1 \
    "$BIN" > "$WORK/app.log" 2>&1 &
APP=$!
PIDS="$PIDS $APP"
sleep 6
kill -0 "$APP" 2>/dev/null || fail "the app did not start (see $WORK/app.log)"

# The app acts on a key only when it is held for a moment.
press() {
    for key in "$@"; do
        xdotool keydown "$key"
        sleep 0.15
        xdotool keyup "$key"
        sleep 0.4
    done
}

shoot() {
    xdotool mousemove 1279 799
    sleep 0.5
    import -window root "$WORK/shots/$1.png"
    echo "  $1"
}

# On any failure, keep a picture of the screen for the error message.
trap 'import -window root "$WORK/last-screen.png" 2>/dev/null || true; cleanup' EXIT INT TERM

# Xvfb has no window manager: give the window keyboard focus by hand.
xdotool windowfocus --sync "$(xdotool search --pid "$APP" | head -1)"
xdotool mousemove 1279 799 click 1
sleep 1

echo "Taking screenshots:"
# Opening Movies and Series loads their catalogs for the Main rows.
press Right Return
sleep 4
press Escape Right Return
sleep 4
press Escape Left Left
sleep 4
shoot home

press Return
sleep 3
press Left Down Down Right
sleep 1.5
shoot live-tv

# On software OpenGL a live playback sometimes stays black. Play the
# channel, check the picture for colour, and retry a few times.
colour() {
    convert "$WORK/shots/$1.png" -crop 600x300+300+100 -colorspace HSL \
        -channel G -separate -format "%[fx:mean]" info:
}
attempt=1
while :; do
    press Return
    sleep 8
    press e
    sleep 1.5
    shoot player
    saturation=$(colour player)
    python3 -c "import sys; sys.exit(0 if float('$saturation') > 0.3 else 1)" && break
    [ "$attempt" -lt 4 ] || fail "the live picture shows no colour bars after $attempt tries"
    echo "  live picture black (try $attempt); playing again"
    attempt=$((attempt + 1))
    press Escape
    sleep 1.5
done

# Back to Main, then the second Continue Watching tile.
press Escape Escape
sleep 1
press Down Right Return
sleep 3
shoot details

# Play from Start (the saved position lies past the end of the test clip),
# open the audio and subtitle panel, and turn the subtitles on.
press Right Return
sleep 6
press o Down Right
sleep 3
shoot tracks

for name in home live-tv player details tracks; do
    pngquant --force --quality 70-95 --strip --output "$OUT/$name.png" 256 "$WORK/shots/$name.png"
done
echo "Wrote $OUT/{home,live-tv,player,details,tracks}.png"
