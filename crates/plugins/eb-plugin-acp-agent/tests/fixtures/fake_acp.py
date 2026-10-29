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


def private_update(session_id, payload):
    """The envelope Grok really uses: same params, private method name."""
    send({
        "jsonrpc": "2.0",
        "method": "_x.ai/session_notification",
        "params": {"sessionId": session_id, "update": payload},
    })


def wait_response(want_id):
    """Read stdin until the client answers the given request id."""
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("id") == want_id and ("result" in message or "error" in message):
            if "error" in message:
                return {"error": message["error"].get("message", "")}
            return message["result"]
    return {"error": "client closed stdout"}


def run_client_tools(session_id):
    """Exercise the fs/terminal client methods end to end on a real turn.

    Every step's observed fact lands in the conversation text, so the Rust
    suite can assert on what the *agent* saw the client do.
    """
    notes = []

    # fs/write_text_file: the client creates parents for a relative path.
    send({"jsonrpc": "2.0", "id": 910, "method": "fs/write_text_file", "params": {
        "sessionId": session_id, "path": "client_tools/a.txt",
        "content": "one\ntwo\nthree\n",
    }})
    result = wait_response(910)
    notes.append("write done" if not isinstance(result, dict) or "error" not in result
                 else f"write FAILED {result}")

    # fs/read_text_file with a 1-based line window.
    send({"jsonrpc": "2.0", "id": 911, "method": "fs/read_text_file", "params": {
        "sessionId": session_id, "path": "client_tools/a.txt", "line": 2, "limit": 1,
    }})
    result = wait_response(911)
    notes.append("read window=" + result.get("content", "<none>"))

    # terminal: create, drain output, wait for exit, release.
    send({"jsonrpc": "2.0", "id": 912, "method": "terminal/create", "params": {
        "sessionId": session_id, "command": "sh", "args": ["-c", "echo from-terminal"],
    }})
    result = wait_response(912)
    terminal_id = result.get("terminalId")
    notes.append("terminal=" + str(terminal_id))

    send({"jsonrpc": "2.0", "id": 913, "method": "terminal/output", "params": {
        "sessionId": session_id, "terminalId": terminal_id,
    }})
    # The output thread may lag behind the process; poll with a small pause
    # until the line shows up.
    import time
    output = None
    for _ in range(60):
        result = wait_response(913)
        output = result.get("output") if isinstance(result, dict) else None
        if output and "from-terminal" in output:
            break
        time.sleep(0.05)
        send({"jsonrpc": "2.0", "id": 913, "method": "terminal/output", "params": {
            "sessionId": session_id, "terminalId": terminal_id,
        }})
    notes.append("termout=" + (output or "<none>").strip())

    send({"jsonrpc": "2.0", "id": 914, "method": "terminal/wait_for_exit", "params": {
        "sessionId": session_id, "terminalId": terminal_id,
    }})
    result = wait_response(914)
    notes.append("termexit=" + str(result.get("exitCode")))

    send({"jsonrpc": "2.0", "id": 915, "method": "terminal/release", "params": {
        "sessionId": session_id, "terminalId": terminal_id,
    }})
    wait_response(915)

    # A released terminal id must be rejected, not silently served.
    send({"jsonrpc": "2.0", "id": 916, "method": "terminal/output", "params": {
        "sessionId": session_id, "terminalId": terminal_id,
    }})
    result = wait_response(916)
    notes.append("released-rejected=" + str("error" in result))

    update(session_id, {
        "sessionUpdate": "agent_message_chunk",
        "content": {"type": "text", "text": "CLIENTTOOLS " + " | ".join(notes)},
    })


def activity(session_id, state):
    """Grok's `_x.ai/sessions/changed`: the session row, with `activity` set."""
    send({
        "jsonrpc": "2.0",
        "method": "_x.ai/sessions/changed",
        "params": {
            "upserted": [{"sessionId": session_id, "activity": state}],
            "removed": [],
        },
    })


def main():
    session_id = "sess_fake"
    prompt_id = None
    prompt_count = 0
    cancel = False
    # Scenarios come from FAKE_ACP_SCENARIO (what the Rust suite sets); the
    # older FAKE_ACP_* variables still work so manual harness runs keep working.
    scenario = os.environ.get("FAKE_ACP_SCENARIO", "")

    def mode(name):
        return scenario == name or os.environ.get("FAKE_ACP_" + name.upper()) == "1"

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
            prompt_count += 1
            if (
                mode("tail_busy")
                or mode("backlog")
                or (mode("idle_between") and prompt_count == 1)
                or (mode("resume") and prompt_count == 1)
            ):
                # Grok announces the running prompt before it says anything else.
                activity(session_id, "working")
            if mode("cancel"):
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
            if mode("client_tools"):
                # Exercise the fs/terminal client methods the client declared
                # in `initialize`; the observations land in the reply text.
                run_client_tools(session_id)
                respond(prompt_id, {"stopReason": "end_turn"})
                prompt_id = None
                continue
            if mode("code"):
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
                    "sessionUpdate": "agent_thought_chunk",
                    "content": {"type": "text", "text": "Thinking: "},
                })
                update(session_id, {
                    "sessionUpdate": "agent_thought_chunk",
                    "content": {"type": "text", "text": "greet the user."},
                })
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "Hello "},
                })
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "from ACP."},
                })
            if mode("long"):
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
                    "text": ("hi\n" if not mode("long") else ("L" * 4000 + "\n")),
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
            if mode("retry"):
                # A provider-side retry, in the two shapes seen in the wild:
                # Grok wraps it in a private notification, other agents use the
                # spec method. The client must show either or the UI looks hung.
                private_update(session_id, {
                    "sessionUpdate": "retry_state",
                    "type": "retrying",
                    "attempt": 1,
                    "max_retries": 15,
                    "reason": "request error: Connection reset by peer",
                })
                update(session_id, {
                    "sessionUpdate": "retry_state",
                    "type": "retrying",
                    "attempt": 2,
                    "max_retries": 15,
                    "reason": "request error: Connection reset by peer",
                })
            respond(prompt_id, {"stopReason": "end_turn"})
            prompt_id = None
            if mode("tail"):
                # Background work: the agent answers its own turn first, then
                # keeps streaming. A client that stops reading at the response
                # loses all of this.
                import time
                time.sleep(0.2)
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "\n\nresearch: part one."},
                })
                send({
                    "jsonrpc": "2.0",
                    "id": 901,
                    "method": "session/request_permission",
                    "params": {
                        "sessionId": session_id,
                        "toolCall": {"toolCallId": "call_tail"},
                        "options": [
                            {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                            {"optionId": "reject", "name": "Reject", "kind": "reject_once"},
                        ],
                    },
                })
                while True:
                    tail_line = sys.stdin.readline()
                    if not tail_line:
                        return
                    try:
                        tail_response = json.loads(tail_line)
                    except json.JSONDecodeError:
                        continue
                    if tail_response.get("id") == 901:
                        break
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": " research: part two."},
                })
                # Grok emits this alongside the (already-consumed) response. A
                # client that treats it as the end of the turn would cut the
                # continuation off after the first chunk.
                private_update(session_id, {
                    "sessionUpdate": "turn_completed",
                    "prompt_id": "p_tail",
                    "stop_reason": "end_turn",
                    "elapsed_ms": 400,
                })
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": " research: part three."},
                })
            if mode("tail_error"):
                # The background work died after the planning turn answered, so
                # the only signal left is the private failure notice.
                import time
                time.sleep(0.2)
                private_update(session_id, {
                    "sessionUpdate": "turn_completed",
                    "prompt_id": "p_tail_error",
                    "stop_reason": "error",
                    "agent_result": "reqwest error stream: Connection reset by peer",
                    "elapsed_ms": 1402086,
                })
            if mode("tail_busy"):
                # Grok's deep-research shape: the answer arrives in bursts that
                # are longer apart than any silence probe can wait, while the
                # session keeps saying it is working. Only `idle` ends it.
                import time
                time.sleep(0.6)
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "\n\nresearch: after the first silence."},
                })
                time.sleep(0.6)
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": " after the second."},
                })
                activity(session_id, "idle")
            if mode("past_idle"):
                # The other ordering, which a client must also survive: `idle`
                # first, then the answer. The first chunk revives the
                # continuation window, so the second one is not cut off at
                # half a probe.
                import time
                activity(session_id, "idle")
                time.sleep(0.2)
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "\n\nresearch: after idle,"},
                })
                time.sleep(0.6)
                update(session_id, {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": " still streaming."},
                })
            if mode("backlog"):
                # Work the client has already given up on: it answers, says idle,
                # and only then keeps talking. Whatever lands in the pipe after
                # the turn closed belongs to nobody, and must not be read out as
                # the next turn's answer.
                import time
                activity(session_id, "idle")
                time.sleep(1.5)
                for index in range(3):
                    update(session_id, {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": " LEFTOVER%d" % index},
                    })
            if mode("resume") and prompt_count > 1:
                # Turn two: nothing announces it. The session is *already*
                # `working` from the turn the client gave up on, and the row only
                # changes when the state does — a client that waits for a fresh
                # flip ends this turn at the probe and loses the late chunk.
                # Turn three: `idle` landed in between, so this one must close at
                # the probe instead of riding the busy window.
                import time
                if prompt_count == 2:
                    time.sleep(0.7)
                    update(session_id, {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "\n\nresearch: still running."},
                    })
                    activity(session_id, "idle")
            if mode("idle_between") and prompt_count == 1:
                # The client gave up while the session still said `working`, and
                # the work really did finish a moment later. That `idle` lands
                # with nobody reading, and it is still the agent's last word.
                import time
                time.sleep(4.0)
                activity(session_id, "idle")
        elif method == "session/cancel":
            cancel = True
            if prompt_id is not None:
                respond(prompt_id, {"stopReason": "cancelled"})
                prompt_id = None
        elif request_id == 900 and "result" in message:
            # Permission response: don't block; the scenario ignores the choice.
            continue
    _ = cancel


if __name__ == "__main__":
    main()
