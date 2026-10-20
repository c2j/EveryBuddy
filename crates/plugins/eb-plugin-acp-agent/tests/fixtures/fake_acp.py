#!/usr/bin/env python3
"""Minimal ACP v1 stdio agent used by the plugin integration test.

It is deliberately protocol-only: initialize, session/new, session/prompt,
session/update, session/request_permission and session/cancel.
"""
import json
import os
import sys

out = sys.stdout


def send(message):
    out.write(json.dumps(message, separators=(",", ":")) + "\n")
    out.flush()


def respond(request_id, result):
    send({"jsonrpc": "2.0", "id": request_id, "result": result})


def update(session_id, payload):
    send({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {"sessionId": session_id, "update": payload},
    })


def main():
    session_id = "sess_fake"
    prompt_id = None
    cancel = False
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue

        method = message.get("method")
        request_id = message.get("id")

        if method == "initialize":
            respond(request_id, {
                "protocolVersion": 1,
                "agentInfo": {"name": "fake-acp", "title": "Fake ACP", "version": "1.0"},
                "agentCapabilities": {},
                "authMethods": [],
            })
        elif method == "session/new":
            respond(request_id, {"sessionId": session_id})
        elif method == "session/prompt":
            prompt_id = request_id
            if os.environ.get("FAKE_ACP_CANCEL") == "1":
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "waiting"},
                })
                while True:
                    cancel_line = sys.stdin.readline()
                    if not cancel_line:
                        return
                    try:
                        cancel_message = json.loads(cancel_line)
                    except json.JSONDecodeError:
                        continue
                    if cancel_message.get("method") == "session/cancel":
                        respond(prompt_id, {"stopReason": "cancelled"})
                        prompt_id = None
                        break
                continue
            if os.environ.get("FAKE_ACP_CODE") == "1":
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": (
                        "已创建 `Demo.java`。\n\n"
                        "```java\n"
                        "public class Demo {\n"
                        "    public static void main(String[] args) {\n"
                        "        System.out.println(\"hello\");\n"
                        "    }\n"
                        "}\n"
                        "```\n\n"
                        "**说明**\n"
                        "- 第一项使用 `Markdown`\n"
                        "- 第二项\n"
                    )},
                })
            else:
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "Hello "},
                })
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "from ACP."},
                })
            if os.environ.get("FAKE_ACP_LONG") == "1":
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "A" * 4000},
                })
            update(session_id, {
                "sessionUpdate": "usage_update",
                "used": 42,
                "size": 100,
            })
            update(session_id, {
                "sessionUpdate": "plan",
                "entries": [
                    {"content": "Inspect the input", "status": "completed", "priority": "high"},
                    {"content": "Render the result", "status": "in_progress", "priority": "medium"},
                ],
            })
            update(session_id, {
                "sessionUpdate": "tool_call",
                "toolCallId": "call_exec",
                "title": "Ran a command",
                "kind": "execute",
                "status": "pending",
                "rawInput": {"command": "echo hi"},
            })
            # A permission request in the middle of the turn; the client must
            # answer it without blocking the prompt.
            send({
                "jsonrpc": "2.0",
                "id": 900,
                "method": "session/request_permission",
                "params": {
                    "sessionId": session_id,
                    "toolCall": {"toolCallId": "call_exec"},
                    "options": [
                        {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                        {"optionId": "reject", "name": "Reject", "kind": "reject_once"},
                    ],
                },
            })
            # Wait for the client's permission response; a client that ignores
            # requests would make this test hang instead of silently passing.
            while True:
                response_line = sys.stdin.readline()
                if not response_line:
                    return
                try:
                    response = json.loads(response_line)
                except json.JSONDecodeError:
                    continue
                if response.get("id") == 900:
                    break
                if response.get("method") == "session/cancel":
                    respond(prompt_id, {"stopReason": "cancelled"})
                    prompt_id = None
                    break
            update(session_id, {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call_exec",
                "status": "completed",
                "content": [{"type": "content", "content": {
                    "type": "text",
                    "text": ("hi\n" if os.environ.get("FAKE_ACP_LONG") != "1" else ("L" * 4000 + "\n")),
                }}],
            })
            update(session_id, {
                "sessionUpdate": "tool_call",
                "toolCallId": "call_diff",
                "title": "Edited a file",
                "kind": "edit",
                "status": "pending",
            })
            update(session_id, {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call_diff",
                "status": "completed",
                "content": [{
                    "type": "diff",
                    "path": "demo.txt",
                    "oldText": "a\nb\n",
                    "newText": "a\nB\n",
                }],
            })
            respond(prompt_id, {"stopReason": "end_turn"})
            prompt_id = None
        elif method == "session/cancel":
            cancel = True
            if prompt_id is not None:
                respond(prompt_id, {"stopReason": "cancelled"})
                prompt_id = None
        elif request_id == 900 and "result" in message:
            # Permission response: don't block; the scenario ignores the choice.
            continue

    _ = os.environ.get("FAKE_ACP_SCENARIO", "basic")
    _ = cancel


if __name__ == "__main__":
    main()
