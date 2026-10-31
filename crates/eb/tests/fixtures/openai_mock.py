#!/usr/bin/env python3
"""Tiny OpenAI-compatible mock for the Custom Model tests.

MOCK_MODE=plain  streamed answer with a leading reasoning chunk.
MOCK_MODE=tools  first request answers with a write_file tool_call; once the
                 client posts back a `role: tool` result, answers with text
                 proving the result was seen.
MOCK_MODE=multi  a three-tool script ending in prose: search_content to find
                 a marker in a seeded file, replace_in_file to edit the
                 matched line, todo_write to publish the plan — each round
                 asserts the previous tool result came back, and the last
                 request (no tool messages beyond the plan) answers in text.

Prints its listening address as the first stdout line.
"""
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

mode = os.environ.get("MOCK_MODE", "plain")
out = sys.stdout


def send(message):
    out.write(json.dumps(message, separators=(",", ":")) + "\n")
    out.flush()


def sse(chunk):
    return f"data: {json.dumps(chunk, separators=(',', ':'))}\n\n".encode()


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        try:
            body = json.loads(self.rfile.read(length))
        except json.JSONDecodeError:
            body = {}
        auth = self.headers.get("Authorization", "")

        if "messages" not in body or body.get("model") != "mock-model":
            return self._plain(json.dumps({"error": {"message": "mock: bad request body"}}))
        if auth != "Bearer test-key":
            return self._plain(json.dumps({"error": {"message": "mock: bad auth"}}))

        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()

        if mode == "tools":
            has_tool_result = any(m.get("role") == "tool" for m in body["messages"])
            if not has_tool_result:
                call = {
                    "choices": [{
                        "delta": {"tool_calls": [{
                            "index": 0,
                            "id": "call_1",
                            "type": "function",
                            "function": {
                                "name": "write_file",
                                "arguments": json.dumps({
                                    "path": "tool-test/out.txt",
                                    "content": "hello from the model",
                                }),
                            },
                        }]},
                    }]
                }
                self.wfile.write(sse(call))
                self.wfile.write(b'data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}\n\n')
            else:
                tool = next(m for m in body["messages"] if m.get("role") == "tool")
                assert "tool-test/out.txt" in tool["content"], tool["content"]
                assert "wrote" in tool["content"], tool["content"]
                for piece in ("tool result ", "seen"):
                    self.wfile.write(sse({"choices": [{"delta": {"content": piece}}]}))
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
            return

        if mode == "multi":
            self._multi(body)
            return

        # plain mode: a reasoning chunk, then the answer.
        self.wfile.write(sse({"choices": [{"delta": {"reasoning_content": "thinking hard"}}]}))
        prompt = body["messages"][-1]["content"]
        for piece in ("mock says: ", prompt):
            self.wfile.write(sse({"choices": [{"delta": {"content": piece}}]}))
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def _tool_call(self, tool_id, name, arguments):
        chunk = {"choices": [{
            "delta": {"tool_calls": [{
                "index": 0,
                "id": tool_id,
                "type": "function",
                "function": {"name": name, "arguments": json.dumps(arguments)},
            }]},
        }]}
        self.wfile.write(sse(chunk))
        self.wfile.write(b'data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}\n\n')
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    def _tool_messages(self, body):
        return [m for m in body["messages"] if m.get("role") == "tool"]

    def _multi(self, body):
        """search_content -> replace_in_file -> todo_write -> final prose.

        Every round asserts the previous round's tool result came back with
        the markers the tools are known to emit, so a regression in the
        result plumbing (not just the execution) fails the test.
        """
        results = self._tool_messages(body)
        count = len(results)

        if count == 0:
            # Round 1: find the marker line the test seeded.
            return self._tool_call("call_s", "search_content", {
                "pattern": "REPLACE_MARKER",
                "path": ".",
                "regex": False,
            })

        round1 = results[0]["content"]
        if count == 1:
            assert "todo.txt:1:" in round1, f"round 1 lost the hit line: {round1}"
            return self._tool_call("call_r", "replace_in_file", {
                "path": "todo.txt",
                "old_string": "REPLACE_MARKER original",
                "new_string": "REPLACE_MARKER edited",
            })

        round2 = results[1]["content"]
        if count == 2:
            assert "replaced" in round2, f"round 2 lost the edit result: {round2}"
            return self._tool_call("call_t", "todo_write", {
                "todos": [
                    {"content": "find the marker", "status": "completed"},
                    {"content": "edit the line", "status": "completed"},
                    {"content": "wrap up", "status": "completed"},
                ],
            })

        # Final round: the plan came back; the model must now answer in prose.
        round3 = results[2]["content"]
        assert "plan updated" in round3, f"round 3 lost the plan result: {round3}"
        for piece in ("multi-tool turn ", "complete"):
            self.wfile.write(sse({"choices": [{"delta": {"content": piece}}]}))
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
