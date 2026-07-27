#!/usr/bin/env python3
"""Loopback origin for the modern Debian proxy boot test."""

import http.server
import pathlib
import socketserver
import sys


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"DEBIAN_PROXY_OK\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        request_body = self.rfile.read(length)
        if request_body == b"chunked guest body":
            body = b"DEBIAN_CHUNKED_OK\n"
            status = 200
        else:
            body = b"BAD_REQUEST_BODY\n"
            status = 400
        self.send_response(status)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format, *_args):
        pass


with socketserver.TCPServer(("127.0.0.1", 0), Handler) as server:
    pathlib.Path(sys.argv[1]).write_text(str(server.server_address[1]))
    server.serve_forever()
