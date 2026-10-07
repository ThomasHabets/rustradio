#!/usr/bin/env python3
"""Serve the built example with the headers required for shared WASM memory."""
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import argparse
import functools

class Handler(SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        super().end_headers()

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8080)
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent / "web-dist"
    if not (directory / "index.html").exists():
        parser.error("Run ./build-local.sh first")
    server = ThreadingHTTPServer(("127.0.0.1", args.port), functools.partial(Handler, directory=str(directory)))
    print(f"Open http://127.0.0.1:{args.port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
