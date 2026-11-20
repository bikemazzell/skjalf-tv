"""Fake Xtream Codes server for the README screenshots, on 127.0.0.1 only.

Usage: python3 server.py WORK_DIR PORT
"""
import base64
import json
import re
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse
sys.path.insert(0, str(Path(__file__).parent))
from catalog import CHANNELS, LIVE_GROUPS, MOVIES, SERIES, SERIES_GROUPS, VOD_GROUPS

WWW = Path(sys.argv[1]) / "www"
PORT = int(sys.argv[2])
BASE = f"http://127.0.0.1:{PORT}"
PLOTS = {
    "1001": "A lighthouse keeper on a storm-cut island finds letters that someone keeps writing to a ship lost forty years ago.",
    "1004": "A deep-sea survey crew follows a signal past the edge of the charted ocean and into a current that runs backward.",
}

def b64(text):
    return base64.b64encode(text.encode()).decode()

def api(action, query):
    if action == "":
        return {"user_info": {"username": "demo", "password": "demo", "status": "Active", "exp_date": "1924992000",
                              "active_cons": "0", "max_connections": "2", "auth": 1},
                "server_info": {"url": "127.0.0.1", "port": str(PORT), "timezone": "UTC"}}
    if action == "get_live_categories":
        return [{"category_id": g, "category_name": n, "parent_id": 0} for g, n in LIVE_GROUPS]
    if action == "get_live_streams":
        return [{"num": num, "name": name, "stream_type": "live", "stream_id": int(cid), "category_id": group,
                 "stream_icon": f"{BASE}/images/logo-{cid}.png", "epg_channel_id": "", "tv_archive": 0}
                for cid, num, name, group in CHANNELS]
    if action == "get_vod_categories":
        return [{"category_id": g, "category_name": n, "parent_id": 0} for g, n in VOD_GROUPS]
    if action == "get_vod_streams":
        return [{"num": i + 1, "name": name, "stream_id": int(mid), "category_id": group, "rating": rating,
                 "added": str(added), "container_extension": "mkv", "stream_icon": f"{BASE}/images/movie-{mid}.png"}
                for i, (mid, name, group, rating, added, *_r) in enumerate(MOVIES)]
    if action == "get_vod_info":
        mid = query.get("vod_id", [""])[0]
        movie = next(m for m in MOVIES if m[0] == mid)
        _, name, _, rating, _, year, genre, minutes = movie
        return {"info": {"name": name, "description": PLOTS.get(mid, "A story told in quiet rooms and long journeys."),
                         "genre": genre, "director": "Mara Ellison", "cast": "Jonah Reyes, Ilse Varga, Tom Ashby",
                         "releasedate": f"{year}-03-14", "duration_secs": str(minutes * 60), "rating": rating,
                         "movie_image": f"{BASE}/images/movie-{mid}.png"},
                "movie_data": {"stream_id": int(mid), "container_extension": "mkv"}}
    if action == "get_series_categories":
        return [{"category_id": g, "category_name": n, "parent_id": 0} for g, n in SERIES_GROUPS]
    if action == "get_series":
        return [{"num": i + 1, "name": name, "series_id": int(sid), "category_id": group, "rating": rating,
                 "last_modified": str(modified), "cover": f"{BASE}/images/series-{sid}.png"}
                for i, (sid, name, group, rating, modified) in enumerate(SERIES)]
    if action == "get_series_info":
        sid = query.get("series_id", [""])[0]
        episodes = [{"id": f"{sid}{n:02d}", "episode_num": n, "title": f"Episode {n}", "container_extension": "mkv",
                     "season": 1, "info": {"duration_secs": 2700, "plot": "The tide turns."}} for n in range(1, 7)]
        return {"seasons": [{"season_number": 1, "name": "Season 1", "cover_big": f"{BASE}/images/season-{sid}-1.png"}],
                "info": {"plot": "A cold harbor town keeps its secrets until the winter fleet returns.",
                         "genre": "Drama", "cast": "Ilse Varga, Tom Ashby", "rating": "8.4", "releaseDate": "2025-01-10"},
                "episodes": {"1": episodes}}
    if action == "get_short_epg":
        return {"epg_listings": [
            {"title": b64("Evening Edition"), "description": b64(""), "start": "2026-11-20 09:00:00", "end": "2026-11-20 10:00:00"},
            {"title": b64("The Long Read"), "description": b64(""), "start": "2026-11-20 10:00:00", "end": "2026-11-20 11:00:00"}]}
    return None

class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(WWW), **kwargs)

    def log_message(self, *_):
        pass

    def do_GET(self):
        url = urlparse(self.path)
        if url.path == "/player_api.php":
            query = parse_qs(url.query)
            body = api(query.get("action", [""])[0], query)
            if body is None:
                return self.send_error(404)
            data = json.dumps(body).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        if url.path.startswith("/live/"):
            return self.send_file(WWW / "demo.ts", "video/mp2t")
        if re.match(r"^/(movie|series)/", url.path):
            return self.send_file(WWW / "demo.mkv", "video/x-matroska")
        super().do_GET()

    def send_file(self, path, content_type):
        size = path.stat().st_size
        start, end = 0, size - 1
        match = re.match(r"bytes=(\d+)-(\d*)", self.headers.get("Range", ""))
        if match:
            start = int(match.group(1))
            end = int(match.group(2)) if match.group(2) else size - 1
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        else:
            self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(end - start + 1))
        self.end_headers()
        with open(path, "rb") as f:
            f.seek(start)
            remaining = end - start + 1
            try:
                while remaining > 0:
                    chunk = f.read(min(65536, remaining))
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    remaining -= len(chunk)
            except (BrokenPipeError, ConnectionResetError):
                pass

if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
