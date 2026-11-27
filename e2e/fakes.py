"""Fake external services for the browser tests. Nothing leaves the machine.

    python3 fakes.py <download-dir>

Ports: 7101 TMDB (API under /3, images under /img), 7102 Torznab (/api),
7103 Synology DSM with Download Station. GET /health on each answers ok.

Download Station tasks finish after two list calls; their files appear in
<download-dir>/<release name>/.
"""

import json
import os
import struct
import sys
import threading
import urllib.parse
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DOWNLOADS = sys.argv[1]
TMDB, TORZNAB, SYNOLOGY = 7101, 7102, 7103

MOVIES = {
    603: ("The Matrix", "The Matrix", 1999, "A hacker learns the truth about his reality.", (40, 10, 80)),
    438631: ("Dune", "Dune", 2021, "Paul Atreides arrives on Arrakis.", (150, 90, 30)),
    872585: ("Oppenheimer", "Oppenheimer", 2023, "The story of the atomic bomb.", (90, 20, 20)),
}

RELEASES = [
    ("2160p.UHD.BluRay.x265-BIG", 60e9, 80),
    ("1080p.BluRay.x264.CASTELLANO-GOOD", 9e9, 50),
    ("1080p.WEB-DL.DDP5.1-NTb", 6e9, 120),
    ("720p.WEBRip.x264-OK", 2e9, 10),
]

tasks = []
lock = threading.Lock()


def png(rgb):
    w, h = 60, 90
    raw = b"".join(b"\x00" + bytes([min(255, rgb[0] + y), rgb[1], min(255, rgb[2] + y * 2)]) * w for y in range(h))

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def details(i):
    title, original, year, overview, _ = MOVIES[i]
    return {
        "id": i,
        "imdb_id": "tt%07d" % i,
        "title": title,
        "original_title": original,
        "release_date": "%d-01-01" % year,
        "overview": overview,
        "poster_path": "/p%d.png" % i,
    }


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def send(self, body, ct="application/json", code=200):
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
        if isinstance(body, str):
            body = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", ct)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def params(self):
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        if self.command == "POST":
            n = int(self.headers.get("Content-Length", 0))
            q.update(dict(urllib.parse.parse_qsl(self.rfile.read(n).decode())))
        return u.path, q

    def do_POST(self):
        self.do_GET()

    def do_GET(self):
        path, q = self.params()
        port = self.server.server_address[1]
        if path == "/health":
            return self.send("ok", "text/plain")
        if port == TMDB:
            return self.tmdb(path, q)
        if port == TORZNAB:
            return self.torznab(q)
        if port == SYNOLOGY:
            return self.synology(path, q)
        self.send({"error": "not found"}, code=404)

    def tmdb(self, path, q):
        if path.startswith("/img/"):
            i = int(path.rsplit("/p", 1)[1].split(".")[0])
            return self.send(png(MOVIES[i][4]), "image/png")
        if q.get("api_key") != "tmdb-test-key":
            return self.send({"status_message": "Invalid API key"}, code=401)
        if path == "/3/search/movie":
            s = q.get("query", "").lower()
            hits = [details(i) for i, m in MOVIES.items() if s in m[0].lower()]
            return self.send({"results": hits})
        if path.startswith("/3/movie/"):
            return self.send(details(int(path.split("/")[-1])))
        if path == "/3/configuration":
            return self.send({})
        self.send({}, code=404)

    def torznab(self, q):
        if q.get("apikey") != "torznab-test-key":
            return self.send('<error code="100" description="Invalid API Key"/>', "application/xml")
        if q.get("t") == "caps":
            return self.send("<caps><searching/></caps>", "application/xml")
        items = ""
        for title, _, year, _, _ in MOVIES.values():
            name = title.replace(" ", ".")
            for suffix, size, seeders in RELEASES:
                rel = "%s.%d.%s" % (name, year, suffix)
                items += (
                    "<item><title>%s</title><link>http://fake/%s.torrent</link><size>%d</size>"
                    '<torznab:attr name="seeders" value="%d"/></item>' % (rel, rel, size, seeders)
                )
        feed = '<rss xmlns:torznab="http://torznab.com/schemas/2015/feed"><channel>%s</channel></rss>' % items
        self.send(feed, "application/xml")

    def synology(self, path, q):
        api = q.get("api")
        if path == "/webapi/query.cgi":
            return self.send({"success": True, "data": {
                "SYNO.API.Auth": {"path": "entry.cgi", "minVersion": 1, "maxVersion": 7},
                "SYNO.DownloadStation.Task": {"path": "DownloadStation/task.cgi", "minVersion": 1, "maxVersion": 3},
                "SYNO.DownloadStation2.Task": {"path": "entry.cgi", "minVersion": 1, "maxVersion": 2},
            }})
        if api == "SYNO.API.Auth":
            if q.get("passwd") != "ds-test-password":
                return self.send({"success": False, "error": {"code": 400}})
            return self.send({"success": True, "data": {"sid": "sid"}})
        if q.get("_sid") != "sid":
            return self.send({"success": False, "error": {"code": 119}})
        if api == "SYNO.DownloadStation2.Task":
            url = json.loads(q["url"])[0]
            title = url.rsplit("/", 1)[1].replace(".torrent", "")
            with lock:
                task = {"id": "dbid_%d" % (len(tasks) + 1), "title": title,
                        "dest": json.loads(q["destination"]), "uri": url, "done": 0}
                tasks.append(task)
            folder = os.path.join(DOWNLOADS, title)
            os.makedirs(folder, exist_ok=True)
            with open(os.path.join(folder, title + ".mkv"), "wb") as f:
                f.write(b"x" * 1000)
            return self.send({"success": True, "data": {"task_id": [task["id"]]}})
        if api == "SYNO.DownloadStation.Task":
            if q.get("method") == "list":
                out = []
                with lock:
                    for t in tasks:
                        t["done"] = min(100, t["done"] + 50)
                        out.append({
                            "id": t["id"], "title": t["title"], "size": 100,
                            "status": "finished" if t["done"] >= 100 else "downloading",
                            "additional": {"detail": {"destination": t["dest"], "uri": t["uri"]},
                                           "transfer": {"size_downloaded": t["done"]}},
                        })
                return self.send({"success": True, "data": {"total": len(out), "tasks": out}})
            if q.get("method") == "delete":
                with lock:
                    tasks[:] = [t for t in tasks if t["id"] != q.get("id")]
            return self.send({"success": True, "data": {}})
        self.send({"success": False, "error": {"code": 102}})


if __name__ == "__main__":
    os.makedirs(DOWNLOADS, exist_ok=True)
    for port in (TMDB, TORZNAB, SYNOLOGY):
        s = ThreadingHTTPServer(("127.0.0.1", port), Handler)
        threading.Thread(target=s.serve_forever, daemon=True).start()
    print("fakes listening on 7101-7103", flush=True)
    threading.Event().wait()
