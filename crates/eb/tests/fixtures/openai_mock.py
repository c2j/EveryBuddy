#!/usr/bin/env python3
"""Tiny OpenAI-compatible mock: asserts the request body is real JSON with
messages, and answers with a chat completion referencing the prompt.

Prints its listening address as the first stdout line.
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

seen = {}


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        try:
            seen.update(json.loads(self.rfile.read(length)))
        except json.JSONDecodeError:
            seen.clear()
        auth = self.headers.get("Authorization", "")

        if "messages" not in seen or seen.get("model") != "mock-model":
            body = json.dumps({"error": {"message": "mock: bad request body"}})
            self._plain(body)
            return
        if auth != "Bearer test-key":
            # Plain JSON error on a streamed request: exercises the client's
            # non-SSE fallback path.
            self._plain(json.dumps({"error": {"message": "mock: bad auth"}}))
            return

        prompt = seen["messages"][-1]["content"]
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for piece in ("mock says: ", prompt):
            chunk = json.dumps({
                "choices": [{"delta": {"content": piece}}]
            }, separators=(",", ":"))
            self.wfile.write(f"data: {chunk}\n\n".encode())
            self.wfile.flush()
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def _plain(self, body):
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body.encode())

    def log_message(self, *args):
        pass


server = HTTPServer(("127.0.0.1", 0), Handler)
print(f"{server.server_address[0]}:{server.server_address[1]}", flush=True)
server.serve_forever()
