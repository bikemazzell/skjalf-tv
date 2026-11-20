"""Draw the made-up artwork and seed the app config for the screenshots.

Usage: python3 prepare.py WORK_DIR PORT
"""
import json
import sys
import time
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, str(Path(__file__).parent))
from catalog import CHANNELS, LIVE_GROUPS, MOVIES, PALETTES, SERIES

FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"


def gradient(size, top, bottom):
    image = Image.new("RGB", size)
    draw = ImageDraw.Draw(image)
    for y in range(size[1]):
        t = y / (size[1] - 1)
        colour = tuple(int(a + (b - a) * t) for a, b in zip(top, bottom))
        draw.line([(0, y), (size[0], y)], fill=colour)
    return image


def poster(title, index, path):
    top, bottom = PALETTES[index % len(PALETTES)]
    image = gradient((300, 450), top, bottom)
    draw = ImageDraw.Draw(image)
    radius = 70 + (index * 13) % 40
    draw.ellipse([150 - radius, 170 - radius, 150 + radius, 170 + radius], outline=(255, 255, 255), width=4)
    font = ImageFont.truetype(FONT, 26)
    lines, line = [], ""
    for word in title.upper().split():
        trial = f"{line} {word}".strip()
        if draw.textlength(trial, font=font) > 260 and line:
            lines.append(line)
            line = word
        else:
            line = trial
    lines.append(line)
    y = 330 - 16 * len(lines)
    for text in lines:
        draw.text(((300 - draw.textlength(text, font=font)) / 2, y), text, font=font, fill=(255, 255, 255))
        y += 32
    image.save(path)


def logo(name, index, path):
    top, bottom = PALETTES[index % len(PALETTES)]
    image = Image.new("RGBA", (256, 256), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.ellipse([8, 8, 248, 248], fill=bottom, outline=top, width=14)
    initials = "".join(word[0] for word in name.split()[:2]).upper()
    font = ImageFont.truetype(FONT, 96)
    draw.text(((256 - draw.textlength(initials, font=font)) / 2, 70), initials, font=font, fill=top)
    image.save(path)


def short(name):
    return name.rsplit(" (", 1)[0]


def draw_art(images):
    images.mkdir(parents=True, exist_ok=True)
    for index, (movie_id, name, *_rest) in enumerate(MOVIES):
        poster(short(name), index, images / f"movie-{movie_id}.png")
    for index, (series_id, name, *_rest) in enumerate(SERIES):
        poster(short(name), index + 3, images / f"series-{series_id}.png")
        poster(f"{short(name)} Season 1", index + 3, images / f"season-{series_id}-1.png")
    for index, (channel_id, _number, name, _group) in enumerate(CHANNELS):
        logo(name, index, images / f"logo-{channel_id}.png")


def private_write(path, text):
    path.write_text(text)
    path.chmod(0o600)


def fnv_hex(text):
    """The store's cache file name: FNV-1a 64-bit of the source id."""
    value = 0xCBF29CE484222325
    for byte in text.encode():
        value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


def seed_config(config, port):
    """A connected source with its playlist cached, so the app opens on Main."""
    host = f"http://127.0.0.1:{port}"
    source = f"xtream:{host}/demo"
    (config / "cache").mkdir(parents=True, exist_ok=True)
    config.chmod(0o700)
    (config / "cache").chmod(0o700)
    private_write(
        config / "settings.toml",
        "volume = 80\nignore_ssl = false\n\n[source]\n"
        f'id = "{source}"\nname = "127.0.0.1:{port}"\nkind = "xtream"\n'
        f'host = "{host}"\nusername = "demo"\npassword = "demo"\nstream_format = "ts"\n',
    )
    playlist = {
        "source": {"id": source, "name": f"127.0.0.1:{port}", "kind": "xtream",
                   "host": host, "username": "", "password": "", "stream_format": "ts"},
        "groups": [{"id": group, "name": name} for group, name in LIVE_GROUPS],
        "channels": [
            {"id": channel_id, "name": name, "num": number,
             "logo": f"{host}/images/logo-{channel_id}.png", "group_id": group,
             "url": f"{host}/live/{{user}}/{{pass}}/{channel_id}.ts"}
            for channel_id, number, name, group in CHANNELS
        ],
    }
    private_write(config / "cache" / f"{fnv_hex(source)}.json", json.dumps(playlist))
    now = int(time.time())

    def entry(kind, item_id, title, position, duration, age, series=None):
        return {
            "source_id": source, "kind": kind, "id": item_id, "title": title,
            "position_secs": position, "duration_secs": duration, "updated_at": now - age,
            "finished": False, "series_id": series and series[0], "series_name": series and series[1],
            "season": series and 1, "episode": series and 3,
        }

    progress = {
        "entries": [
            entry("episode", "200103", "Episode 3", 1050.0, 2700.0, 0, ("2001", "Ravenhold (2025)")),
            entry("movie", "1001", "The Lantern Keeper (2025)", 4200.0, 6720.0, 60),
            entry("movie", "1004", "Beyond the Ninth Wave (2026)", 1920.0, 7680.0, 120),
        ],
        "series_totals": [],
    }
    private_write(config / "progress.json", json.dumps(progress))
    private_write(config / "favorites.json", json.dumps([[source, "101"], [source, "103"]]))


if __name__ == "__main__":
    work, port = Path(sys.argv[1]), int(sys.argv[2])
    draw_art(work / "www" / "images")
    seed_config(work / "config" / "skjalf", port)
