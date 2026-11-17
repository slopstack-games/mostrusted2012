#!/usr/bin/env python3
"""Serve the browser build (target/web) and the player's own install to this machine only (127.0.0.1).

  python3 tools/web/serve.py [--install DIR] [--port 8080] [--out DIR]

then open http://127.0.0.1:8080/ (pick the install folder there) or, without picking,
http://127.0.0.1:8080/?serve=1&args=--camera=-1131,27,1627,-1190,15,1720+--units+4 (reads /game/ below).

- `/`, `/pkg/...`: target/web, with the cross-origin isolation headers threads need (COOP / COEP).
- `/game-listing`: one `<size>\\t<path>` line per install file; `/game/<path>`: the file, with HTTP range requests.
- POST `/__screenshot?name=x.png`: `--screenshot` in a browser saves there, into --out (default scratch/web);
  POST `/__audio?name=x.webm`: the page's `?record_audio=<seconds>` recording of the game's output.
The install is never copied or uploaded anywhere: it is read in place, over loopback.
"""
import argparse, http.server, os, re, shutil, sys, urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STEAM = Path.home() / ".local/share/Steam/steamapps/common/Need for Speed(TM) Most Wanted"

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--install", type=Path, default=Path(os.environ.get("MW2012_DIR", STEAM)))
    ap.add_argument("--port", type=int, default=8080)
    ap.add_argument("--out", type=Path, default=ROOT / "scratch/web")
    a = ap.parse_args()
    web, install = ROOT / "target/web", a.install.resolve()
    a.out.mkdir(parents=True, exist_ok=True)
    listing = "".join(
        f"{p.stat().st_size}\t{p.relative_to(install).as_posix()}\n" for p in sorted(install.rglob("*")) if p.is_file()
    ).encode()

    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def head(self, code, ctype, length, extra=()):
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(length))
            self.send_header("Cross-Origin-Opener-Policy", "same-origin")
            self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
            self.send_header("Cache-Control", "no-store")
            for k, v in extra:
                self.send_header(k, v)
            self.end_headers()

        def send_file(self, path, ctype):
            size = path.stat().st_size
            start, end, code, extra = 0, size - 1, 200, [("Accept-Ranges", "bytes")]
            m = re.fullmatch(r"bytes=(\d+)-(\d*)", self.headers.get("Range", ""))
            if m:
                start, end = int(m[1]), min(int(m[2]) if m[2] else size - 1, size - 1)
                if start > end:
                    return self.head(416, "text/plain", 0, [("Content-Range", f"bytes */{size}")])
                code = 206
                extra.append(("Content-Range", f"bytes {start}-{end}/{size}"))
            self.head(code, ctype, end - start + 1, extra)
            with open(path, "rb") as f:
                f.seek(start)
                left = end - start + 1
                while left > 0:
                    chunk = f.read(min(left, 1 << 20))
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    left -= len(chunk)

        def do_GET(self):
            url = urllib.parse.urlsplit(self.path)
            path = urllib.parse.unquote(url.path)
            if path == "/game-listing":
                self.head(200, "text/plain; charset=utf-8", len(listing))
                return self.wfile.write(listing)
            if path.startswith("/game/"):
                f = (install / path[len("/game/"):]).resolve()
                if install in f.parents and f.is_file():
                    return self.send_file(f, "application/octet-stream")
                return self.head(404, "text/plain", 0)
            f = (web / (path.lstrip("/") or "index.html")).resolve()
            if (web == f.parent or web in f.parents) and f.is_file():
                types = {".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm"}
                return self.send_file(f, types.get(f.suffix, "application/octet-stream"))
            self.head(404, "text/plain", 0)

        def do_POST(self):
            url = urllib.parse.urlsplit(self.path)
            name = urllib.parse.parse_qs(url.query).get("name", ["shot.png"])[0]
            body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
            if url.path in ("/__screenshot", "/__audio"):
                (a.out / Path(name).name).write_bytes(body)
                print(f"{url.path[2:]} {a.out / Path(name).name} ({len(body)} bytes)", flush=True)
            self.head(200, "text/plain", 0)

    print(f"serving http://127.0.0.1:{a.port}/ (install {install}, {listing.count(b'\n')} files)", flush=True)
    http.server.ThreadingHTTPServer(("127.0.0.1", a.port), H).serve_forever()

if __name__ == "__main__":
    main()
