#!/usr/bin/env python3
"""Serve the browser build (target/web) and the player's own install to this machine only (127.0.0.1).

  python3 tools/web/serve.py [--install DIR] [--port 8080] [--out DIR] [--web DIR]

then open http://127.0.0.1:8080/ (pick the install folder there) or, without picking,
http://127.0.0.1:8080/?serve=1&args=--camera=-1131,27,1627,-1190,15,1720+--units+4 (reads /game/ below).

- `/`, `/pkg/...`: target/web (--web: another directory), with the cross-origin isolation headers threads need
  (COOP / COEP, on every response). GET and HEAD.
- `/game-listing`: one `<size>\\t<path>` line per install file; `/game/<path>`: the file, with HTTP range requests.
- POST `/__screenshot?name=x.png`: `--screenshot` in a browser saves there, into --out (default scratch/web);
  POST `/__audio?name=x.webm`: the page's `?record_audio=<seconds>` recording of the game's output.
The install is never copied or uploaded anywhere: it is read in place, over loopback. Requests must name this
server (`Host: 127.0.0.1:<port>` or `localhost:<port>`) and come from its own pages (no foreign `Origin`), so no
other site open in the browser can read the install or post files through it.
"""
import argparse, http.server, os, re, shutil, sys, urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STEAM = Path.home() / ".local/share/Steam/steamapps/common/Need for Speed(TM) Most Wanted"

def parse_range(header, size):
    """A `Range` header against a file of `size` bytes: `(first, last)` to send (206), `"unsatisfiable"` (416: the
    range starts at or past the end), or None for the whole file (no header, or one we do not serve: several
    ranges, other units, last < first; RFC 9110 lets a server ignore those)."""
    m = re.fullmatch(r"\s*bytes\s*=\s*(\d*)\s*-\s*(\d*)\s*", header)
    if not m or not (m[1] or m[2]):
        return None
    if not m[1]:  # bytes=-N: the last N bytes
        n = int(m[2])
        return (max(size - n, 0), size - 1) if n > 0 and size > 0 else "unsatisfiable"
    start = int(m[1])
    if m[2] and int(m[2]) < start:
        return None
    if start >= size:
        return "unsatisfiable"
    return start, min(int(m[2]), size - 1) if m[2] else size - 1


def inside(root, relative):
    """The file `relative` names below `root`, or None: not for `..`, an absolute path or a link that lead out of
    it, a directory, a missing file or a name the system refuses (a NUL byte)."""
    try:
        f = (root / relative).resolve()
        return f if root in f.parents and f.is_file() else None
    except (OSError, ValueError):
        return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--install", type=Path, default=Path(os.environ.get("MW2012_DIR", STEAM)))
    ap.add_argument("--port", type=int, default=8080)
    ap.add_argument("--out", type=Path, default=ROOT / "scratch/web")
    ap.add_argument("--web", type=Path, default=ROOT / "target/web")
    a = ap.parse_args()
    web, install = a.web.resolve(), a.install.resolve()
    a.out.mkdir(parents=True, exist_ok=True)
    listing = "".join(
        f"{p.stat().st_size}\t{p.relative_to(install).as_posix()}\n" for p in sorted(install.rglob("*")) if p.is_file()
    ).encode()

    hosts = {f"{h}:{a.port}" for h in ("127.0.0.1", "localhost")}

    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def end_headers(self):
            # On every response, the errors http.server writes itself included (a page that is not cross-origin
            # isolated cannot start the game's threads, and an isolated page may not load a response without them).
            self.send_header("Cross-Origin-Opener-Policy", "same-origin")
            self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
            self.send_header("Cache-Control", "no-store")
            super().end_headers()

        def handle(self):
            try:
                super().handle()
            except (BrokenPipeError, ConnectionResetError):
                pass  # the browser dropped the request (a reload, a closed tab)

        def head(self, code, ctype, length, extra=()):
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(length))
            for k, v in extra:
                self.send_header(k, v)
            self.end_headers()

        def local(self):
            """Only this machine's own pages: the name asked for must be ours (a rebinding DNS name is not), and a
            request sent by another site's page (its Origin) is refused."""
            origin = self.headers.get("Origin")
            if self.headers.get("Host") in hosts and (origin is None or origin in {f"http://{h}" for h in hosts}):
                return True
            self.head(403, "text/plain", 0)
            return False

        def send_file(self, path, ctype):
            size = path.stat().st_size
            start, end, code, extra = 0, size - 1, 200, [("Accept-Ranges", "bytes")]
            rng = parse_range(self.headers.get("Range", ""), size)
            if rng == "unsatisfiable":
                return self.head(416, "text/plain", 0, [("Content-Range", f"bytes */{size}")])
            if rng:
                start, end = rng
                code = 206
                extra.append(("Content-Range", f"bytes {start}-{end}/{size}"))
            self.head(code, ctype, end - start + 1, extra)
            if self.command == "HEAD":
                return
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
            if not self.local():
                return
            url = urllib.parse.urlsplit(self.path)
            path = urllib.parse.unquote(url.path)
            if path == "/game-listing":
                self.head(200, "text/plain; charset=utf-8", len(listing))
                return None if self.command == "HEAD" else self.wfile.write(listing)
            if path.startswith("/game/"):
                f = inside(install, path[len("/game/"):])
                if f:
                    return self.send_file(f, "application/octet-stream")
                return self.head(404, "text/plain", 0)
            f = inside(web, path.lstrip("/") or "index.html")
            if f:
                types = {".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm"}
                return self.send_file(f, types.get(f.suffix, "application/octet-stream"))
            self.head(404, "text/plain", 0)

        do_HEAD = do_GET

        def do_POST(self):
            if not self.local():
                return
            url = urllib.parse.urlsplit(self.path)
            if url.path not in ("/__screenshot", "/__audio"):
                return self.head(404, "text/plain", 0)
            name = Path(urllib.parse.parse_qs(url.query).get("name", ["shot.png"])[0]).name
            try:
                length = int(self.headers.get("Content-Length", ""))
            except ValueError:
                return self.head(411, "text/plain", 0)
            if name in ("", ".", "..") or "\0" in name or length < 0:
                return self.head(400, "text/plain", 0)
            body = self.rfile.read(length)
            (a.out / name).write_bytes(body)
            print(f"{url.path[2:]} {a.out / name} ({len(body)} bytes)", flush=True)
            self.head(200, "text/plain", 0)

    print(f"serving http://127.0.0.1:{a.port}/ (install {install}, {listing.count(b'\n')} files)", flush=True)
    http.server.ThreadingHTTPServer(("127.0.0.1", a.port), H).serve_forever()

if __name__ == "__main__":
    main()
