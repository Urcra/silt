"""A stand-in for Hugging Face in the local models fixture: one GGUF
repository, `owner/tiny-GGUF`, whose one file is `Tiny-Q4_K_M.gguf` at the
path given. Run as `python3 fake-hub.py <port> <gguf>`."""

import hashlib
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse

PORT = int(sys.argv[1])
WEIGHTS = open(sys.argv[2], "rb").read()
FILE = "Tiny-Q4_K_M.gguf"
REPO = "owner/tiny-GGUF"


class Hub(BaseHTTPRequestHandler):
    def send(self, status, body, kind="application/json"):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = urlparse(self.path).path
        if path == "/api/models":
            found = [{"id": REPO, "downloads": 4321, "lastModified": "2026-11-01T00:00:00.000Z"}]
            self.send(200, json.dumps(found).encode())
        elif path == f"/api/models/{REPO}":
            lfs = {"sha256": hashlib.sha256(WEIGHTS).hexdigest(), "size": len(WEIGHTS)}
            model = {
                "id": REPO,
                "gguf": {"architecture": "llama", "context_length": 8192},
                "siblings": [{"rfilename": FILE, "size": len(WEIGHTS), "lfs": lfs}],
            }
            self.send(200, json.dumps(model).encode())
        elif path == f"/{REPO}/resolve/main/{FILE}":
            self.send(200, WEIGHTS, "application/octet-stream")
        else:
            self.send(404, b"{}")

    def log_message(self, *args):
        pass


ThreadingHTTPServer(("127.0.0.1", PORT), Hub).serve_forever()
