"""Serves the demo page and proxies /v1/audio/* to a kokoro-server, adding its API key, so the key never
reaches the browser and the page needs no CORS.

    python3 serve.py --listen 0.0.0.0:PORT --upstream http://127.0.0.1:PORT --key-file KEY
"""

import argparse
import http.client
import http.server
import os
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
DIST = os.path.join(HERE, "dist")
PROXIED = {("GET", "/v1/audio/voices"), ("POST", "/v1/audio/speech")}
MAX_BODY = 64 * 1024


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    # Applies to every read and write on the client socket.
    timeout = 30

    def do_GET(self):
        static = self.static_file()
        if static:
            path, content_type = static
            with open(path, "rb") as f:
                data = f.read()
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(data)
        else:
            self.proxy("GET")

    def static_file(self):
        if self.path == "/":
            return os.path.join(HERE, "index.html"), "text/html; charset=utf-8"
        name = self.path.removeprefix("/dist/")
        # esbuild's output names only: no directories, so nothing outside dist/ can be named.
        if name != self.path and name.endswith(".js") and os.path.isdir(DIST) and name in os.listdir(DIST):
            return os.path.join(DIST, name), "text/javascript"
        return None

    def do_POST(self):
        self.proxy("POST")

    def proxy(self, method):
        if (method, self.path) not in PROXIED:
            self.send_error(404)
            return
        length = self.headers.get("Content-Length", "0")
        if not length.isascii() or not length.isdigit() or int(length) > MAX_BODY:
            self.send_error(413 if length.isdigit() else 400)
            return
        if method == "POST" and self.headers.get("Content-Type", "").split(";")[0].strip() != "application/json":
            # A cross-site form or no-cors fetch can't send this without a CORS preflight, which we never answer.
            self.send_error(415)
            return
        body = self.rfile.read(int(length)) if method == "POST" else None
        upstream = http.client.HTTPConnection(self.server.upstream.hostname, self.server.upstream.port, timeout=60)
        headers = {"Authorization": f"Bearer {self.server.key}"}
        if body is not None:
            headers["Content-Type"] = "application/json"
        try:
            upstream.request(method, self.path, body=body, headers=headers)
            response = upstream.getresponse()
        except OSError:
            upstream.close()
            self.send_error(502)
            return
        self.send_response(response.status)
        self.send_header("Content-Type", response.getheader("Content-Type", "application/octet-stream"))
        self.send_header("Transfer-Encoding", "chunked")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        try:
            while chunk := response.read1(64 * 1024):
                self.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
                self.wfile.flush()
            self.wfile.write(b"0\r\n\r\n")
        except OSError:
            # Client gone, or the upstream failed mid-stream: drop the connection without the final chunk,
            # so the browser sees a truncated body rather than a complete one.
            self.close_connection = True
        finally:
            upstream.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", required=True)
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--key-file", required=True)
    args = parser.parse_args()
    host, port = args.listen.rsplit(":", 1)
    server = http.server.ThreadingHTTPServer((host, int(port)), Handler)
    server.upstream = urllib.parse.urlsplit(args.upstream)
    with open(args.key_file) as f:
        server.key = f.read().strip()
    if not server.key or "," in server.key or any(c.isspace() for c in server.key):
        parser.error("the key file must hold exactly one key")
    server.serve_forever()


if __name__ == "__main__":
    main()
