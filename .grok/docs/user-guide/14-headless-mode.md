# Headless Mode and Scripting

Headless mode runs Grok non-interactively from the command line. It accepts a single prompt, executes it with full tool access, and returns the result. Use it to automate tasks, script workflows, build integrations, and parse output programmatically.

---

## Basic Usage

Passing a prompt non-interactively triggers headless mode. The most common way is the `-p` flag (short for `--single`); `--prompt-json` and `--prompt-file` also trigger it:

```bash
grok -p "Your prompt here"
```

Grok processes the prompt, runs any necessary tools, and prints the result to stdout. The process exits when the response is complete.

---

## Command-Line Options

| Flag                    | Description                                           |
| ----------------------- | ----------------------------------------------------- |
| `-p, --single <PROMPT>` | The prompt to send (or use `--prompt-json` / `--prompt-file`) |
| `-m, --model <MODEL>`   | Model to use (e.g., `grok-4.6`)              |
| `-s, --session-id <ID>` | Create a **new** session with this **UUID** (errors if invalid UUID or already in use under the target session directory; does not resume, use `-r`/`-c`) |
| `--fork-session`        | With `-r`/`-c`, fork into a new session ID instead of appending to the original |
| `-r, --resume <ID_OR_TITLE>` | Resume an existing session by ID, or by title for the current directory, ignoring letter case (without an exact match, the same words joined by any separators also match, so `fix-login-bug` finds "Fix login bug"; a sole manually renamed match wins among duplicates; remaining duplicates error with up to ten of them, newest first, with IDs and last update times; a miss suggests up to five similar titles with their IDs; UUID-shaped values always take the ID path; scripts should prefer IDs) |
| `-c, --continue`        | Continue the most recent session in current directory  |
| `--cwd <PATH>`          | Set working directory                                 |
| `--add-dir <DIR>`       | Also work in this folder (repeatable). A relative path starts from the working directory. Refused if the folder isn't trusted or the agent works only in its working directory. Can't be combined with `-r`/`-c`: a resumed session keeps the folders it was created with. |
| `--output-format <FMT>` | Output format: `plain`, `json`, `streaming-json`, `streaming-messages-json` |
| `--include-partial-messages` | Emit raw `stream_event` deltas. Only affects `--output-format streaming-messages-json`; ignored (with a warning) otherwise. |
| `--yolo`                | Auto-approve all tool executions                      |
| `--rules <TEXT>`        | Custom rules for the system prompt                    |
| `--tools <TOOLS>`       | Allowlist of built-in tools (comma-separated). MCP meta-tools remain available unless denied. |
| `--disallowed-tools <TOOLS>` | Denylist of built-in tools to remove (comma-separated). Supports `Agent` entries. |
| `--max-turns <N>`       | Maximum number of agentic turns before stopping. Headless only. |
| `--reasoning-effort` / `--effort <LEVEL>` | Reasoning effort for reasoning models. Canonical levels: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` (each a distinct tier; a model only accepts the levels its menu advertises). Also accepts per-model menu option ids (e.g. `deep` → mapped wire value), same as `/effort`. Works in TUI and headless. |
| `--permission-mode <MODE>` | Permission mode. `bypassPermissions` enables always-approve (see [Permissions and safety](22-permissions-and-safety.md#permission-modes)). `plan` starts the session in [plan mode](19-plan-mode.md). For deny-by-default use `defaultMode` in `.claude/settings.json`. |
| `--allow <RULE>`        | Permission allow rule with glob patterns (repeatable). Works in TUI and headless. |
| `--deny <RULE>`         | Permission deny rule with glob patterns (repeatable). Works in TUI and headless. |
| `--prompt-json <JSON>`  | Prompt as JSON content blocks                         |
| `--prompt-file <PATH>`  | Prompt from a file                                    |
| `--verbatim`            | Send prompt exactly as given                          |
| `--no-auto-update`      | Disable update checks for this session                |
| `--sandbox <PROFILE>`   | Sandbox profile for filesystem/network access         |

> **Note:** `--max-turns` and `--agents` are headless-only flags. If used in the interactive TUI, a warning is printed and the flag is ignored. `--tools`, `--disallowed-tools`, `--reasoning-effort`/`--effort`, `--permission-mode`, `--allow`, and `--deny` work in both modes. For more flags (agents and worktrees), see [Additional Headless Flags](#additional-headless-flags).

### Tool Filtering

Use `--tools` to restrict the agent to an explicit set of tools (allowlist), or `--disallowed-tools` to remove specific tools from the default set (denylist). Both accept comma-separated tool names.

Tool names are internal tool IDs (e.g. the shell tool is `run_terminal_cmd`, not `bash`). On the daemon route, the names that route shows work too: `Shell`, `Await`, `StrReplace`, `EditNotebook`, `Delete`, `ReadLints`, `SwitchMode`, `AskQuestion`, `CreatePlan`, `CallMcpTool`, `GetMcpTools`, `FetchMcpResource`, `ListMcpResources`, `GenerateImage`, `GenerateVideo`, `CreateGoal`, `UpdateGoal`, and `XSearch`.
`StrReplace` and `EditNotebook` both name the one edit tool, so `--tools EditNotebook` allows edits
to any file.

```bash
# Only allow read-only tools
grok -p "Explain this codebase" --tools "read_file,grep,list_dir"

# Remove web access and file editing
grok -p "Review this code" --disallowed-tools "web_search,web_fetch,write,search_replace"

# Remove shell access
grok -p "Review this code" --disallowed-tools "run_terminal_cmd"
```

A `--disallowed-tools` entry may also be the name the model is shown, which is what a transcript
displays. Five tools are offered under a name that differs
from their ID: `run_terminal_cmd` as `run_terminal_command`, `task` as `spawn_subagent`,
`get_task_output` as `get_command_or_subagent_output`, `wait_tasks` as `wait_commands_or_subagents`,
and `kill_task` as `kill_command_or_subagent`. Either spelling removes the tool on the shell route.
On the daemon route, an entry removes nothing, with a warning, when the tools it names have no
daemon equivalent, such as `get_task_output`, `wait_tasks`, `kill_task`, `lsp`,
`enter_plan_mode`, `monitor`, and the `scheduler_*` tools. `workflow` also removes nothing there,
with the same warning: the model has no `workflow` tool on that route, and it starts workflows with
`execute`. Turning workflows off (`[workflows] enabled = false` or `GROK_WORKFLOWS=0`) removes
`execute`. A `--disallowed-tools`
entry naming a subagent type such as `Agent(explore)` removes nothing and warns too, unless
the list also has a bare `Agent` or `Task`, which already removes the task tool.
`run_terminal_cmd` and `task` remove the daemon's shell and task tools.
`StrReplace`, `EditNotebook` or `search_replace` removes the edit tool, and `Delete` goes
with it. `write` removes Write alone. Where the bundle on the daemon route can't name Write
alone, `write` removes the whole edit tool and `Delete` with it, as `grok -p` warns.
`read_file` and `hashline_read`, `grep` and `hashline_grep`, and `search_replace` and
`hashline_edit` name the same file tool, whichever file toolset is on.
On the daemon route `image_edit` names the same image tool as `image_gen`.
`CreateGoal` leaves when `UpdateGoal` does, and `/goal` goes with it.
A kept `UpdateGoal` stays, so `--tools CreateGoal` needs `UpdateGoal` as well.
That route also shows `Read`, `Grep`, `Glob`, `TodoWrite`,
`WebSearch`, `WebFetch`, and the edit tool as `Write`. An entry of one of
those names removes nothing and warns; use `StrReplace` for the edit tool.
`Await` is removed once the shell tool and the task tool are both removed.
An entry that names no tool removes nothing, and `grok -p` prints a warning for it on stderr.
A `--tools` entry that names no tool leaves the other tools available. That entry warns too.
A `--tools` entry that names a tool outside the built-in toolsets grants nothing unless the agent
adds it, and on the shell route it warns.
The TUI prints these warnings on stderr before it opens, and its welcome banner also shows them
above any other startup warning. In the TUI a set list keeps the session off the
shared leader. On the daemon route no session can open on another computer while either list is set,
and no workflow can start or resume then. When this computer is a Remote Control target (`/remote`),
a turn a controlling device starts here follows the `--tools` and `--disallowed-tools` of the grok
window whose worker holds the connection.
On the daemon route a `--tools` entry naming a subagent type, such as `Agent(explore)`,
stops `grok -p` and the TUI with an error.

`--disallowed-tools` also supports special `Agent` entries to control subagent spawning:

| Entry                  | Effect                                  |
| ---------------------- | --------------------------------------- |
| `Agent`                | Block all subagent spawning             |
| `Agent(explore)`       | Block the `explore` subagent type only  |
| `Agent(explore, plan)` | Block multiple specific types           |

```bash
# Prevent the agent from spawning any subagents
grok -p "Fix this bug" --disallowed-tools "Agent"

# Block only the explore subagent
grok -p "Refactor this module" --disallowed-tools "Agent(explore)"
```

`--tools` preserves the selected agent profile's injection policy: stock profiles inject enabled optional tools before applying the allowlist, while curated profiles remain strict. The final toolset retains requested tools plus always-on MCP meta-tools. When both flags are present, `--disallowed-tools` wins.

### Permission Rules (`--allow` / `--deny`)

Permission rules control whether specific tool invocations are auto-approved, denied, or require user confirmation. Unlike `--disallowed-tools` (which removes tools entirely), permission rules leave tools available but gate their execution.

Rules use `ToolPrefix(glob_pattern)` syntax:

| Prefix        | What it controls                   |
| ------------- | ---------------------------------- |
| `Bash(...)`   | Shell command execution            |
| `Edit(...)`   | File editing (path glob)           |
| `Write(...)`  | File writing (path glob)           |
| `Read(...)`   | File reading (path glob)           |
| `Grep(...)`   | Search operations (path glob)      |
| `WebFetch(...)` | URL fetching (glob or `domain:host`) |
| `MCPTool(...)` | MCP tool invocations              |

For path rules (`Read`, `Edit`, `Write`, `Grep`), `*` is a single-level wildcard and `**` is recursive. For `Bash` rules, `*` matches any characters including spaces. A bare prefix without parentheses matches all invocations of that type, and `Bash(cmd:*)` is equivalent to prefix matching on `cmd`. See [22-permissions-and-safety.md](22-permissions-and-safety.md#rule-matching-reference) for the full matching semantics.

```bash
# Deny shell commands matching "rm*"
grok -p "Clean up this project" --deny "Bash(rm*)"

# Allow npm commands, deny sudo
grok -p "Set up the project" --allow "Bash(npm*)" --deny "Bash(sudo*)"

# Allow all bash commands (auto-approve without prompting)
grok -p "Build the project" --allow "Bash"
```

`--allow` and `--deny` can be repeated. Deny rules take precedence over allow rules.
One value can also hold several rules, separated by commas or by a space after a closing parenthesis: `--deny "Bash(rm *) Edit(.env)"`.
Write a `)` inside a pattern as `\)` so it is not read as the end of the rule.

`--allowedTools` and `--allowed-tools` are aliases of `--allow`.
`--disallowedTools` is an alias of `--deny`.
`--disallowed-tools` removes tools from the offered list (see [Tool Filtering](#tool-filtering)).

---

## Output Formats

Headless mode supports four output formats, selected with `--output-format`.

### plain (default)

Human-readable text, suitable for direct display or piping:

```
Here's a summary of the codebase...
```

### json

A single JSON object emitted after the response completes: response text,
stop reason, session ID, request ID, and the folder the session ran in (plus
`thought` when reasoning is present).
When the prompt reached the model, the same object also carries spend fields
(`usage`, `num_turns`, `modelUsage`, cost). `stopReason` is the snake_case
ACP/Messages token (`end_turn`, `max_tokens`, …).

```json
{
  "text": "Here's a summary of the codebase...",
  "stopReason": "end_turn",
  "sessionId": "abc123",
  "requestId": "xyz789",
  "cwd": "/repo",
  "num_turns": 7,
  "usage": {
    "input_tokens": 7210,
    "cache_read_input_tokens": 41000,
    "cache_creation_input_tokens": 0,
    "output_tokens": 1893,
    "reasoning_tokens": 412,
    "total_tokens": 50103
  },
  "modelUsage": {
    "grok-4.6": {
      "inputTokens": 7210,
      "outputTokens": 1893,
      "cacheReadInputTokens": 41000,
      "modelCalls": 7,
      "costUSD": 0.01268905
    }
  },
  "total_cost_usd": 0.01268905,
  "total_cost_usd_ticks": 126890500
}
```

Usage notes:

- `usage` sums tokens for the prompt, including subagents that finished
  before turn end (also under their own `modelUsage` keys). Compaction and
  other side-model calls are excluded.
- **Token field policy (headless result / `end` / error spend):**
  - `usage.input_tokens` and `modelUsage.*.inputTokens` are **uncached only**.
  - `cache_read_input_tokens` / `cacheReadInputTokens` are cache hits.
  - `total_tokens` is full input + output (includes both cache buckets):
    `total_tokens = input_tokens + cache_read_input_tokens + cache_creation_input_tokens + output_tokens`.
  - ACP `_meta.usage.inputTokens` (PromptUsage) is still the **full** prompt
    sum; only the headless projector subtracts cache. Prefer headless fields
    for spend automation.
- `num_turns` counts main-agent model rounds recorded on the prompt ledger
  (tool-loop rounds that reported usage). Subagent sampler calls do not
  increase it. Per-model call counts (including subagents) stay on
  `modelUsage.*.modelCalls`. This is the same counter family as `--max-turns`,
  not a guarantee of exact equality when rounds lack usage or hit gates.
- `total_cost_usd` appears only when the server reported a **complete** cost.
  Absence means unreported or incomplete, never free. Cost is stamped for
  API-key traffic today; pool/OAuth paths often omit it until the server
  stamps cost. When some calls lacked cost, `cost_is_partial` is true and
  **all** cost floats are omitted (`total_cost_usd` and every
  `modelUsage.*.costUSD`) so consumers cannot sum model rows into a fake
  complete bill.
- `total_cost_usd_ticks` is the same value in exact integer ticks
  (1 USD = 10^10 ticks) and appears under the same conditions. Use it for
  billing reconciliation: summing per-invocation ticks matches the server's
  usage export exactly, which float dollars cannot guarantee.
- When subagent usage could not be applied, nested subagent usage was incomplete,
  or the success-path drain timed out (up to 120s on the turn task),
  `usage_is_incomplete` is true and cost floats are omitted the same way
  (token totals may under-count subagents). Cancel snapshots without that long
  drain and marks incomplete while subagents are still live. Incomplete with
  no recorded tokens emits only `usage_is_incomplete` (no zero `usage` object).
- A prompt that never reached the model omits the spend fields.

The `sessionId` field is useful for resuming the conversation later.

`cwd` is the folder the session ran in.
With `--worktree NAME`, it is inside the new worktree.
When a worktree named `NAME` exists, Grok exits with an error that names its folder and creates nothing.
A `NAME` whose folder you deleted by hand is used again, unless git still holds its registration; the error then says how to free it.
In a jj repository, a taken `NAME` still becomes `NAME-2`, `NAME-3`, and so on.

On failure, Grok emits an error object (process exit non-zero). Prompt-level
failures may also include frozen spend fields when usage was recorded:

```json
{"type":"error","message":"Couldn't start session: ..."}
```

Once the session has started, the error object also carries `cwd`.

### streaming-json

Newline-delimited JSON, one `type`-tagged object per line, derived from the agent's ACP session updates. Leaf field names (`toolCallId`, `kind`, `rawInput`, `rawOutput`) follow ACP; `toolName` and the `usage` line are xAI additions. Consume it by switching on `type`.

```json
{"type":"thought","data":"Analyzing the directory structure..."}
{"type":"tool_call","toolCallId":"call_1","title":"Read","kind":"read","status":"in_progress","toolName":"read_file","rawInput":{"path":"src/main.rs"},"content":[],"locations":[]}
{"type":"tool_call_update","toolCallId":"call_1","status":"completed","content":[],"rawOutput":{"lines":42},"locations":[]}
{"type":"text","data":"Here's a summary"}
{"type":"usage","messageId":"resp_1","stopReason":"end_turn","usage":{"input_tokens":812,"output_tokens":45,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"reasoning_tokens":0},"signature":"..."}
{"type":"end","stopReason":"end_turn","sessionId":"abc123","requestId":"xyz789","cwd":"/repo","usage":{...},"num_turns":7,"modelUsage":{...}}
```

Event types:

| Type               | Description                                                                                  |
| ------------------ | ------------------------------------------------------------------------------------------- |
| `text`             | A chunk of the agent's response text                                                          |
| `thought`          | Internal reasoning (thinking tokens)                                                          |
| `tool_call`        | A tool call the agent started (`toolCallId`, `toolName`, `kind`, `status`, `rawInput`, `content`, `locations`) |
| `tool_call_update` | Progress or result for a tool call (`status`, `rawOutput`, `content`, `locations`)            |
| `usage`            | Per-response boundary (`messageId`, `stopReason`, `usage`, `signature`), one per model response |
| `plan`             | The agent's current plan (`entries`)                                                          |
| `available_commands` | Tool and slash command lists (`tools`, `commands`)                                          |
| `end`              | Final event with the session folder, metadata, and spend fields when available               |
| `error`            | An error occurred (carries `message`, the session folder once the session has started, and spend fields if any) |

`end` is always the last event. Spend fields on `end` match the json object
shape (snake_case uncached `input_tokens`, safe cost floats). `end.stopReason`
is the turn stop reason in snake_case (`end_turn`, `max_tokens`,
`max_turn_requests`, `refusal`, `cancelled`); the verbatim per-response provider
reason (e.g. `tool_use`, `pause_turn`) is on the `usage` line's `stopReason`.
Per-response `message_id`/`stopReason`/`signature` are populated on the Messages
API backend; other backends report what they carry.

Grok may also emit `max_turns_reached` and `auto_compact_*` events; treat the list as non-exhaustive and switch on `type`.

### streaming-messages-json

Newline-delimited JSON in the Messages API `stream-json` wire format. The data-bearing surface matches the Messages shape exactly. This includes the `assistant`/`user` message bodies, `usage`, `tool_use`/`tool_result`, inline web search, `stop_reason`, and the `--include-partial-messages` event framing. A consumer that reconstructs messages, reads spend, or detects errors works without changes.

The `system`/`init` and terminal `result` lines carry metadata. Grok emits the fields it has real data for and omits pure-placeholder fields it cannot fill, rather than zero-filling them. As a result, those two lines may not pass strict `init`/`result` schema validation. The individual fields are listed below. Read the fidelity notes before treating any one field as authoritative. For a clean xAI-native stream with no placeholder shape, use `streaming-json`.

The stream opens with a `system`/`init` line, then `assistant` messages whose `message.content[]` holds `text`, `thinking`, and `tool_use` blocks, `user` messages carrying `tool_result` blocks, and a terminal `result`:

```json
{"type":"system","subtype":"init","session_id":"abc123","apiKeySource":"user","model":"grok-4.6","cwd":"/repo","permissionMode":"default","tools":["read_file","bash"],"slash_commands":["review"],"mcp_servers":[{"name":"linear","status":"connected"}],"skills":[],"uuid":"..."}
{"type":"assistant","message":{"id":"msg_0","type":"message","role":"assistant","model":"grok-4.6","content":[{"type":"text","text":"Let me read the file."},{"type":"tool_use","id":"call_1","name":"read_file","input":{"path":"src/main.rs"}}],"stop_reason":"tool_use","stop_sequence":null,"usage":{...}},"parent_tool_use_id":null,"session_id":"abc123","uuid":"..."}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"fn main() {}","is_error":false}]},"parent_tool_use_id":null,"session_id":"abc123","uuid":"..."}
{"type":"result","subtype":"success","is_error":false,"duration_ms":0,"duration_api_ms":0,"num_turns":7,"result":"Here's a summary...","stop_reason":"end_turn","total_cost_usd":0.0127,"usage":{"input_tokens":812,"output_tokens":210,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"server_tool_use":{"web_search_requests":0}},"modelUsage":{},"session_id":"abc123","uuid":"..."}
```

Message types:

| Type        | Description                                                              |
| ----------- | ---------------------------------------------------------------------- |
| `system`    | Session preamble (`subtype: "init"`) with model, cwd, permission mode, tools, slash commands, and MCP servers. `subtype: "compact_boundary"` marks an auto compaction |
| `assistant` | A model message; `message.content[]` holds `text`/`thinking`/`tool_use`, plus `server_tool_use`/`web_search_tool_result` for inline backend web search |
| `user`      | Tool results, as `tool_result` blocks inside `message.content[]`         |
| `result`    | Terminal message with final text, stop reason, and spend fields         |

The `assistant` and `user` messages carry `session_id`, `uuid`, and `parent_tool_use_id` (`null` for the main conversation). The `system`/`init` and terminal `result` lines carry `session_id` and `uuid` but no `parent_tool_use_id`.

The `uuid` on each line is freshly generated per emitted line. It is not a provider, message, or event id, and not a correlation key. It does not match the provider `message.id` (that value rides `assistant.message.id`). It is unique per line, even for lines that describe the same message, and it carries no cross-line or cross-run identity. Do not use it to correlate or deduplicate.

Text and reasoning chunks are grouped into one assistant message per model response. A response's parallel `tool_result` blocks are grouped into a single `user` message. `result.result` is the final assistant message text. A model response that produces no content blocks emits no `assistant` line in the default mode. Only `--include-partial-messages` surfaces such a response, as its empty `message_start` … `message_stop` envelope.

On `init`, `skills` is live. It lists the session's user-invocable skill names, a subset of `slash_commands` sourced from the session's advertised commands, or `[]` when the session surfaces no skills. The `init` line is emitted once, deferred to the first output line so it captures the session's advertised `tools`, `slash_commands`, and `skills`. The Messages schema defines no second `init`, so a command list that changes after streaming begins is not re-advertised.

The other `init` fields carry real data:

- `apiKeySource` is `user` for API-key auth and `oauth` otherwise. Grok does not distinguish the schema's `project`, `org`, and `temporary` sources.
- `permissionMode` is the effective headless mode mapped to the Messages enum: the `--permission-mode` value, or `bypassPermissions` under `--yolo`, else `default`. Grok-only modes such as `auto` collapse to `default`.
- `mcp_servers[].status` is one `x.ai/mcp/list` snapshot, emitted only for `streaming-messages-json`: `connected`, `failed`, `needs-auth`, `pending`, or `disabled`. Servers still handshaking are `pending`. `disabled` is only stamped after the session reports `sessionMcpResolved`; an unresolved list row is `pending` even when `enabled` is still false. The snapshot does not wait for the Blocking startup grace; that grace still applies to the prompt's toolset. Other output formats omit the array and do not call `x.ai/mcp/list`.

Grok omits the schema's pure-placeholder `init` fields it has no data for, rather than emitting dummy values: `claude_code_version`, `output_style`, and `plugins`.

`result` includes `duration_ms`, `duration_api_ms`, `num_turns`, `stop_reason`, `total_cost_usd`, `usage` (Messages API `message.usage` shape), and `modelUsage`. It also includes `errors[]` on the error subtypes. Grok omits the schema's always-empty `permission_denials`, because it does not collect permission denials. `structured_output` (with `--json-schema`) is snake_case, matching the schema.

`model` appears on `init` and every `assistant` frame. It is the real model id when known, and the literal `"unknown"` only when no model is known at emit time.

The assistant frame's `stop_sequence` is wired end-to-end. It carries the provider's matched stop sequence when the model stopped on a configured one (`stop_reason: "stop_sequence"`), and is `null` on every other stop reason and backend. In `--include-partial-messages` framing, the matched sequence rides both the flushed `assistant` frame and the partial `message_delta.stop_sequence`, so a partial rebuild matches the frame. Only the partial `message_start.stop_sequence` stays `null`, because the matched sequence is not known at message open.

The emitted error subtypes are `error_max_turns`, `error_during_execution`, and `error_max_structured_output_retries`. The schema's `error_max_budget_usd` subtype is never emitted, because grok has no budget feature.

`result.usage` reports the Messages `message.usage` shape with the three token buckets disjoint: `input_tokens` (uncached), `cache_read_input_tokens`, and `cache_creation_input_tokens`. Grok derives these from the turn's aggregate ledger, reshaped into those buckets. Subagent cache creation is included in `cache_creation_input_tokens`. The aggregate ledger tracks it as its own bucket, so it is no longer folded into `input_tokens`.

`result.usage` always emits numeric buckets, even when data is missing. This happens when the turn's usage ledger is incomplete (the same condition that surfaces `usage_is_incomplete` in the `json` format), or when no aggregate ledger reached the reducer at all. Any bucket grok cannot account for falls back to `0`, because the Messages API schema has no marker for incomplete or absent usage. The reducer logs a warning to stderr in both cases. Read an all-zero `usage` here as "unknown", not "free".

The nested `server_tool_use` counter is populated. `web_search_requests` is the number of *successful* backend web searches emitted this run. Failed searches and non-search `WebSearch` actions such as open_page are excluded, matching the Messages API, which does not bill errored searches. A failed backend search still emits a `web_search_tool_result` in the error shape (`content.type: "web_search_tool_result_error"`), but is not counted. Its `error_code` is a fixed `"unavailable"` placeholder, not a code forwarded from the backend. There is no `web_fetch_requests` key, because grok has no server-side `web_fetch`, so the placeholder is omitted.

Backend web search is inline. It folds into the same `assistant` frame as the surrounding text. The frame carries a `server_tool_use` block (`name: "web_search"`, `input.query`) immediately followed by a `web_search_tool_result` block. That result block's `tool_use_id` matches the `server_tool_use.id`, and its `content` is a `web_search_result` hit array of `{type, url, title}`. This matches the Messages API's inline server-tool shape rather than splitting the response across frames.

X search and code interpreter are a documented divergence. They stay generic, surfaced as a client `tool_use` block plus a `user` `tool_result`, because the Messages API defines no inline block type for them. Every other client tool likewise keeps the `tool_use`/`tool_result` split.

`--include-partial-messages` emits the raw event framing so a consumer can rebuild each message with the Messages streaming accumulator. The framing is `message_start`, `content_block_start`/`content_block_delta`/`content_block_stop`, `message_delta`, and `message_stop`. It carries the structural events an accumulator needs. The deltas are coarser than the Messages API's token-level streaming: tool input arrives as a single `input_json_delta`, and `citations_delta` is never produced (see below). The result is a faithful reconstruction of each message rather than a token-by-token replay.

On the Messages API backend, the framing is faithful. `message_start` carries the real provider `message.id` and the input-side `usage`. A thinking block emits its `signature_delta` in order, before the block's `content_block_stop`. The `message_start.usage` input side reports all three prompt-side buckets known at message open: `input_tokens` (the uncached portion), `cache_read_input_tokens`, and `cache_creation_input_tokens`. A cache hit is therefore visible on `message_start`, rather than only appearing later on `message_delta`/`result`. `output_tokens` seeds `0` there and is finalized on `message_delta`. A response that starts but produces no content still emits the `message_start` … `message_stop` envelope with no content blocks.

Some backends surface per-response metadata only at end of turn. Those backends fall back to a synthesized `message_start.id` and zero-seeded input `usage`. They defer the reasoning `signature` to the final `assistant` line, which is authoritative in that case.

Tool-call input is emitted as a single `input_json_delta` carrying the complete arguments JSON, followed by `content_block_stop`. It is not a sequence of token-level fragments. This is a deliberate divergence from the Messages API's incremental `partial_json` streaming. Grok's ACP tool-call path delivers each tool call as one validated JSON object once the arguments are fully parsed, so a single delta is the accurate representation. A consumer that concatenates `partial_json` reassembles the identical object either way. The backend web-search `server_tool_use` block's `input.query` is emitted the same way, as one `input_json_delta`.

The Messages API `citations_delta` carries inline citations for cited text spans, such as those from web search. This stream does not produce it. Grok's Messages content deltas are limited to text, thinking, signature, and tool-input JSON, so there is no citation data to surface as a `citations_delta`. Backend web-search source URLs are reported inline on the completed `web_search_tool_result` block instead (see above), not as per-span text citations.

Fidelity caveats apply to a few fields.

`duration_ms` is the prompt-execution wall clock. `duration_api_ms` is the summed *reported* per-call model time. A model call that does not report its own duration contributes `0`, so `duration_api_ms` can under-count the true API time.

`num_turns` and `total_cost_usd` are authoritative when known. When they are not, `num_turns` falls back to the count of completed model responses this turn, and `total_cost_usd` falls back to `0`. A completed but contentless response emits no `assistant` line, yet still counts as a turn. Spend is never overreported.

`modelUsage` carries the per-model token and cost fields grok tracks, plus `webSearchRequests` attributed to the active model. The reducer tracks a single global web-search count rather than per-model, so the whole count lands on the current or last model and other rows stay `0`. A per-model `modelUsage.*.costUSD` is `0` when that model's cost is unknown or withheld. This is the same fail-closed-to-zero behavior as the top-level `total_cost_usd`. The `json` format omits cost floats entirely when partial, but this stream keeps the field present and `0`. `contextWindow` is the current model's real total context window (the same value grok uses for auto-compaction), and it appears only on the current model's row. Other rows omit it, and so does the current row when the window is unknown. `maxOutputTokens` has no grok catalog, so that key is omitted entirely. `modelUsage` is `{}` when no per-model breakdown is available.

Like `streaming-json`, this stream is read only. Tool approvals and other bidirectional flows use the ACP interface (`grok agent`).

---

## Session Management in Headless Mode

By default, each `grok -p` invocation creates a fresh session. To maintain context across calls, use session flags.

### Named Sessions (`-s`)

To carry context across headless calls, use `-r/--resume` or `-c/--continue`. Use `-s/--session-id` only for a **new** session with a **UUID** (errors if not a UUID or already in use under the target directory). Older hidden `-s` upsert/resume behavior is gone. Use `-r`/`-c` to continue. With `-r`/`-c`, `-s` requires `--fork-session`:

```bash
# Start a headless session and capture its ID
grok -p "Review the changes in this PR" --output-format json | jq -r '.sessionId'

# Continue in the same session
grok -p "Now check for security issues" --resume "<id>"

# Optional: create with a client-chosen UUID (must not already exist)
grok -p "hello" --session-id "$(uuidgen | tr '[:upper:]' '[:lower:]')" --output-format json
```

> **Note:** `-s/--session-id` creates a new session only (valid UUID; errors if already in use). Use `-r` to resume.

### Resume (`-r`)

The `-r/--resume` flag resumes a specific session by ID, or by title for the current directory when the value is not an ID, ignoring letter case (without an exact match, the same words joined by any separators also match, so `fix-login-bug` finds "Fix login bug"; a sole manually renamed match wins among duplicates; remaining duplicates error with up to ten of them, newest first, with IDs and last update times; a miss suggests up to five similar titles with their IDs; UUID-shaped values always take the ID path, so scripts should prefer IDs). It errors if the session does not exist:

```bash
# Get the session ID from a previous JSON response
grok -p "Remember: the secret number is 42" --output-format json
# Output includes "sessionId": "abc123"

# Resume that exact session
grok -p "What's the secret number?" --resume abc123
```

### Continue (`-c`)

The `-c/--continue` flag continues the most recent session in the current working directory:

```bash
grok -p "Continue where we left off" -c
```

### Extracting Session IDs

Use `--output-format json` and parse the `sessionId` field:

```bash
grok -p "Hello" --output-format json | jq -r '.sessionId'
```

---

## Piping Input and Output

Headless mode works naturally with Unix pipes and redirection.

### Standard Output

```bash
# Pipe output to a file
grok -p "Generate a README" > README.md

# Parse JSON output with jq
grok -p "List files" --output-format json | jq -r '.text'
```

### Standard Input

Headless mode does not read piped stdin into the prompt. Pass external content through command substitution or `--prompt-file`:

```bash
# Include git diff as context via command substitution
grok -p "Write a concise commit message for these changes:

$(git diff --staged)"

# Or read the prompt from a file
grok --prompt-file ./prompt.txt
```

---

## CI/CD Integration Examples

### Automated Code Review

```bash
grok -p "Review changes for bugs and security issues." \
  --output-format json --yolo | jq -r '.text' > review.md
```

### Pre-Commit Hook

```bash
grok -p "Review staged changes for obvious bugs. Reply OK if fine, or list issues." \
  --yolo --output-format json | jq -r '.text' | grep -q "^OK" || exit 1
```

### Batch Processing

```bash
for file in src/*.js; do
  grok -p "Migrate $file from CommonJS to ES modules." --yolo
done
```

---

## Scripting Patterns

### Python Wrapper

Grok's headless mode can be wrapped as an OpenAI-compatible chat completion API:

```python
import asyncio
import json
import os

class GrokChat:
    """Simple OpenAI-compatible wrapper using headless mode."""

    def __init__(self, cwd="."):
        self.cwd = cwd
        self.env = {**os.environ}

    def _build_cmd(self, prompt, model, stream):
        return ["grok", "-p", prompt, "-m", model, "--cwd", self.cwd,
                "--output-format", "streaming-json" if stream else "json",
                "--yolo"]

    async def create(self, messages, model="grok-4.6", stream=False):
        prompt = messages[-1]["content"] if len(messages) == 1 else "\n".join(
            f"{m['role']}: {m['content']}" for m in messages
        )
        cmd = self._build_cmd(prompt, model, stream)

        if stream:
            return self._stream(cmd)

        proc = await asyncio.create_subprocess_exec(
            *cmd, env=self.env, stdout=asyncio.subprocess.PIPE
        )
        stdout, _ = await proc.communicate()
        data = json.loads(stdout.decode()) if stdout else {"text": ""}
        return {
            "choices": [{
                "message": {"role": "assistant", "content": data.get("text", "")},
                "finish_reason": "stop"
            }]
        }

    async def _stream(self, cmd):
        proc = await asyncio.create_subprocess_exec(
            *cmd, env=self.env, stdout=asyncio.subprocess.PIPE
        )
        async for line in proc.stdout:
            if not line.strip():
                continue
            event = json.loads(line)
            if event.get("type") == "text":
                yield {"choices": [{"delta": {"content": event["data"]}}]}
            elif event.get("type") == "end":
                yield {"choices": [{"delta": {}, "finish_reason": "stop"}]}


async def main():
    client = GrokChat(cwd=".")
    response = await client.create(
        [{"role": "user", "content": "What files are here?"}]
    )
    print(response["choices"][0]["message"]["content"])

asyncio.run(main())
```

### Shell Script

```bash
#!/bin/bash
# Run a code review and exit with failure if issues are found

RESULT=$(grok -p "Review this PR for bugs. Output JSON with 'issues' array." \
  --output-format json --yolo | jq -r '.text')

ISSUE_COUNT=$(echo "$RESULT" | jq '.issues | length' 2>/dev/null || echo "0")

if [ "$ISSUE_COUNT" -gt 0 ]; then
  echo "Found $ISSUE_COUNT issues"
  echo "$RESULT" | jq '.issues[]'
  exit 1
fi

echo "No issues found"
```

---

## Always-approve for automation

`--always-approve` (alias `--yolo`, same as `--permission-mode bypassPermissions`) runs tool calls without interactive permission prompts. Deny rules, hooks, and admin locks still apply (see [Permissions and safety](22-permissions-and-safety.md#permission-modes)).

Without always-approve, including `--permission-mode plan`, a tool call that needs approval is refused, since a headless run cannot ask, and stderr names the call once: `permission required: Edit calc.py was not run, as a headless run cannot ask for approval`. The note prints only when the run gets an approval request, and plan-mode refusals happen before that. The exit code stays `0`.

On the daemon route, an approval that a background workflow run's own `host` call needs is decided the same way. While the headless run waits for its background work after its turn, stderr names the workflow: `permission required: Edit REVIEW.md (workflow "review") was not run, as a headless run cannot ask for approval`. While the turn runs, the run's approval is asked on that turn like the turn's own tool calls, and the line names only the call. The run gets the refusal, and a run that then fails sets the exit code (see [Workflows and Background Work](#workflows-and-background-work)).

```bash
grok -p "Format all files" --always-approve
grok -p "Run the tests and fix any failures" --cwd ~/projects/my-app --always-approve
```

For agent servers and SDKs, see [Agent mode](15-agent-mode.md#automation-and-sdks).

---

## Kept plans on the daemon route

On the daemon route, a `grok -p` turn that keeps a plan sends one follow-up that asks the agent to build the plan. The follow-up is sent when the turn ends with `end_turn`. `--permission-mode plan`, `--json-schema`, and `--max-turns` send no follow-up, and the kept plan stays unbuilt. A plan kept during the build stays unbuilt.

When the follow-up is sent, the result's `usage` sums both turns and `requestId` names the build turn. The result includes a cost when both turns name one and neither bill is partial or incomplete. When one turn names a cost and the other billed tokens and named none, or either bill is already partial, the result omits the cost and sets `cost_is_partial`. When both turns billed tokens and neither named a cost, and neither bill is partial or incomplete, the result omits the cost and sets neither flag. An incomplete bill omits the cost and sets `usage_is_incomplete`. A recorded planning bill omits the cost and sets `usage_is_incomplete` when the build turn brings no bill. A planning turn that brings no bill leaves the result incomplete, with no cost, whether or not the build turn brings a bill. When neither turn brings a bill, the result sets `usage_is_incomplete` and has no cost. The result omits `num_turns` when only one turn names it. A model only one turn used keeps that turn's call count. A model both turns used keeps a call count only when both turns named one.

---

## Environment Variables for Headless

Key environment variables that affect headless mode:

| Variable                        | Description                                                   |
| ------------------------------- | ------------------------------------------------------------- |
| `XAI_API_KEY`        | API key for authentication (required when no browser login)   |
| `GROK_HOME`                    | Override config directory (default: `~/.grok`)                |
| `GROK_LOG_FILE`                | Path to a log file (used verbatim as the path; works in headless and TUI, honors `RUST_LOG`) |
| `RUST_LOG`                     | Log level filter (e.g. `debug`). Headless logs to stderr.     |

For CI environments without browser access, set `XAI_API_KEY` with an API key from [console.x.ai](https://console.x.ai):

```bash
export XAI_API_KEY="xai-..."
grok -p "Run the test suite" --yolo
```

---

## Workflows and Background Work

A headless run waits for the background work it started before it exits, for up to 10 minutes after the latest turn ends. `--background-wait-timeout <secs>` sets another limit. When a background shell task or subagent finishes, the model gets its result in a new turn (an auto-wake turn, or the notification turn the idle drain runs when auto-wake is off); the headless run prints that reply after the first turn's text, as its own paragraph, and waits for that turn to end. The wait limit never cuts a running turn short: it counts from the end of the latest turn and applies only while the run is waiting on background work.

Work still running when the wait runs out is stopped, and the run then exits `1` and says what it stopped, such as `the wait for background work ran out after 600 s and exit stopped 1 background task (…)`. Pass `--no-wait-for-background` to exit as soon as the first turn ends; the work that exit stops was asked for and does not change the exit code.

`--background-shell-timeout <secs>` limits only background shells, and only when the session runs on the daemon: once shells have run that long after the latest turn ended, with no new turn or finished background task since, the run stops them without starting a turn for them and keeps waiting on background subagents with no limit. The run does not fail because of the stopped shells. It replaces the overall wait limit, so it can't be combined with `--background-wait-timeout`, and it doesn't limit the wait on an active goal.

On the daemon route, a workflow run is background work too. When a run the model started in the background ends, the model gets the run's result in a new turn. The headless run prints that reply after the first turn's text, then exits. With `--output-format json`, the reply is part of `text`. The wait also ends when the only background work left is runs paused at their agent budget: a headless run cannot raise a budget. The exit then stops those runs, so a later resume cannot raise it either.

Workflow commands work in a headless run on the daemon route:

- `grok -p '/<name> [--agent-budget N] [args]'` and `grok -p '/workflow <name> [args]'` start a listed workflow directly, with the budget you pass, and wait for it as above. A start that is refused, such as `/deep-research` with no query, fails the headless run with the reason. A `.rhai` workflow the daemon cannot start still goes to the model as text, and `/workflow list` says so.
- `grok -p '/workflow list'` and `grok -p '/workflow help'` answer at once, with no model turn. So do `/workflow runs`, `stop` and `resume`, but each headless run starts its own daemon, so they see no run of an earlier headless run. A `/workflow` command that fails, such as `stop` with no run to stop, fails the headless run with the reason.
- In a folder you have not trusted, Grok does not read the project's `.grok/workflows`. `/workflow list` names the workflows it left out, and `/<name>` or `/workflow <name>` for one of them fails the headless run with that reason. Pass `--trust` to trust the folder.

The newest background run of each workflow decides the exit code, unless a newer run of it completed. If that run did not complete, the headless run exits `1`, and stderr says which run and why:

| The newest run                            | What stderr says                                                       |
| ----------------------------------------- | ---------------------------------------------------------------------- |
| Failed, including a run that failed to start | `workflow gate (run wf_…) failed: Error: GATE-FAILED: 2 checks red`  |
| Paused at its agent budget, which a headless run cannot raise | `workflow deep-research (run wf_…) paused at its agent budget (3 of 3 agents used)` |
| Still running when the wait ran out         | `workflow slow (run wf_…) was stopped after 600 s: the wait for background work ran out. …` |
| Still running at exit after the wait ended, such as a run a goal's later turn started | `workflow slow (run wf_…) was still active at exit and was stopped: it was still going after the wait for background work ended` |

A run that failed and was then rerun to completion in the same headless run, in the background or the foreground, does not fail it. A run the model ran in the foreground never fails it: its result reached the model in that turn. If the runs cannot be read after the session had workflow runs, after a goal went on to another turn once the wait ended, or after the wait ran out, the run exits `1` and says so; otherwise the exit does not list them. Other background work stopped when the wait runs out, such as a shell, fails the run too (`… exit stopped 2 background jobs`). Outside a goal, if a turn the agent runs after the prompt fails, such as the turn that answers with a workflow's result or a finished background task's, the run exits `1` and stderr says why: `a turn the agent ran after the prompt failed: …`. A turn cut off at its output limit fails the run too, as a truncated prompt does: `a turn the agent ran after the prompt stopped at its output limit, so its reply is cut short`. A turn that ends normally does not. A goal's own turns are reported by the goal's outcome.

With `--output-format json` or a streaming format, the output ends with the error object in place of the result, without the session ID. To resume such a session, choose its ID up front with `-s <uuid>`. A run stopped when the wait runs out leaves no result.

---

## Exit Codes

| Code | Meaning                              |
| ---- | ------------------------------------ |
| `0`  | Success. The prompt completed normally |
| `1`  | Error. Authentication failure, network error, or runtime error. On the daemon route, also a workflow run that did not finish, or, outside a goal, a failed turn the agent ran after the prompt (see [Workflows and Background Work](#workflows-and-background-work)) |
| `130` | Interrupted by SIGINT (Ctrl+C)                                   |
| `143` | Terminated by SIGTERM                                            |

---

## Authentication for Headless Environments

For headless use, authenticate with one of:

- **`XAI_API_KEY`**: simplest for CI. See [Environment Variables](#environment-variables-for-headless) above.
- **`grok login --device-auth`** (or `--device-code`): no browser needed on the target machine.
  See [Authentication > Device Code Flow](02-authentication.md#device-code-flow).
- **`grok login`**: browser-based OAuth2 on machines with a GUI.

If you've previously logged in, cached credentials are used automatically.

---

## Tips

- Headless mode starts a **fresh session by default**. Use `-r/--resume` or `-c/--continue` to maintain context across calls.
- The `--output-format json` response always includes a `sessionId` you can use with `--resume` for follow-up calls.
- Combine `--yolo` with `--rules` to set guardrails: `grok -p "..." --yolo --rules "Never delete files"`.
- For debugging, raise the log level and capture stderr: `RUST_LOG=debug grok -p "..." 2> debug.log`.

---

## Project Root Discovery

When Grok starts, it discovers the project root by walking upward from `--cwd`
(or the current directory) until it finds a `.git` directory.

Note: If `--cwd` is nested inside a large repository (such as a monorepo),
Grok discovers that repository as the project root and scopes its discovery (AGENTS.md, skills, git history) to it, which can make
startup slow. Point `--cwd` at the specific subproject you want to work in to keep
the scope small.

---

## File Locations

Grok stores data in `~/.grok` (override with `GROK_HOME`; see [Environment Variables for Headless](#environment-variables-for-headless)):

| Path                     | Contents                              |
| ------------------------ | ------------------------------------- |
| `config.toml`            | User configuration                    |
| `auth.json`              | Cached OAuth2/API credentials         |
| `version.json`           | Version cache for update checks       |
| `sessions/`              | Session transcripts (SQLite)          |
| `memory/`                | Cross-session memory store            |
| `logs/`                  | Internal log files (for example `unified.jsonl`) |
| `logs/mcp/`              | MCP server logs                       |
| `skills/`                | User skill definitions                |
| `personas/`              | User-scoped agent personas            |
| `crash/`                 | Crash reports                         |
| `trace-exports/`         | Session trace exports                 |
| `worktrees/`             | Git worktree metadata                 |

### Read-Only `~/.grok`

For containers or CI, mount `~/.grok` read-only:

- Pre-populate `auth.json` or use `XAI_API_KEY`
- Session persistence fails silently (ephemeral)
- Update checks log a warning and skip

```bash
export XAI_API_KEY="xai-..."
export GROK_DISABLE_AUTOUPDATER=1
grok -p "..." --no-auto-update
```

---

## Update Check Suppression

| Method                          | Scope     |
| ------------------------------- | --------- |
| `--no-auto-update`              | Session   |
| `GROK_DISABLE_AUTOUPDATER=1`    | Process   |
| Non-TTY stderr (auto-detected)  | Automatic |
| `[cli] auto_update = false`     | Persistent|

`GROK_DISABLE_AUTOUPDATER` set to a falsy value (`0`, `false`, `off`, `no`, or empty, any
case) counts as not set. The agent SDKs
inject `GROK_DISABLE_AUTOUPDATER=1` for the non-leader agents they spawn (a falsy value in
the SDK's isolation env keeps updates on), and the stdio agent skips its background update
unless it runs from the managed install (`$GROK_HOME/bin/grok`).

Update messages go to **stderr**. Stdout stays clean for `--output-format json`. See also [Environment Variables for Headless](#environment-variables-for-headless).

---

## Additional Headless Flags

These flags supplement the [Command-Line Options](#command-line-options) table above. Flags already listed there (`--prompt-json`, `--prompt-file`, `--verbatim`, `--sandbox`, `--no-auto-update`) are not repeated here.

| Flag                          | Description                                       |
| ----------------------------- | ------------------------------------------------- |
| `--agent <NAME>`              | Agent name or definition file path                |
| `--agents <JSON>`             | Inline subagent definitions as JSON               |
| `--system-prompt-override`    | Override the agent's system prompt                |
| `--no-plan`                   | Disable plan mode                                 |
| `--no-subagents`              | Disable subagent spawning                         |
| `GROK_MEMORY=0`                | Disable cross-session memory for the process      |
| `--disable-web-search`        | Disable web search and fetch tools                |
| `--no-alt-screen`             | Run inline (no alternate screen)                  |
| `--worktree [NAME]`           | Create a git worktree from the current checkout (dirty changes included) and run the session there. Launching from a subdirectory lands in the same subdirectory of the worktree. For a new session, if the worktree lacks that directory, such as an ignored directory or one the `--worktree-ref` commit predates, the session opens at the worktree root and grok prints a note saying so. With `-r`, the session is resumed into the new worktree. Not combinable with `--fork-session`. Under a sandbox profile that can't make worktrees, such as `strict`, grok exits with an error before the session starts (see [Sandbox Mode](18-sandbox.md#worktrees)). |
| `--ref <REF>` / `--worktree-ref <REF>` | Branch/tag/commit to base the worktree on (with `--worktree`); a clean checkout, no dirty overlay |

---

## Interrupted Headless Runs

On SIGINT/SIGTERM:

- Session state saved up to the last completed tool call
- File modifications by tools are **not rolled back**
- Exit code is **130** for SIGINT (`128 + 2`) and **143** for SIGTERM (`128 + 15`); CI pipelines can distinguish these from a normal error (exit code `1`)
- Resume: `grok -p "continue" --resume "<id>"` or `grok -p "continue" --continue`

See [Session Management in Headless Mode](#session-management-in-headless-mode) for details on named sessions and the `-s`/`-r`/`-c` flags.
