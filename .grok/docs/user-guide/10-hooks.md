# Hooks

Hooks let you run a script or send an HTTP request at key moments in a Grok session. Use them to automate tasks, enforce safety checks, log activity, send notifications, and integrate your own tools.

---

## What Are Hooks?

A hook is a shell command or HTTP endpoint that Grok calls when a specific lifecycle event occurs. Hooks can:

- **Block actions**: A `PreToolUse` hook can deny a dangerous command before it runs.
- **Keep the agent working**: A `Stop` hook can block the agent from finishing its turn until a condition holds (e.g. the test suite passes) and feed the reason back to the model.
- **React to events**: A `PostToolUse` hook can log every tool execution to a file.
- **Correct a call after it ran**: A `PostToolUse` hook can tell the model what a tool result means, or replace the output the model reads — redact a secret, trim a wall of log lines — while the real result stays on the record.
- **Set up context**: A `SessionStart` hook can export environment variables or run setup scripts.

---

## Common Use Cases

- **Safety guards**: Block commands such as `rm -rf /` before they run.
- **Audit logging**: Record tool use and sessions to a file or external service.
- **Notifications**: Send a message when a task finishes.
- **Auto-formatting**: Run `cargo fmt` or `prettier` after edits.
- **Environment setup**: Export variables at session start.
- **Custom workflows**: Trigger builds, tests, or deployments on specific events.

---

## Quick Start

1. Create the hooks directory:

   ```sh
   mkdir -p ~/.grok/hooks
   ```

2. Create a hook file, e.g. `~/.grok/hooks/session-start.json`:

   ```json
   {
     "hooks": {
       "SessionStart": [
         {
           "hooks": [
             { "type": "command", "command": "echo \"Grok session started in workspace $GROK_WORKSPACE_ROOT\"" }
           ]
         }
       ]
     }
   }
   ```

3. Start (or restart) a Grok session. The hook runs automatically on `SessionStart`.

   This hook runs in the workspace root, the root of the git repository or worktree you launch Grok in. Outside a repository, or where Grok reads only the launch directory (see [Hook Locations](#hook-locations)), the workspace root is the launch directory. A package's own hooks run there too. Launched from a subdirectory, `$(pwd)` in this hook prints the workspace root, not the subdirectory. A hook runs in that directory with symlinks resolved when the hooks loaded. A link that changes later can't move it. The directory the session started in is the `cwd` field of the event on stdin (see [Input](#input)), spelled as you launched Grok.

4. Press `Ctrl+L` on non–VS Code family terminals (or run `/hooks` anywhere — preferred on VS Code family) and check the Hooks tab to confirm it loaded.

---

## Hook Locations

Hooks are discovered from several places (all are merged):

| Scope | Path | Trusted? | Notes |
|-------|------|----------|-------|
| Global | `~/.grok/hooks/*.json` | After folder trust | Personal hooks |
| Global | `~/.claude/settings.json` (and `settings.local.json`) | After folder trust | Claude Code compatibility (configurable) |
| Global | `~/.cursor/hooks.json` | After folder trust | Cursor compatibility (configurable) |
| Project | `<project>/.grok/hooks/*.json` | Requires trust | Per-repo automation |
| Project | `<project>/.claude/settings.json` (and `settings.local.json`) | Requires trust | Claude compatibility (configurable) |
| Project | `<project>/.cursor/hooks.json` | Requires trust | Cursor compatibility (configurable) |
| Config | `~/.grok/config.toml` | After folder trust | Your hooks alongside the rest of your config |
| Config | `managed_config.toml` (`$GROK_HOME` and `/etc/grok`) | Always | Organization-distributed hooks (server-synced and on-device) |
| Config | `requirements.toml` (signed cache, and `/etc/grok`) | Always | Organization-enforced hooks; see [Enforced hooks](#enforced-hooks) |
| Config | `~/.grok/requirements.toml` without a matching signature | After folder trust | Loads as your own hooks |
| Plugin | Bundled inside installed plugins | Per-plugin, after folder trust | Shared team hooks |

`<project>` is the repository root and each directory below it down to the one Grok was launched in. A monorepo package's own hook files load alongside the root's. A package's hooks are named `project/<package path>/…` and run in the workspace root like the root's, with `CLAUDE_PROJECT_DIR` set to the workspace root. A package hook written as `"$CLAUDE_PROJECT_DIR"/.claude/hooks/deny.sh` runs the root's script. When the root has no such script, the hook can't start and fails open, like any hook that can't start. Grok reports a hook error and the call goes ahead, even for a deny hook. To run the package's own script, write `"$CLAUDE_PROJECT_DIR"/packages/app/.claude/hooks/deny.sh`. A handler that both the root's files and a package's name runs once. A hook file that resolves to one an earlier source already loaded, such as a package's `.claude` linked to the root's or a project file linked to `~/.claude/settings.json`, loads once, from that earlier source. Outside a repository, only the launch directory is read. So is a repository rooted at a filesystem or drive root. So is a repository rooted at or above a shared temporary directory such as `/tmp`. For a launch inside your home directory, so is a repository rooted at your home directory or one of its parents. A repository git can't open, such as one a `safe.directory` check refuses, loads no project hooks. Grok reports the error. Launched in your home directory, at a filesystem or drive root, or in a shared temporary directory such as `/tmp`, Grok loads no project hooks.

Config-file hooks live in the same TOML your organization already controls; see [Hooks in Config Files](#hooks-in-config-files) for the format. The compatible vendor hook sources are scanned by default. To disable scanning for a specific vendor, set `[compat.<vendor>] hooks = false` in `~/.grok/config.toml` or the corresponding environment variable. See [Configuration](05-configuration.md#harness-compatibility) for details.

**Trusting a project**: The first time you open a project with hooks, you must trust it before its project hooks will run; until then they are silently skipped. Grant trust by running `/hooks-trust` (or launching with `--trust`); the decision is recorded in the unified folder-trust store (`~/.grok/trusted_folders.toml`), the same gate that governs repo-local MCP/LSP servers. As in Claude Code, a folder that isn't trusted also holds back your own hooks: those in `~/.grok/hooks/`, `hooks-paths`, `~/.grok/config.toml`, an unsigned `~/.grok/requirements.toml`, `~/.claude/settings.json` and `~/.cursor/hooks.json`, your agents' frontmatter `hooks:`, and your plugins' hooks. While you have such a hook, folder trust asks in every folder but your home directory and filesystem or drive roots, which count as trusted. Only your organization's `managed_config.toml` and `requirements.toml` hooks run before you trust a folder. A headless run, such as `grok -p`, counts the folder as trusted for your own hooks, but project hooks and a project agent's own hooks still need the folder trusted. In that run, a command in your `~/.claude` settings or an agent's `hooks:` that is only a relative path, such as `.claude/hooks/check.sh`, fails with a message saying the folder isn't trusted, while one with arguments or shell syntax still runs. This prevents untrusted repos from running arbitrary code.

Because hooks are unified under folder-trust, a `--trust` / `/hooks-trust` grant trusts the whole folder for **MCP, LSP, hooks, project instructions, and project skills** together, and covers subdirectories of the same repository. A nested git checkout under that folder is a separate workspace and is not covered. Conversely, disabling folder-trust (`GROK_FOLDER_TRUST=0` or `[folder_trust] enabled = false`) ungates those surfaces together.

---

## Hook Events

Events fire at three cadences: once per session (`SessionStart`, `SessionEnd`), once per turn (`UserPromptSubmit`, `Stop`, `StopFailure`), and on every tool call inside the turn (`PreToolUse`, `PostToolUse`, `PostToolUseFailure`).

| Event | When it fires | Blocking? |
|-------|---------------|-----------|
| `SessionStart` | A session starts, or Grok reopens a saved one. Does not fire for a subagent's own session. Compaction fires only `PreCompact` and `PostCompact`, never `SessionStart`. | No |
| `UserPromptSubmit` | You submit a prompt. | Yes: can block the prompt |
| `PreToolUse` | A tool is about to run. | Yes: can deny |
| `PostToolUse` | A tool finishes running (including a built-in logical error such as a non-zero `run_terminal_command` exit; a dispatch failure or an MCP error result fires `PostToolUseFailure` instead). | No, but it can feed the model feedback and replace the output the model sees |
| `PostToolUseFailure` | A tool fails to dispatch, or an MCP tool returns an error result. | No, but it can feed the model `additionalContext` and replace the error text the model sees |
| `PermissionDenied` | The permission system denies a tool call. | No |
| `Stop` | An agent turn ends on a genuine completion (an interrupt fires `StopCancelled` instead). | Yes: can block the stop |
| `StopFailure` | A turn ends because of an API error. A `systemMessage` from its hooks is not shown. | No |
| `StopCancelled` | Runs instead of `Stop` when a turn ends without completing: a user interrupt (Ctrl+C / a client stop), a declined permission prompt, the `--max-turns` limit, a no-progress bail-out, or a tool hook's `continue: false`. | No |
| `Notification` | User-attention events (`idle_prompt`, `permission_prompt`, `task_complete`, …). | No |
| `SubagentStart` | A subagent starts. | No |
| `SubagentStop` | A subagent's turn ends (fires once, in the subagent, with stop decision control). | Yes: can block the stop |
| `PreCompact` | Conversation compaction is about to run. A `systemMessage` from its hooks is not shown. | No |
| `PostCompact` | Conversation compaction completes. A `systemMessage` from its hooks is not shown. | No |
| `SessionEnd` | The session ends. Carries `subagentType` for a child session, so a host can tell a child's teardown from its own. A `systemMessage` from its hooks is not shown. | No |

`SubagentEnd` is accepted as an alias for `SubagentStop`. `PreToolUse` can block a tool call, `UserPromptSubmit` can block a prompt (see below), and `Stop`/`SubagentStop` can block the agent from stopping (see [Stop Decision Control](#stop-decision-control)). `PostToolUse` runs too late to block anything, but its stdout is read: it can feed the model feedback and replace the tool output the model sees (see [PostToolUse Output](#posttooluse-output)). Every other event is passive.

### UserPromptSubmit Decision Control

A `UserPromptSubmit` hook can reject a prompt: exit 2 blocks (stderr becomes the message), and so does JSON `{"decision": "block", "reason": "..."}` on stdout, on any exit code. `{"continue": false, "stopReason": "..."}` blocks it too, whatever the decision says. Its message is `stopReason`, else the block `reason`, else the hook's trimmed stderr, else "Prompt blocked by hook '<name>'". The reason is shown to you and is never added to the model's context. Only a prompt you typed can be blocked: auto-wake turns (task and subagent completions, scheduler fires) and subagent sessions run the hook observe-only. The default timeout for this event is 30 seconds; a timed-out or crashed hook fails open and the prompt proceeds.
A reply past a read limit blocks at any exit code but 2 (see [Reading a Hook's Reply](#reading-a-hooks-reply)).

After a block, prompts already queued behind the blocked one do not auto-run: the queue holds until you act (send a prompt, or edit / remove / reorder / force-run a queued row). A blocked prompt is not recorded: it never enters the conversation history the model sees on later turns, the on-disk session record, or the session summary. It stays visible in the live scrollback and is held at the front of the queue for you to edit, resend, or discard — but after a session restart the blocked bubble is gone from the scrollback, exactly because nothing was stored. One deliberate exception: your client's local prompt history (the up-arrow recall) keeps the text, recorded at submit time before the hook runs, so a discarded prompt is still recoverable. One current limit: stdout of an allowing hook is discarded (no `additionalContext`).

### Cursor Hook Compatibility

Grok runs the matcher groups in `~/.cursor/hooks.json` and `.cursor/hooks.json`.
A matcher group is an entry with a `hooks` list, the shape a Claude settings file holds.
Cursor's own entries, written straight under the event as `{"command": "...", "matcher": "...", "timeout": 30}`, don't run yet.
A Cursor hook answers in Cursor's reply format (`permission`, `continue`).
Grok doesn't read that format yet.
A deny such a hook printed would not block.
Each such entry under an event Grok knows is reported as the load error "this Cursor hook won't run until Grok reads Cursor hook replies".
The rest of the file still loads.
An entry under an event Grok doesn't know, such as Cursor's `afterFileEdit`, is skipped with its event and only logged.
Every other hook source (Claude settings, plugin `hooks.json`, Grok hook files and `config.toml`) reads only matcher groups.
An entry there without a `hooks` list is a load error and never runs.

Grok accepts Cursor's camelCase hook event names:

| Cursor event | Maps to |
|---|---|
| `sessionStart`, `sessionEnd` | `SessionStart`, `SessionEnd` |
| `preToolUse`, `postToolUse`, `postToolUseFailure` | `PreToolUse`, `PostToolUse`, `PostToolUseFailure` |
| `beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile` | `PreToolUse` |
| `afterShellExecution`, `afterMCPExecution`, `afterFileEdit` | `PostToolUse` |
| `afterAgentResponse`, `afterAgentThought` | `PostToolUse` |
| `beforeSubmitPrompt` | `UserPromptSubmit` |
| `subagentStart`, `subagentStop` | `SubagentStart`, `SubagentStop` |
| `preCompact`, `stop` | `PreCompact`, `Stop` |

Cursor's per-operation hooks (`beforeShellExecution`, `afterFileEdit`, etc.) map to the generic `PreToolUse`/`PostToolUse` events. The hook script receives the tool name in the JSON input and can filter accordingly, or use the `matcher` field.

---

## The Hook JSON Format

Each `.json` file can define hooks for multiple events:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "bin/safety-check.sh", "timeout": 10 }
        ]
      }
    ],
    "PostToolUse": [
      {
        "hooks": [
          { "type": "command", "command": "bin/log-activity.sh" }
        ]
      }
    ]
  }
}
```

### Key Fields

- **Event name** (top-level key): any event listed in [Hook Events](#hook-events). Grok skips unrecognized event names so a shared Claude or Cursor settings file still loads.
- **matcher** (optional): A name, a list of names, or a regular expression. It picks the invocations that trigger the hook (see [Matcher Syntax](#matcher-syntax)). What it tests depends on the event: the tool name on tool events (`PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PermissionDenied`), the notification type on `Notification`, the subagent type on `SubagentStart`/`SubagentStop` (e.g. `explore`), the start source on `SessionStart` (`new` for a fresh session, including one from `/new` or `/clear`, or `load` for a saved session reopened with `--resume`, `--continue`, or the `/resume` picker; Claude Code's `startup`, `resume`, `clear`, and `compact` never match), the end reason on `SessionEnd`, the compaction trigger on `PreCompact`/`PostCompact` (`manual` or `auto`), the error type on `StopFailure` (`rate_limit`, `authentication_failed`, `invalid_request`, `server_error`, `max_output_tokens`, or `unknown`), and the reason on `StopCancelled` (`user_interrupt`, `permission_rejected`, `permission_cancelled`, `max_turns`, `no_progress`, `hook_stopped`, or `unknown`). A matcher on `Stop` or `UserPromptSubmit` is ignored with a warning (those events always fire). An empty or omitted matcher matches everything. A finish-thinking chime should set `matcher` to `idle_prompt` on `Notification` (any turn end, then sustained idle). `permission_prompt` fires only when a permission UI is actually waiting. The matcher tests the real tool name. MCP calls routed through the internal `use_tool` / Cursor `CallMcpTool` dispatchers appear as the qualified `server__tool` name (e.g. `linear__save_issue`). Match on that name, not the dispatcher name. A matcher can also use Claude Code's `mcp__server__tool` name.
- **type**: `"command"` (run a script or shell one-liner) or `"http"` (POST the event to a URL).
- **command**: Path to executable (relative to the JSON file) or inline shell command. A bare name such as `my-guard` runs the file of that name beside the JSON file, or the `my-guard` on `PATH` when there is none. A bare name ignores relative `PATH` entries such as `.` or `node_modules/.bin`. In a Claude settings file (`.claude/settings.json` or `.claude/settings.local.json`), a relative path with a `/`, such as `.claude/hooks/check.sh`, is taken from the workspace root, the folder Claude runs it from. Quotes around a whole command that is one path only group it, spaces and apostrophes included. `"/opt/my hooks/guard.sh"` runs `/opt/my hooks/guard.sh`, and Grok starts it directly, as it does an unquoted path, so on Linux and macOS a script needs a `#!` line, where `sh -c` once ran one without it. `".claude/O'Brien hooks/check.sh"` in a Claude settings file is a relative path like any other. Single quotes keep everything inside as it is, so `'/opt/R&D/$5/guard.sh'` is one path too. A double-quoted path that holds `$` or a backtick still runs in the shell, which expands it. Under `sh` and Git Bash, so does one with a `\` before another `\` or a line break, or at its end, while `"bin\check.sh"` names the file `bin\check.sh`. Under cmd, so does one that holds `%`, such as `"%ProgramFiles%\My Tool\guard.exe"`. `sh` and cmd read a `~` inside quotes as a folder of that name, so `"~/guard.sh"` is a relative path like any other. PowerShell reads a leading `~` as your home folder, so there a quoted word that starts with `~` runs in the shell, and so does one that isn't a program path, such as a message or a JSON reply. Under PowerShell, a command's opening quoted word runs as a program in the cases [Shell Commands Under PowerShell](#shell-commands-under-powershell) lists. On Windows, a `.ps1` path or bare name runs with `-File` under the detected PowerShell. Under Git Bash or cmd, that is pwsh when it is on `PATH`, else Windows PowerShell. Windows starts an `.exe`, `.cmd`, `.bat` or `.com` path or bare name itself. Under Git Bash, any other one, such as a `.sh` or `.py` script or a file with no extension, runs through Git Bash, which reads its `#!` line. Under PowerShell or cmd, a file with another extension fails with a message instead, since they would open it with its file type's app and pass it none of the hook's input. That holds whatever your `PATHEXT` lists: under PowerShell, start the command with `&`, as in `& 'C:\hooks\guard.py'`, to call a file whose extension it adds. Grok starts every hook's PowerShell with `-ExecutionPolicy Bypass` and sets `PSExecutionPolicyPreference=Bypass` for every hook, so a PowerShell the hook starts runs scripts too, under any shell. A Group Policy execution policy still applies.
- **args** (optional): A present `args` array, including `[]`, makes `command` one program. The elements are its arguments. The runner starts no shell. A command that carried its own arguments puts them in `args`. `command` stays the program name or path. Each element is one argument. Spaces, quotes, `$` and backticks stay as written. Grok fills an argument from the handler's `env`, from the plugin variables when the plugin set them (`${CLAUDE_PLUGIN_DATA}` only when the plugin has a data folder), and from the runner variables (`GROK_HOOK_EVENT`, `GROK_HOOK_NAME`, `GROK_SESSION_ID`, `GROK_WORKSPACE_ROOT`, `CLAUDE_PROJECT_DIR`), in both `$NAME` and `${NAME}` form. A `command` that starts with `${CLAUDE_PROJECT_DIR}` or `$CLAUDE_PROJECT_DIR` and a path separator is a program under the workspace root when the rest is a non-empty relative path. Any other name stays as written. That includes `$HOME`, an unset name, `${NAME:-default}`, `~`, quotes and globs. An `http` handler ignores `args`.
- **timeout**: Seconds before killing the hook, any non-negative number such as `1.5` (default: 5, or 600 for `Stop`/`SubagentStop`/`PostToolUse`).
  A `timeout` of `0` keeps the default.
  All hook failures (timeouts, crashes, malformed output) are fail-open: the failure is recorded for the UI scrollback but the tool call is not blocked.
  Only an explicit `deny` decision, a reply past a read limit, or an `updatedInput` that can't be applied blocks a tool call.
  An `updatedInput` can't be applied when it nests past 124 levels or fails the tool's schema (see [Reading a Hook's Reply](#reading-a-hooks-reply)).

### Shell Commands Under PowerShell

On Windows under pwsh or Windows PowerShell, `sh -c` would run a command's opening quoted word as a program, but PowerShell reads it as a string. The opening word is the first one past any blank lines, comments and backtick line continuations, and past a leading `&` and any comment or continuation on its line. Grok puts the `&` call operator in front of it in these cases.

The opening quoted word runs as a program when:

- Arguments follow it: `"C:\Program Files\nodejs\node.exe" guard.js`.
- It's alone, or before `;`, a line break, a `#` comment, `&&`, `||` or `&`, and it's a program path. A program path holds no `://`, and ends in `.exe`, `.cmd`, `.bat`, `.com` or `.ps1` or holds `\` or `/`. With a space, it must hold `\` or `/`, and either end in one of those extensions, start at a root (`C:\`, `\\server\`, `.\`, `..\`, `~\`, `$env:ProgramFiles\`, the same with `/`, or a `\` or `/` with another after it), or end in another extension with a letter, such as `.sh`, with its first `\` or `/` before its first space. So `'Blocked: see docs/deploy.md'` is a message, and so is `"my hooks/check.sh"`: write it as `"./my hooks/check.sh"`.
- The rest of the path follows the quoted folder directly: `"$CLAUDE_PROJECT_DIR"/.claude/hooks/check.cmd` runs as one program.
- The command starts with `$CLAUDE_PROJECT_DIR` or another hook variable.

It stays a string, as PowerShell reads it, when:

- It isn't a program path, such as `'Use pnpm, not npm'` or `'https://wiki/deploy-policy'`, or it holds `"` or `$(…)`, such as a JSON reply. PowerShell prints it.
- A pipe, a redirect such as `2>&1`, or text glued to it such as `.Length` or `[0]` follows it.
- A PowerShell operator and its value follow it, such as `- 1` or `-eq 'x'`, or `-f` and a value after a string holding `{`.

To run such a word anyway, start the command with `& "…"`.

It isn't run, and its call fails with a message instead, when:

- It's a file Windows can't start as a program, such as a `.sh`, `.py` or `.js` script, also when your own `&` calls it with a quoted path or one that starts with a hook variable. PowerShell would open it with its file type's app and pass it none of the hook's input. An unquoted path, such as `& .\check.sh` or `C:\hooks\check.sh --strict`, isn't caught and opens that way.

In a shell command, a file whose extension your `PATHEXT` adds, such as `.py`, is called as PowerShell calls it. That's the hook's `env` `PATHEXT` when it sets one. A command that is only such a file's literal path, quoted or not, or its bare name isn't shell text, so it still fails (see **command** above), with a message naming the `& '<full path>'` to write instead. A path that starts at a hook variable or, under PowerShell, a quoted `~`, such as `"$CLAUDE_PROJECT_DIR"/hooks/guard.py` or `"~\hooks\guard.py"`, is shell text, so it's called. Windows' own Windows Script Host and management console entries, such as `.js`, `.vbs` and `.msc`, aren't, since they open in a window of their own.

The statements after a call that isn't run still run, so a trailing `; exit 2` or `& exit 2` still blocks. Start the command with the program that runs the file, such as `python` or Git Bash's `"C:\Program Files\Git\bin\bash.exe"`. A bare `bash` under PowerShell is WSL's or none, since Git for Windows puts only `Git\cmd` on `PATH`. In settings shared with other platforms or with Claude Code, set `GROK_SHELL=bash` instead, so the command runs under Git Bash.

### Matcher Syntax

The characters in a matcher decide how Grok reads it. Claude Code uses the same rules.

| Matcher | Read as | Example |
|---------|---------|---------|
| `"*"`, `""`, or omitted | Match everything | Fires on every occurrence of the event |
| Only letters, digits, `_`, `-`, spaces, `,`, and `\|` | One name, or a list of names separated by `\|` or `,`. Spaces around a name are ignored. Every name is compared exactly. | `read_file` matches only that tool. `Edit\|Write` and `Edit, Write` each match either tool. `code-reviewer` matches only that agent type. |
| Anything else | A regular expression that can match any part of the name, or any Claude or client tool name that maps to the tool in the alias table (see [Tool Name Aliases](#tool-name-aliases)) | `^Notebook` matches any tool whose name starts with `Notebook`, and also `search_replace`, `hashline_edit`, `StrReplace`, and `EditNotebook`, because `NotebookEdit` maps to them. `Edit.*` matches `Edit` and `NotebookEdit`. |

A `StopFailure` matcher lists error types only with `|`.
A `,`, space, or `-` in it makes it a regular expression.
`rate_limit|server_error` matches either type, and `rate_limit,server_error` and `rate_limit, server_error` match neither.
Grok logs a warning when it loads a `StopFailure` matcher written as a list of names with a `,`, space, or `-`.

Anchor a regular expression with `^` and `$` (as in `^Edit$`) when you need a whole-name match.

Grok names an MCP tool `<server>__<tool>`, for example `github__search_repositories`. Claude Code names it `mcp__<server>__<tool>`. Grok also tests a matcher that contains `mcp__` against the Claude Code name. An imported `mcp__github__.*` and a native `github__.*` both select every tool from the `github` server. `mcp__github__search_repositories` matches only that one tool. A bare `mcp__github` matches no tool, in Grok or in Claude Code. Add `__.*` to match every tool on the server. A matcher without `mcp__` is never tested against the Claude Code name.

### Unsupported Handler Fields

Grok reads only `type`, `command`, `args`, `url`, `timeout`, and `env` from a handler. When a handler that will run sets one of the Claude Code fields below, Grok records a hook load error that names the field and the hook file. A handler that cannot run reports its type or missing field. The error is written to the tracing log (`hook loading error` at startup, `hook reload error` on reload, and `hook loading` or `hook loading from settings file` while a hook file is read) and returned as `load_errors` on the hooks protocol. The pager's Hooks tab lists each load error as an `Error:` row above the hook groups. Pressing `Enter` on the row shows the whole message. `grok inspect` lists them under Hook Load Errors. A skipped handler has no hook row of its own. The other handlers in the file still load.

| Field | What Grok does |
|-------|----------------|
| `if` | Skips the handler. An [enforced hook](#enforced-hooks) still runs. Without `if`, the handler would run on every call its matcher selects. A skipped handler's error names only this field, not the fields below. |
| `async`, `asyncRewake`, `shell`, `statusMessage`, `headers` | Loads the handler and runs it without the field. The handler runs in the foreground. A command without `args` runs as `sh -c` on Unix and under the detected Windows shell (pwsh, Windows PowerShell, or Git Bash) on Windows. There is no status message and no extra HTTP headers. |

Grok does not report a field whose value asks for nothing. Those values are `null`, `false`, an empty string, an empty list, and an empty object. For example, a handler with `"statusMessage": ""` loads normally. A non-empty `shell` is reported, including `"bash"`. The command is not run under that shell. Grok ignores any other key it does not recognize.

### Tool Name Aliases

In a `matcher`, Grok maps Claude-style tool names to its own so hooks migrated from Claude fire correctly. Common aliases include:

- `Bash` → `run_terminal_command`
- `Read` → `read_file`
- `Edit`, `Write`, and `MultiEdit` → `search_replace`
- `Grep` → `grep`
- `Glob` and `LS` → `list_dir`
- `WebSearch` → `web_search`
- `Task` → `spawn_subagent`

A matcher keeps its original name too, so `Bash` matches both `Bash` and `run_terminal_command`.

Under the Cursor preset (`agent_type = "cursor"`), the Claude names also match that preset's tools:

- `Bash` → `Shell`
- `Edit`, `Write`, `MultiEdit`, and `NotebookEdit` → `StrReplace` and `EditNotebook`
- `AskUserQuestion` → `AskQuestion`
- `TaskOutput`, `BashOutput`, `BashOutputTool`, and `AgentOutputTool` → `AwaitShell`
- `Agent` → `Task`

These tools give hooks their own input keys.
`StrReplace` and `Write` name the file in `path`, and `Write` holds the text in `contents`.
`EditNotebook` names the notebook in `target_notebook`.
A hook that reads `tool_input.file_path` or `tool_input.content` reads nothing for these tools.

The client tool names also work as matcher terms on every route:

- `Shell` → `run_terminal_command`
- `StrReplace` and `EditNotebook` → `search_replace` and `hashline_edit`
- `List` → `list_dir`
- `Fetch` → `web_fetch`

An `EditNotebook` matcher fires on every `search_replace` and `hashline_edit` edit on every route, because a notebook edit reaches daemon hooks as `search_replace`.
Under the Cursor preset, it still doesn't match `StrReplace`.
A regular expression also widens with these names: `^List` and `List$` match `list_dir`, `^Fetch` matches `web_fetch`, and `Notebook$`, `Replace` or `^Str` match every `search_replace` and `hashline_edit` edit.

On the daemon route, a file tool's event names `read_file`, `grep`, `search_replace` or `write` under either file toolset.
A `hashline_*` matcher never fires there.
Match `Read`, `Grep`, `Edit` or `Write` to select the tool on both routes.

---

## How a Hook Resolves

When an event fires, Grok resolves it in four steps:

1. **Select matching groups.** For that event, each matcher group whose `matcher` matches the event's field runs. The matcher tests the tool name on tool events, the notification type on `Notification`, and so on (see [Key Fields](#key-fields)). An empty or omitted matcher matches everything.
2. **Run the handlers in order.** Handlers in the selected groups run in config order, each receiving the event as JSON on stdin, until one returns `deny` (which stops the chain). Handlers from different sources (global, project, plugin, config) are merged, and a handler that several selected groups name runs once per call, even when their matchers differ. Two handlers are the same when they run the same thing with the same arguments and the same `env`: the same URL, or the same command resolved the same way with the same `args`. A relative command such as `guard.sh` resolves against its own hook file's directory, so `guard.sh` in `~/.grok/hooks/` and in `.grok/hooks/` are two handlers. Every copy loads, and `/hooks` lists each one. When copies differ in `timeout`, the first copy in config order runs with its own timeout, unless a later copy has higher authority (a managed one, for example), which then runs in the first copy's place. A plugin's or agent's handler is its own, so it never merges with another plugin's or agent's. Every handler sees the model's original tool input; a `PreToolUse` `updatedInput` is applied only after all handlers finish, so one handler cannot see another's rewrite (the last rewrite wins).
3. **Apply the decision.** For a `PreToolUse` gate, the first `deny` blocks the call and its reason is shown to the model, an `updatedInput` rewrites the tool input, and otherwise the call proceeds. For `Stop` and `SubagentStop`, a `block` keeps the agent working. For `PostToolUse` the tool has already run, so nothing is blocked and every hook runs: a `block` reason and any `additionalContext` are delivered to the model with the tool result, and an output replacement rewrites the model's copy of that result. Every other event is passive: its output is recorded but does not change control flow.
4. **Fail open.** A handler that times out, crashes, or emits malformed output is recorded in the scrollback but never blocks the action.
   There are three exceptions.
   A `PreToolUse` `updatedInput` that fails the tool's schema cannot run safely.
   Grok blocks the call and reports it as an invalid-input error.
   An `updatedInput` nested past 124 levels, from a hook that succeeded, denies the call.
   A `PreToolUse` or `UserPromptSubmit` reply past a read limit denies the call or blocks the prompt (see [Reading a Hook's Reply](#reading-a-hooks-reply)).
   Otherwise only an explicit `deny` blocks a tool call.

---

## Hooks in Config Files

Hooks can also live directly in your Grok config, so a team can distribute them with the rest of their configuration instead of shipping separate JSON files. The same `hooks` object is read from three TOML files:

| File | Tier | Who sets it |
|------|------|-------------|
| `~/.grok/config.toml` | User | You |
| `managed_config.toml` (`$GROK_HOME`, `/etc/grok`) | Managed / system | Your organization |
| `requirements.toml` (`$GROK_HOME` signed cache, `/etc/grok`) | Requirements | Your organization |

The TOML is structurally identical to the JSON hook object, so an existing hook transliterates directly:

```toml
[[hooks.PreToolUse]]
matcher = "Bash|Write|Edit"
hooks = [
  { type = "command", command = "/opt/guard/pretooluse.sh", timeout = 10 },
]
```

Each matcher group is a `[[hooks.<Event>]]` entry with an optional `matcher` and an inner `hooks` array of handlers. The handler fields (`type`, `command`, `args`, `url`, `timeout`, `env`) and event names are exactly the same as the [JSON format](#the-hook-json-format).

TOML offers two equivalent notations for the inner handlers, and both parse to the identical structure. The inline-table array shown above is recommended: it reads best for the common single-handler case. The nested array-of-tables form is also accepted:

```toml
[[hooks.PreToolUse]]
matcher = "Bash|Write|Edit"
[[hooks.PreToolUse.hooks]]
type = "command"
command = "/opt/guard/pretooluse.sh"
timeout = 10
```

Prefer the inline form to avoid repeating the `[[hooks.<Event>.hooks]]` header for each handler.

- **Additive across layers.** Every layer's hooks run; a lower-priority layer adds hooks but never replaces another layer's block. A hook defined identically in more than one layer loads from each, and runs once per call: the highest-authority copy runs.
- **Provenance labels.** Config hooks appear in `/hooks` tagged by origin (`managed:`, `requirements/signed:`, `requirements/user:`, `user:`, and so on) so you can see which layer contributed each one.
- **No read-time expansion.** A literal `${VAR}` in a `command` or `url` reaches the hook runner unchanged, matching JSON hook-file semantics; the runner performs the single expansion.

### Enforced hooks

Hooks from the config layers your organization controls are enforced: they carry a `[policy]` badge in `/hooks`, `Space` refuses to disable them, an entry in `~/.grok/disabled-hooks` does not skip them, and their source cannot be removed. Two layers qualify:

- The root-owned system files `/etc/grok/requirements.toml` and `/etc/grok/managed_config.toml` (`requirements/system:` and `system_managed:` names).
- The `~/.grok/requirements.toml` that the deployment sync writes, while its bytes match the signed policy the server sent (`requirements/signed:` names). If the file is edited or its signature file is missing or unreadable, its hooks load as your own (`requirements/user:` names) and can be disabled again. Organizations that need the file to stay intact set `fail_closed = true` in the same requirements. Grok then refuses to start on an edited copy or a missing signature. Grok refuses to start on a `requirements.toml` it cannot read or parse, even without `fail_closed`. See [Configuration](26-config-reference.md#requirementstoml).

Hooks in `~/.grok/managed_config.toml` and `~/.grok/config.toml` are distribution, not enforcement: you can disable them.

### Allow only managed hooks

An organization can restrict a machine to the hooks it enforces:

```toml
# /etc/grok/requirements.toml
allow_managed_hooks_only = true
```

With this set:

- **What runs.** Only [enforced hooks](#enforced-hooks): the ones from the root-owned `/etc/grok/requirements.toml` and `/etc/grok/managed_config.toml`, and the ones in the synced `~/.grok/requirements.toml` while it matches its signature. An edited synced file loads as your own hooks, which the pin skips.
- **What is skipped.** Every other hook, at dispatch: `~/.grok/hooks`, every `~/.grok/*.toml` file, project hooks, plugin hooks, agent frontmatter hooks, the Claude and Cursor compatibility files, and the pager's `[[ui.notifications.hooks]]` commands. In `/hooks` they show `[disabled]`, and enabling them is refused ("Hooks outside managed policy are disabled by your organization."); `grok inspect` names the file that set the pin.
- **What still runs.** Hooks the embedding client registers over ACP (an IDE or the desktop app), as under Claude Code's `allowManagedHooksOnly`.
- **Windows.** There is no root-owned layer (no `/etc/grok`), so only the signed synced `requirements.toml` hooks and ACP client hooks still run.

The key is a tighten-only policy pin: any native policy layer (`requirements.toml` or `managed_config.toml` in `$GROK_HOME` or `/etc/grok`, or macOS MDM) or Claude's `managed-settings.json` can set it, the camelCase `allowManagedHooksOnly` is accepted everywhere, no layer can release it, and a non-boolean value engages it. Grok reads it with the hooks each time it loads them, so a pin the deployment sync writes after startup applies to the next session and to `/hooks` Reload. Once engaged it stays engaged until Grok restarts, even if the file that set it changes. `grok inspect` lists it under **Enforced by policy** as "Hooks outside managed policy disabled" with the file that set it.

---

## Writing Hook Scripts

### Input

The event is sent as one line of JSON, ending in a newline, on **stdin** (for example, a `PreToolUse` event, reformatted here for readability; the payload also always includes `toolUseId` and `toolInputTruncated`):

```json
{
  "hookEventName": "pre_tool_use",
  "hook_event_name": "PreToolUse",
  "sessionId": "abc-123",
  "cwd": "/Users/you/project",
  "workspaceRoot": "/Users/you/project",
  "permissionMode": "default",
  "toolName": "run_terminal_command",
  "toolInput": { "command": "npm test" },
  "timestamp": "2026-04-14T12:00:00Z"
}
```

Every event carries the same common fields: `hookEventName`, `sessionId`, `cwd`, `workspaceRoot`, `timestamp`, `permissionMode` (`default`, `auto`, `plan`, or `bypassPermissions`), and `promptId` (the turn the event belongs to; absent for session-scoped events and for tool events), plus event-specific fields like `toolName` above. When an external runtime runs the tools, a tool event's `cwd` is the directory the step runs in. A shell call's own `cwd` comes first. It is read as text with no lookup on disk, as the runtime runs it. A `file://` URL reads as its path and a leading `~` as your home directory. A relative `cwd` joins the session's directory. A `..` drops the name before it, even when that name is a symlink. Grok's own shell sends the session's directory. A directory over 4,096 bytes does not resolve. The event's `cwd` is then the session's. A shell call still keeps its `cwd` in `toolInput`. A search without a path gets the sent `cwd` as its path. The `hook_event_name` (snake_case key) carries Claude's PascalCase value; `hookEventName` (camelCase key) carries grok's snake_case value.

A tool input whose JSON passes 128 KiB keeps every key. Its strings longer than one shared length are cut to it, each ending in a short tag of its whole value and ` [truncated]`, and `toolInputTruncated` is `true`. That length is the longest that fits 128 KiB but never under 4 KiB, so an input can stay past 128 KiB after its cut. A string of 4 KiB or less is never cut, so a field such as `file_path` stays whole, and an input with no longer string is sent whole with `toolInputTruncated` set to `false`. A cut string hides its end, so a guard that checks a string's text should deny unless `toolInputTruncated` is `false`. When `toolInputTruncated` is `true`, a cut string that `updatedInput` returns unchanged in its own place runs whole, even beside another string that starts the same way. A string or key the `updatedInput` adds or changes that still holds ` [truncated]` is refused, which covers a cut string edited, wrapped, or copied or moved to another field, such as `new_string` set to `old_string`. A transform that drops or encodes the mark runs as written, like any other replacement.

### Output (Blocking Hooks)

For `PreToolUse` hooks, write JSON to **stdout**:

- **Allow**: `{"decision": "allow"}`
- **Deny**: `{"decision": "deny", "reason": "Unsafe command detected"}`
- **Ask the user**: `{"decision": "ask", "reason": "Confirm this deploy"}`
- **State no opinion**: `{"decision": "defer"}`
- **Rewrite the tool input**: `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "updatedInput": {"command": "npm test"}}}`
- **Tell the model something**: `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": "This repo builds with xb, not cargo"}}`
- **Stop the agent**: `{"continue": false, "stopReason": "The build is red"}`. The call doesn't run and the turn ends, whatever the decision says. Calls earlier in the same response that were already approved still run, and later ones are skipped. You see the `stopReason`; the model doesn't. Queued prompts then wait, and no task, subagent or scheduler wake restarts the agent, until you send a prompt. A subagent's stop ends only its own turn. When an external runtime runs the tools, the call is denied but the turn goes on.

The decision can be written as top-level `decision` or as `hookSpecificOutput.permissionDecision`. Both take `allow`, `deny`, `ask`, or `defer` (the legacy `approve` and `block` spellings also work), each with its own reason field — `reason` and `permissionDecisionReason`. The canonical `permissionDecision` decides when present; the top-level `decision` applies only when it is absent. The deny or ask message is `permissionDecisionReason` if present, otherwise `reason`. An `allow` means only "not blocked" — it does not auto-approve a call the user would otherwise be asked about. A decision value outside that set is a hook failure, which fails open unless the hook also exits 2, in which case the deny stands and carries the mistake in its reason.

An `allow`, `ask` or `defer` reply does not lift an exit-2 deny.
At exit 2, a `deny` reply's reason replaces stderr as the deny message.
Claude Code behaves the same way.

An `ask` makes the call reach the permission prompt: nothing that would otherwise approve it without asking — always-approve mode, auto mode, a saved "always allow" grant, a safe command — applies, and the prompt names your hook and shows your reason. There is never a second prompt: where you would have been asked anyway, the ask only re-labels that one. Approving runs it; rejecting blocks it as an ordinary permission rejection. A client running in full always-approve/YOLO mode (auto-answering every prompt) will still auto-approve the call, matching Claude Code's `bypassPermissions`: the ask overrides the manager's always-approve, auto, saved-grant, and safe-command paths, not a client that blanket-approves every prompt.

An `ask` cannot widen anything, so a permission policy deny, an auto-mode block, or plan mode still decides the call. In auto mode a hook `ask` still runs the classifier before the prompt appears: the classifier may deny the call, but it can never silently approve one the hook asked about. `dontAsk` mode denies whatever it would have to prompt for, so there an ask turns an otherwise-approved call into a denial.

Only hooks configured in a settings file (command and HTTP hooks) can ask, defer, or send `additionalContext`: a `PreToolUse` hook registered through the grok-agent-sdk can allow or deny, and the rest is dropped — an `ask` or a `defer` there leaves the call to the normal permission flow and is logged as an unrecognized decision, and `additionalContext` never reaches the model.

`updatedInput` replaces the tool's input before it runs, silently: the model is not told and nothing is written to the scrollback, so the only sign of a rewrite is the rewritten arguments themselves, which the user sees if the call reaches a permission prompt. The plan-mode gate, the permission prompt, the tool itself, and the later `PostToolUse` payload all see the rewritten input, so a hook can normalize or harden a call rather than only allow or deny it. Hooks run before the plan-mode gate, so a hook with side effects fires even when plan mode later rejects the call.

The value must be a JSON object; a non-object fails the hook. If the rewritten input fails the tool's schema, the call is blocked as a hook denial — the scrollback annotation names the hook — rather than falling back to the original. A rewrite may change a call's arguments but not which tool runs, so one that retargets a `use_tool` call is blocked too. A hook that exits non-zero keeps its `deny` but loses its `updatedInput` and its `additionalContext`.

A `deny` discards any `updatedInput`; when several hooks return one, the last wins. Omitting `decision` while returning `updatedInput` allows the call and applies the rewrite.

A `defer` neither blocks the call nor approves it: the call takes the normal permission flow, exactly as if your hook had not answered, and a warning naming the hook goes to the log. It also acts on nothing else you sent — an `updatedInput` or `additionalContext` next to a `defer` is ignored and named in the log. Across hooks `defer` ranks below `ask`, so where one of your hooks defers and another asks, grok prompts.

`additionalContext` is a note for the model. It arrives with the results of the batch the call belongs to, wrapped in your harness's reminder tag (`<system-reminder>` by default) and naming the hook that wrote it, so the model can tell your text from the user's. Every hook that sends one is delivered, in the order the hooks ran (unlike `updatedInput`, where the last writer wins). A call a `[permission]` deny rule refuses still delivers it with the batch's results. A hook `deny` drops all of it and names the drop in the log. A call rejected at the approval prompt drops it too. Text over 10,000 characters is clipped, the same ceiling `Stop` feedback carries.

### PostToolUse Output

`PostToolUse` runs after the tool finished, so it blocks nothing. Its stdout is still read, because it decides what the model sees next. Write JSON to **stdout**:

```json
{
  "decision": "block",
  "reason": "The diff still contains a debug print",
  "hookSpecificOutput": {
    "hookEventName": "PostToolUse",
    "additionalContext": "This file is generated; edit the template instead",
    "updatedToolOutput": { "type": "Bash", "command": "…", "exit_code": 0, "output_for_prompt": "[redacted]" }
  }
}
```

| Field | Effect |
|-------|--------|
| `decision: "block"` + `reason` | Delivers `reason` to the model next to the tool result. The tool's own output still arrives; "block" means "tell the model something went wrong", not "stop the call". |
| `additionalContext` | Adds a note for the model next to the tool result. |
| `updatedToolOutput` | Replaces the model's copy of the result. Universal key; works for every tool. |
| `updatedMCPToolOutput` | MCP-only alias for `updatedToolOutput`. Ignored on a built-in tool. |
| `continue: false` + `stopReason` | Ends the turn once the call's hooks have run. The result stands; you see the `stopReason`, and the model doesn't. Queued prompts and wakes then wait for you, as after a `PreToolUse` stop. A subagent's stop ends only its own turn. When an external runtime runs the tools, the turn goes on. |

- **Delivery.** The block reason and `additionalContext` arrive after the tool result, wrapped in your harness's reminder tag and naming the hook that wrote them, so the model can act in the same turn. Every hook's block reason and `additionalContext` are delivered in the order the hooks ran, so one hook's finding cannot drop another's. Only replacements are last-writer-wins: when two hooks return one, the last survives and the drop is named in the log. A replacement the model would read the same way as the result the hook was given is an echo. An echo is the same value, a value that renders to the same text on a built-in tool, or that text as a string through a key the tool takes. An echo changes nothing and displaces no earlier hook's replacement. A result that brings images its text leaves out (an image or PDF read, or a file read or MCP result whose images grok took off before the hook ran, unless a text-only harness drops them) counts only as the same value. Given such a result as text, sending that text back drops its images. That covers a large result and a failure's error text. This holds for every tool, MCP included, and for `PostToolUseFailure`.
- **Building `updatedToolOutput`.** On any tool, a JSON string becomes the model-facing text verbatim. A redaction hook masks the secrets in the text and sends it back as a string. On a built-in tool, any other value must carry grok's own output shape for a tool, a tagged object such as `{"type": "Bash", …}`, which the model reads as that tool renders it. Take the `toolResult` the event handed you, edit it, and send it back. When an external runtime runs the tools, a shell result's `output_for_prompt` is that runtime's JSON tool output, not grok's `exit: N…` summary or the text its model read. An edited result reaches the model as grok renders a `Bash`. Its `truncated` and `total_bytes` may be absent. Its `output_file` may be empty. Its `current_dir` is the directory the runtime reported, else the call's directory. It is empty only when neither is known. When an external runtime runs the tools, a backgrounded shell's `pre_formatted` holds that runtime's text. While `pre_formatted` is set, the model reads it and none of the shell's other fields. A `Bash` replacement may leave out `truncated` and `total_bytes` or send them as `null`. Absent or `null`, they read as `false` and `0`. A structured replacement that is not a grok tool output is ignored and the original stands, but the hook's run is recorded `Failed` with the reason, so an exit-0 hook that shows "failed" is reporting a dropped replacement, not that it never ran. A mistyped `decision` (only `"block"` is honored) is reported the same way.
- **Large results.** The typed `toolResult` is sent while its JSON fits in 128 KiB. A larger one is replaced by the tool's whole model-facing text as a JSON string, and `toolResultTruncated` is `true`. That text is the tool's model-facing text after grok's own per-tool output limits. When an external runtime runs the tools, it is that runtime's text tool output. A redaction hook sees every byte of it. Echo it back as a string. For an image or PDF read, that text leaves out the image. Sending it back drops the image.
- **MCP tools.** An MCP tool has no output shape. Neither `updatedToolOutput` nor `updatedMCPToolOutput` is checked against one. A JSON string becomes the model-facing text verbatim. Any other value is serialized. The last hook to write wins across both keys.
- **Caps.** The block reason and `additionalContext` are clipped at 10,000 characters, the ceiling `Stop` feedback and `PreToolUse` context share. A replacement may be up to 64 K characters longer than the model-facing text it replaces. A redacted copy of a large output is never clipped. Caps are measured on the rendered model-facing text and applied once the replacement has rendered, so a long `updatedToolOutput` is clipped like a string, not dropped for its size. A structured replacement is dropped only when it is not a grok tool output.
- **Broken hook.** A non-zero exit — exit 2 included — keeps the block reason and drops everything else: `additionalContext` and the replacement are dropped and the drop is named in the log, the same rule `PreToolUse` applies to `updatedInput`. The block is the fail-safe direction.
- **Record vs. model.** A replacement rewrites only the model's copy. The scrollback, transcript, and telemetry keep the original, so redacting a secret hides it from the model, not from you, and a failure rewritten as success stays real on the record. Images are not delivered under a replacement, so a replaced screenshot or PDF read reaches the model as your text alone. To drop an image, send a string or set the image's `model_text`. An image result sent back with its data blanked still renders its default caption. That caption says the image is attached. A PDF read drops its pages only through a string. Blanked pages still render as rendered pages. Everything a hook sends (note, block reason, replacement) is escaped so it cannot close the reminder tag and pose as harness or user authored instruction.
- **Output replacement is settings-file only.** Command and HTTP hooks can do all of this. A `PostToolUse` hook registered through the grok-agent-sdk can contribute a `block` reason and `additionalContext`, but cannot replace the tool output.
- **When it fires.** `PostToolUse` fires for every tool that actually ran, including one whose result is a built-in logical error such as a non-zero `run_terminal_command` exit. A tool that failed to dispatch, or an MCP tool that returned an error result, fires `PostToolUseFailure` instead. That hook can feed the model `additionalContext`, replace the error text, and end the turn with `continue: false`, but it cannot block. The event's `error` field is the text a replacement stands in for. Either `updatedToolOutput` or `updatedMCPToolOutput` replaces it: a JSON string verbatim, any other value serialized. The last hook to send one wins. A dispatch failure still reaches the model as ``Tool `<name>` failed: <your text>``. The same caps, escaping, and record-vs-model rules apply. The hook inherits the 600-second gate default (it commonly runs a linter or test); set `timeout` explicitly only when the check needs longer or shorter. A timed-out hook is recorded as a failure and contributes nothing.

### Exit Codes

| Exit Code | Meaning |
|-----------|---------|
| `0` | Success / allow (for blocking hooks) |
| `2` | Explicit deny (`PreToolUse`, unless its stdout JSON says `continue: false`, which stops the agent), block-stop with stderr as feedback (`Stop`/`SubagentStop`, when stdout holds no readable reply), or feedback to the model (`PostToolUse`). For `PreToolUse`, the first stderr line (capped) becomes the deny reason when the JSON carries none; `Stop`/`SubagentStop` and `PostToolUse` feed the full stderr to the model, and a JSON `reason` wins over it. |
| Other | Fail-open — the failure is recorded (as `exit code N: <first stderr line>`) but nothing is blocked. A `PreToolUse` or `UserPromptSubmit` reply past a read limit denies or blocks (see [Reading a Hook's Reply](#reading-a-hooks-reply)). For `PreToolUse`, a `deny` decision or `continue: false` in stdout JSON is honored regardless of exit code. For `Stop`/`SubagentStop`, a readable reply on stdout decides at any exit code, exit 2 included. One that sets no block lets the turn end without feeding stderr back. The exit code decides only when stdout holds no readable reply. Then exit 2 blocks with stderr as the feedback. For `PostToolUse`, the tool has already run so nothing is blocked either way; the failure is still recorded, and the hook keeps its block reason and its `continue: false` but loses its `additionalContext` and its output replacement. |

**`PostToolUse` exit 2 is a behavior change.** It used to be an ordinary recorded failure that changed nothing; it now feeds the hook's stderr to the model. A logging hook written as `run_checker; exit $?` therefore hands the model whatever the checker printed whenever the checker exits 2 — `mypy`, `grep`, `pytest` and `argparse` all use exit 2 for "no match" or "bad usage", and on Windows `robocopy` exits 2 when it finds extra files and `net` on any error. End such a hook with an explicit `exit 0` to keep it silent.

Write human-readable diagnostics to **stderr**: it is the hook's feedback channel. On failures the first stderr line appears in the scrollback entry and logs instead of a bare exit code.

#### PowerShell on Windows

Under pwsh or Windows PowerShell, Grok adds exit lines after an inline command so that it exits as it would under `sh -c` where it can:

- A command whose last command is a program exits with that program's code, so a program such as `node guard.js` that exits 2 still blocks.
- When the last command is a PowerShell command that fails, the hook exits with the last program's exit code if that was not 0, else 1. This differs from `sh -c` and errs toward blocking. With no `stale.lock`, `grep -q x missing.txt; Remove-Item stale.lock` exits 2, where `grep -q x missing.txt; rm stale.lock` under `sh` exits 1. It denies a `PreToolUse`, blocks a `Stop` and feeds a `PostToolUse` hook's stderr to the model.
- A terminating error ends the hook with exit 1, whatever ran before it. That is a `throw`, and any error under `$ErrorActionPreference = 'Stop'` or `-ErrorAction Stop`, so `$ErrorActionPreference = 'Stop'; node guard.js; Remove-Item stale.lock` doesn't block when there is no `stale.lock`.
- Under `$PSNativeCommandUseErrorActionPreference = $true` and `$ErrorActionPreference = 'Stop'`, a program whose exit stops the hook keeps its exit code too, as under `set -e`. Under pwsh, Grok adds this trap after the command for that:

  ```powershell
  trap { if ($_.FullyQualifiedErrorId -eq 'ProgramExitedWithNonZeroCode') { exit $_.Exception.ExitCode }; break }
  ```

  It adds the trap only when all of the following hold. Where code runs under another preference, the trap would end the hook at an error the hook would otherwise go on past, such as one before its `exit 2`.

  - The command opens with those two settings, in either order and with the literal values `$true` and `'Stop'` (or `"Stop"`).
  - Only `Set-StrictMode -Off`, or `Set-StrictMode -Version` with `Latest`, `1` to `5` (with or without `.0`) or `5.1`, comes before them, in one piece on one line. A comment may end that line, but only blank lines come before it, not even a comment or `#requires -Version 7.3`. Every PowerShell accepts those versions. A version some PowerShell rejects, such as `3.5` or `7.4`, would be an error the trap ends the hook at.
  - Only blanks and a comment come between a setting and its `;` or line break.
  - The next line after the settings, past blank and comment lines, doesn't start with `|`, which would continue the second setting.
  - Nothing after them holds `errora`, as in `-ErrorAction`, or the word `ea`, even in a path such as `C:\Users\ea`. Nor does it mention `trap`, `iex`, `Invoke-Expression`, `scriptblock`, `InvokeScript`, `Import-Module`, `ipmo`, `#requires`, which can load a module, or a `.ps1`, `.psm1` or `.psd1` file. That includes `pwsh -File guard.ps1`, although a child process can't change the preference.
  - No value in the `env` of the hook or of its plugin names a `.ps1`, `.psm1` or `.psd1` file. Other values may mention anything.

  A script or module the command reaches without naming its file can still set its own preference, and the trap can then cut it short at an error it would go on past. Examples are `.\guard` running `guard.ps1`, a module PowerShell loads on its own, or a `$env:GUARD` the hook inherits rather than sets in `env`.

  Otherwise, a program that stops the hook ends it with exit 1; wrap the program in `try { ... } catch { exit $LASTEXITCODE }` to keep its code. The setting is standard from pwsh 7.4 and experimental in 7.3. Windows PowerShell 5.1 doesn't have it.
- A command made of `begin`, `process`, `end`, `clean` (pwsh 7.3 and later) or `dynamicparam` blocks runs as written, since PowerShell allows nothing after them, and so does a `.ps1` hook. Neither keeps a program's exit 2 on its own. Put `exit $LASTEXITCODE` inside the last block, because an `exit` after the blocks fails to parse. Under `Set-StrictMode`, reading `$LASTEXITCODE` before any program has set it is an error, which becomes the first stderr line. Use `exit (Get-Variable LASTEXITCODE -ValueOnly -ErrorAction Ignore)` there, as the exit lines do. Under the two settings above, such a command or `.ps1` hook also needs its own copy of the trap above, or a `try`/`catch`, to keep the code of a program that stops it.
- Grok tells such a command by how it starts, as PowerShell does. It passes over `using` statements, attributes such as `[CmdletBinding()]` and a `param(...)` block, whose brackets it matches past strings and comments. Blanks, line breaks, comments (`#requires` included) and backtick line continuations may come before and between those parts. The command runs as written only when one of those words comes next, in any case, and then a `{`. Any other command gets the exit lines, so `[Console]::Error.WriteLine('end { }'); node guard.js` keeps `node`'s exit 2. As in PowerShell, a comment or backtick glued to a keyword makes it part of a longer word, so `end<# c #>{ ... }` gets the exit lines too, while `end <# c #> { ... }` runs as written.
- A bracket, quote or comment in that opening that never closes also gets the exit lines. PowerShell can't parse such a command either way.
- Windows PowerShell 5.1 runs a command's `process` block once for each line it pipes in from stdin, and it pipes stdin in only when the command reads its own `$input`, as a splatted `@input` also does, so without it the block would never run. An `$input` in a function or script block the command defines, such as `ForEach-Object { $input }`, reads that one's own input and doesn't count. There, Grok runs such a command inside `& { ... }`, after its `using` statements, so the block runs once, as under pwsh, and can read the hook's input with `[Console]::In.ReadToEnd()`. 5.1 runs a `.ps1` hook's `process` block the same way under `-File`, so put such a hook's work in an `end` block.
- An advanced script, one with `[CmdletBinding()]` or a `[Parameter()]` parameter, gets nothing in `$input` unless one of its parameters has `ValueFromPipeline`. PowerShell can't bind the hook's input to it, so `$input` stays empty and the script goes on to exit 0. Grok still runs such a command that reads its own `$input`, since a deny it makes without the input still blocks. It adds a warning to the hook's `systemMessage`, after any of its own, also when the hook times out or its output can't be read, so wherever the event shows one, a guard that allows for want of its input doesn't pass unseen. A JSON deny keeps its reason, while an `exit 2` blocks with PowerShell's binding error as its reason, the first line on stderr. A deny in a `process` block never runs, since PowerShell skips that block for input it can't bind, so put it in an `end` block. Read the input with `[Console]::In.ReadToEnd()`, in a `.ps1` hook too, which Grok runs without this check.
- Where the text leaves unclear whether an `$input` is the command's own, Grok doesn't run the command inside `& { ... }` under Windows PowerShell 5.1. That's an `$input` in a `{` that starts a `switch` clause, in the value of a class's property, or after a command whose name pwsh may read as a number, such as `1u#` or `0b1#`. Under 5.1, such a command's `process` block never runs. In an advanced script, Grok shows the warning above for it as well. Read the input with `[Console]::In.ReadToEnd()` there too.

### Reading a Hook's Reply

Grok reads a reply that starts with `{` the way JavaScript's `JSON.parse` does.
A leading byte order mark is dropped.
The last of two duplicate keys wins.
A lone UTF-16 surrogate escape becomes U+FFFD.
A number too large for a double becomes the largest one.
Anything else is plain text.

- **Not valid JSON.** For an event that reads its hook's reply, a reply that starts with `{` but is not valid JSON fails the run at exit 0.
  So does one that does not match the event's fields (`"decision": 5`).
  The failure is shown.
  The hook fails open.
  At any other exit code the exit code decides.
  An HTTP hook's run fails whatever its status.
  Observe-only events, such as `SessionStart`, `Notification` and `PreCompact`, read only a top-level `systemMessage` from the reply.
  A `StopFailure`, `PreCompact`, `PostCompact` or `SessionEnd` hook's `systemMessage` is dropped, as in Claude Code.
- **Read limits.** A reply is read up to 8 MiB, building at most 524,288 values and keys, at any depth.
  A reply longer than 8 MiB is still read when only white space follows.
  When other text follows, it is past the limit only if its first 8 MiB could still begin a longer document.
  Otherwise it is plain text or JSON that is not valid.
- **Past a limit.** Grok cannot tell what a reply past a limit decided.
  A `PreToolUse` hook then denies the tool call, whatever an HTTP hook's status.
  A `UserPromptSubmit` hook blocks the prompt.
  An HTTP `UserPromptSubmit` hook's body counts only on a 2xx status.
  The reason is `hook reply was not read: <limit>`.
  A command hook that exits 2 is decided by exit 2 as usual.
  `Stop`, `SubagentStop`, `PostToolUse` and `PostToolUseFailure` treat it like a reply that is not valid JSON.
  The run fails at exit 0 and fails open.
  At any other exit code the exit code decides.
  Observe-only events get no `systemMessage` from it.
- **Nesting in values Grok passes on.** `updatedInput`, `updatedToolOutput` and `updatedMCPToolOutput` are passed on whole down to 124 levels.
  At that depth a whole session update or ACP message carrying one stays within what an ACP client such as the pager decodes.
  A deeper `updatedInput` on an allow, an ask or a reply with no decision, from a hook that succeeded (exit 0, or a 2xx status), denies the tool call with ``hook reply was not read: `updatedInput` nests more than 124 levels deep``.
  From a hook that failed (an exit code other than 0 or 2, or a non-2xx status), the rewrite is dropped and the call goes ahead.
  A deeper output replacement from a hook that succeeded withholds the tool's output.
  The model reads ``Tool output withheld: `updatedToolOutput` nests more than 124 levels deep`` (or `updatedMCPToolOutput`) in its place.
  The run is recorded as failed.
  Its block reason and `additionalContext` still apply.

### Stop Decision Control

`Stop` and `SubagentStop` hooks run when the agent is about to finish its turn and can keep it working (Claude Code-compatible). Write JSON to **stdout**:

- **Block the stop**: `{"decision": "block", "reason": "The test suite hasn't been run yet"}`. The reason is fed back to the model as a user message and the agent runs another round in the same turn.
- **Non-error feedback**: `{"hookSpecificOutput": {"hookEventName": "Stop", "additionalContext": "Run the linter before finishing"}}`. Also keeps the agent working, but is surfaced as hook feedback rather than a hook error.
- **Force stop**: `{"continue": false, "stopReason": "Budget exhausted"}`. Ends the turn, overriding any blocks.
- **Allow the stop**: exit 0 with no output (or any non-JSON output).

Exiting with code `2` also blocks the stop, with **stderr** as the feedback, when stdout holds no reply Grok can read.
A readable reply decides instead, even one that sets no `decision`.

The hook input includes `stopHookActive` and `lastAssistantMessage`. `stopHookActive` is true when the agent is already continuing due to a previous stop-hook block this turn; check it, or the transcript, to avoid blocking on a condition that will never resolve. `lastAssistantMessage` carries the text of the agent's final response this turn, so hooks can act on it without parsing the transcript. Every event carrying this field clips it at 32,768 characters, with the same `… [+N chars]` marker as the other free-text fields. It is far looser than the 1,000 applied to `errorDetails` and friends because it carries a whole answer rather than a label, and it is sized to the same scale as the tool payload cap. After **8 continuations** (blocks or non-error feedback) in one turn the gate is overridden and the turn ends; hooks are not consulted for that final, forced stop. The counter is per turn: the next user prompt starts fresh, so a long-running goal can span turns. Hook failures fail open: the agent stops normally.

`Stop`, `SubagentStop`, and `PostToolUse` hooks default to a 600-second timeout because these gates commonly run builds or test suites, and a timed-out hook fails open, so the check does not block anyway. Every other event keeps the 5-second default. Set `timeout` explicitly when a gate needs more: `{ "type": "command", "command": "bin/verify.sh", "timeout": 1200 }`.

The gate runs only for genuine completions. A turn that was interrupted (Ctrl+C), refused, or cut off at the turn limit skips the Stop gate, though a Ctrl+C that lands while a Stop hook is already running kills it mid-flight (see below); API-error turns fire `StopFailure`, and cancelled turns fire `StopCancelled`. `Esc` never cancels a running turn. A separate Stop also fires at session end (`reason: "channel_closed"` or `"shutdown"`). It runs at the same time as the `SessionEnd` hooks, for at most 1.5s. Its decision output is parsed but ignored, since there is no turn left to continue. A script that counts or gates on Stop fires should check `reason == "end_turn"` so the session-end fire doesn't skew it.

`StopFailure` is observation-only (use it to log failures or send alerts; output and exit code are ignored). Its input carries `error` (the classified type the matcher tests: `rate_limit`, `authentication_failed`, `invalid_request`, `server_error`, `max_output_tokens`, or `unknown` for anything the runtime cannot distinguish; capacity errors classify as `rate_limit`), `errorDetails` (the raw error detail, when available, clipped at 1000 characters; absent for a refusal, whose explanation rides `lastAssistantMessage` alone), `lastAssistantMessage` (the rendered error text shown in the conversation; for this event it is the error string, not assistant output), and `subagentType` (the subagent's type when the turn ran inside one).

`StopCancelled` is observation-only too. **It runs instead of `Stop` when the turn ends without completing**, the same way `StopFailure` runs instead of `Stop` on an API error.

A turn reports **at most one** of the three, with one exception noted below: a `Stop` hook that ran to completion can still be followed by `StopCancelled` if the user interrupts during the gate, because by then the hook has already been told the turn ended. Every turn that runs the model and then ends, errors, or is cancelled reports one, except for the cases listed below.

If your host must never miss an idle transition, listen for the `idle_prompt` `Notification` as well. It covers every exception in which the session is still alive, with one gap: a session whose only activity was a bash-mode command that ran to completion earns neither the report nor the ping, though interrupting one earns both. `SessionEnd` covers teardown. The `idle_prompt` ping fires about a minute after the session settles, needs at least one turn to have ended, and is cancelled if you send another message first.

A cancelled turn's report is dispatched off the session's command loop, so an interrupt is never delayed by your hook. The report can therefore arrive **after** the next turn's `UserPromptSubmit`, and turn-end reports are not ordered against each other across paths.

A script that tracks busy and idle should key on `promptId`, following the rules below. grok mints one per turn, but a client that supplies its own in `_meta` owns its uniqueness, so treat the id as opaque and scope it to the session.

Every turn-end report goes through one worker, so a slow hook delays the next report but never the turn it belongs to. Keep observe-hook timeouts short.

An interrupt while a `Stop` hook is running kills that hook mid-flight, and the turn then reports `StopCancelled`: a `Stop` hook that started is not a promise the turn completed. A `StopFailure` hook runs off the turn, so an interrupt cannot kill it, and that turn has already reported, so no `StopCancelled` follows.

`Stop` is a gate, so when a stop hook blocks it fires again for each continuation round; only the fire that lets the turn end is the report, and a turn that ends cancelled or failed after a blocked `Stop` reports that instead. A passive observer cannot tell a continuation fire from the final one (`stopHookActive` is true for both), so a UI gated on `Stop` alone shows a false idle from the first continuation fire until the user's next prompt, since no `UserPromptSubmit` marks a continuation round. Leave `Stop` out of the state script when you also run a blocking gate, and settle on the `idle_prompt` `Notification` instead.

Some turns report none of the three:

- a bash-mode (`!`) command that runs to completion. It raises no hook at all: no `UserPromptSubmit`, `PreToolUse`, `PostToolUse` or turn-end `Stop`, so a `PreToolUse` hook cannot deny it. Interrupting one still reports `user_interrupt`, without a preceding `UserPromptSubmit`.
- a builtin slash command that runs to completion. Interrupting one still reports `user_interrupt`, without a preceding `UserPromptSubmit`.
- a cancel-and-send, a rewind, or a queued prompt removed before it ran.
- session teardown, reported by the session-end `Stop` and `SessionEnd`.
- a turn whose stop hooks kept the agent working until the per-turn continuation limit forced the stop.
- a turn where no stop hook ran to completion, because they were all disabled, untrusted, or failed, or, in a subagent, because their matchers all missed. Leaving the turn unreported is deliberate: a later cancel or failure can then report it.
- a turn superseded by the next one while its report was still being built.
- a report still queued when the session exits: teardown waits half a second for queued turn-end hooks, then drops what is left and aborts any hook still running.
- a turn that completed, ran its `Stop`, and only then failed to write to disk: it reported `Stop`, so no `StopFailure` follows. The failure still surfaces in the conversation.

`StopCancelled`'s input carries:

- `reason`: the classified cause, and the value the matcher tests. `user_interrupt` (Ctrl+C, a client stop button, or a client `session/cancel`), `permission_rejected` (you declined a tool call), `permission_cancelled` (you dismissed the prompt), `max_turns`, `no_progress` (the agent bailed out after repeated no-op rounds), `hook_stopped` (a `PreToolUse`, `PostToolUse` or `PostToolUseFailure` hook's `continue: false`), or `unknown` (a cancel the runtime could not classify, and the forward-compatible fallback). The matcher tests this field only, so a hook that wants every user-initiated stop matches the reasons it cares about and reads `cancelledBy` from the payload. New reasons may be added over time, so treat an unrecognized value the way you treat `unknown`.
- `cancelledBy`: `user` for an interrupt, a declined tool call, or a dismissed prompt; `runtime` for everything the agent decided itself, such as `max_turns`, `no_progress` and `hook_stopped`; `unknown` when `reason` is `unknown`, because a cancel the runtime could not classify cannot claim the user was uninvolved. Derived from `reason`, so a new reason classifies automatically. Values may be added here too: treat one you do not recognize the way you treat `unknown`, rather than assuming anything that is not `user` was the runtime.
- `cancelTrigger`: the gesture, when the client named one, clipped at 64 characters, since a gesture name is a token. The bundled pager sends one of three: `ctrl_c`, `mouse` (the on-screen stop button), or `dashboard_stop`. It never sends `esc` (Esc does not cancel a turn). Another client may send any string, including `esc`, and it is passed through verbatim. Every value here classifies as `user_interrupt`, including one that happens to spell an internal name such as `shutdown`, because a client asking to cancel is the user asking; read `cancelledBy` from the payload rather than parsing this string. Omitted for a bare `session/cancel` and for every runtime-initiated reason.
- `reasonDetails`: the same kind of detail `StopFailure` puts in `errorDetails`, when the runtime has one. For a declined tool call it is `<tool>: <why>`. Clipped at 1000 characters, like `StopFailure`'s `errorDetails`.
- `lastAssistantMessage`: whatever the turn had committed to the conversation at the interrupt, if any. A Ctrl+C during the final answer leaves the last committed text, or nothing if the turn never committed any. Clipped like the same field on `Stop` and `StopFailure`.
- `subagentType`: the subagent's type when the turn ran inside one, so a hook can tell a nested agent's stop from the session's. Absent in the main session.

The envelope's `timestamp` is stamped when the hook dispatches, not when the turn ended. The three turn-end events queue behind one worker, so a report waiting on a slow hook ahead of it carries a later timestamp than the moment it describes. Use `promptId` to correlate, not the clock.

`StopCancelled` cannot block: the turn is already over, and letting a hook reopen a turn the user deliberately stopped would fight the user. Use `Stop` when you want to keep the agent working.

A "cancel-and-send" (typing a new message while a turn runs) does **not** fire `StopCancelled`, because the turn is being replaced, not stopped, and the agent stays busy. Inside a subagent, a `user_interrupt` does not fire either: it follows the parent's cancel, and the session-level signal is the useful one. A subagent's own `max_turns`, `no_progress`, hook stop, or declined permission does fire. The matcher tests `reason` only, so a script that reports whether the session is idle should exit early when `subagentType` is present.

A complete busy and idle indicator takes five registrations. `UserPromptSubmit` marks the session busy;
`Stop`, `StopFailure`, and `StopCancelled` settle it however the turn ended; the `idle_prompt`
`Notification` is the backstop for the turns that report none of the three. Registering only
`StopCancelled` leaves the host busy after every normal turn.

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "bin/turn-started.sh" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "bin/turn-ended.sh", "timeout": 10 }] }],
    "StopFailure": [{ "hooks": [{ "type": "command", "command": "bin/turn-ended.sh" }] }],
    "StopCancelled": [{ "hooks": [{ "type": "command", "command": "bin/turn-ended.sh" }] }],
    "Notification": [
      { "matcher": "idle_prompt", "hooks": [{ "type": "command", "command": "bin/turn-ended.sh" }] }
    ]
  }
}
```

What the two scripts have to get right:

- **Track the newest `promptId` and ignore reports for older turns.** A cancelled turn's report is
  dispatched off the command loop, so it can arrive after the next turn's `UserPromptSubmit`.
- **Settle unconditionally when there is no `promptId`.** That is grok reporting on the session
  rather than a turn: the `idle_prompt` ping and the session-end `Stop`. It is what makes the
  backstop work for a rewind or a superseded turn, which report nothing.
- **Treat a `promptId` you never saw start as idle.** An interrupted bash-mode turn reports without
  a preceding `UserPromptSubmit`.
- **Exit early when `subagentType` is present.** A subagent's stop is not the session's.
- **Settle the host before you record the turn as handled,** so a hook killed mid-flight leaves the
  turn correctable. Re-read that record first, so you only clear a turn you recorded yourself.
- **Keep it to a local write.** Teardown gives the whole queue of turn-end reports half a second,
  then queued `Notification` hooks 1.5s, and each `SessionEnd` hook is then bounded by its own
  timeout (default 1.5s; set `GROK_SESSION_END_HOOKS_TIMEOUT_MS`, in milliseconds, to change that
  default, capped at 60s). A plugin's `SessionEnd` hook runs for at most that default, whatever its
  `timeout`. Exit waits at most 10s for `SessionEnd` hooks on the shell route. On the daemon route
  it runs a session's `SessionEnd` hooks one after another, each up to its own timeout, and waits
  at most 60s in all.

`Stop` is a gate, so that entry runs on the turn's critical path: keep it fast, give it a `timeout`,
and exit 0, because exit 2 blocks the stop and keeps the agent working. Leave `Stop` out if you also
run a blocking `Stop` gate, since a continuation fire would settle the host while the agent is still
going; register `SessionEnd` instead, which is the only thing that settles a session that exits
before the ping.

Both scripts also run inside a subagent's own session. Every event either script reads carries
`subagentType` there and omits it in the main session, so `[ -n "$subagentType" ] && exit 0`
filters a child out of both halves. This matters most for a background subagent, which outlives
the parent turn and would otherwise hold the host busy after the parent went idle.

`Stop` input also carries `backgroundTasks` and `sessionCrons`, so a hook can distinguish "session is done" from "session is paused waiting for background work to wake it back up". Both arrays are empty when nothing is in flight or scheduled. Each `backgroundTasks` entry describes one in-flight task: `id`, `type` (`shell`, `monitor`, or `subagent`), `status`, and (depending on the type) `command` (shell tasks only), `description` (a monitor's watched command line, or a subagent's task description), and `agentType` (subagents). Each `sessionCrons` entry describes one scheduled wakeup (`scheduler_create` or `/loop`): `id`, `schedule`, `recurring`, and `prompt`. The `schedule` value is a human-readable interval such as `every 5 minutes`; grok schedules are intervals, not cron expressions. Free-text entry fields are capped at 1000 characters with an in-string `… [+N chars]` marker.

Inside a subagent, the gate fires as `SubagentStop` (agent-frontmatter `Stop` hooks are automatically remapped). A `Stop` hook only gates the main agent.

`SubagentStop` fires once per subagent, at the subagent's own turn end, matching Claude Code. Its input carries a `phase` field (currently always `"gate"`) reserved for forward compatibility.

**Porting Claude Code stop hooks**: the output vocabulary (`decision`, `reason`, `continue`, `stopReason`, `additionalContext`) works unchanged. Check this list for what does not match Claude:

- **camelCase input with snake_case aliases**: grok's stdin envelope uses camelCase keys, and also sends Claude's snake_case spelling of these top-level keys: `session_id`, `transcript_path`, `permission_mode`, `tool_name`, `tool_input`, `tool_response`, `tool_use_id`, `duration_ms`, `is_interrupt`, `stop_hook_active`, `last_assistant_message`, `error_details`, and `notification_type`, so a script reading `.stop_hook_active` works unchanged (the `hook_event_name` snake_case key carries Claude's PascalCase value, e.g. `"Stop"`; the `hookEventName` camelCase key carries grok's snake_case value, e.g. `"stop"`). Keys nested in `backgroundTasks` and `sessionCrons` entries stay camelCase only, so a script reading `.background_tasks[].agent_type` must switch to `.backgroundTasks[].agentType`. Hooks registered through the grok-agent-sdk convert both the top-level keys and the `backgroundTasks`/`sessionCrons` entry keys to snake_case, so the wire's `.backgroundTasks[].agentType` reads as `.background_tasks[].agent_type` in the SDK.
- **`toolResult` field**: the `PostToolUse` tool output is `toolResult` (SDK: `tool_result`); grok also emits a `tool_response` snake alias that copies `toolResult`, so a hook reading Claude's `.tool_response` works unchanged.
- **`updatedToolOutput` on built-in tools is text or grok's own output shape**: a `PostToolUse` replacement for a built-in tool is a string, read as written, or a tool output as grok serializes it — the tagged object in that event's `toolResult` — so an object written against another runtime's field names does not parse and is ignored. On an MCP tool there is no shape to enforce, so `updatedToolOutput` passes through like its `updatedMCPToolOutput` alias. See [PostToolUse Output](#posttooluse-output).
- **Session-end fire**: an extra observe-only Stop fires at session end; filter on `reason == "end_turn"` (see above).
- **Interval schedules**: `sessionCrons[].schedule` is a human-readable interval, never a cron expression.
- **Task types**: `backgroundTasks[].type` is only `shell`, `monitor`, or `subagent`; Claude's other labels (`workflow`, `teammate`, …) are not emitted.
- **StopFailure classes**: grok emits six (`rate_limit`, `authentication_failed`, `invalid_request`, `server_error`, `max_output_tokens`, `unknown`). Capacity errors (503/529) classify as `rate_limit`. A matcher on an error class grok does not emit never fires.
- **Default timeout**: grok defaults observe hooks to 5 seconds, which is shorter than most. Set `timeout` explicitly on an imported hook that does real work.
  `/import-claude` writes a fractional `timeout` rounded up to whole seconds (`1.5` becomes `2`).
  Grok builds released before fractional timeouts read only whole seconds.
- **Exit 2 with a JSON reply on a stop hook**: Claude Code blocks at exit 2 whatever `decision` says.
  A `decision: "block"` reason replaces stderr as the feedback.
  Grok reads the reply first.
  A readable reply decides, even one that sets no `decision`.
  A hook that prints `{"systemMessage": "..."}` and exits 2 lets the turn end without feeding stderr back.
- **`UserPromptSubmit` blocks, with one gap**: exit 2 and `decision: "block"` reject the prompt like Claude, and a blocked prompt never enters the conversation history — but an allowing hook's stdout / `additionalContext` is discarded rather than added as context.
- **`StopCancelled` is grok-specific**: a config that uses it is not portable to a runtime with no interrupt hook.
- **`idle_prompt` fires on any turn end**: grok fires it after an interrupted or errored turn too, not only a completed one, because it reports a state rather than an outcome. Its `message` is display text and can change between releases, so match on `notificationType` instead.
- **Subagent identity is `subagentType`, not `agent_type`**: grok puts it in the payload of the events that can fire inside a subagent, matching its own `SubagentStart`/`SubagentStop`, rather than in the common fields.
- **permission_mode values**: grok emits `default`, `auto`, `plan`, or `bypassPermissions`. Claude's `acceptEdits`/`dontAsk` have no grok equivalent (grok's `auto` is the nearest), so a check like `permission_mode === "acceptEdits"` never matches.
- **Client (SDK) gate timeouts**: SDK `Stop`/`SubagentStop` gates default to 600 seconds like file hooks; `PreToolUse` client gates default to 30 seconds (the interactive hot path). Either can be overridden per matcher group via `timeoutS`, capped at 600.
- **`/goal`**: grok's goal loop is a separate feature that runs before the stop gate; it is not a prompt-type Stop hook.

A complete keep-working policy in one script:

```bash
#!/bin/bash
input=$(cat)
# Gate only genuine turn ends, not the session-end observe fire.
if [ "$(echo "$input" | jq -r '.reason')" != "end_turn" ]; then exit 0; fi
if ! bin/verify.sh >/dev/null 2>&1; then
  echo '{"decision": "block", "reason": "verify.sh failed; fix the failures before finishing"}'
fi
```

registered as `{ "type": "command", "command": "bin/stop-gate.sh", "timeout": 300 }` with `timeout` sized for the verify step. The hook fires again after each continuation, and the built-in cap ends the turn after 8; check `stopHookActive` to give up earlier on feedback the agent evidently cannot act on.

### Passive Hooks

For events like `SessionStart` or `Notification`, only a top-level `systemMessage` in stdout is read.
Just exit 0 on success. The exceptions are `PreToolUse` (see [Output (Blocking Hooks)](#output-blocking-hooks)), `Stop`/`SubagentStop` (see [Stop Decision Control](#stop-decision-control)), and `PostToolUse`, whose stdout is read even though it blocks nothing (see [PostToolUse Output](#posttooluse-output)).

On the shell route, one queue in each session runs the tool permission prompt's `permission_prompt` chain and the `idle_prompt` and `task_complete` `Notification` chains. Each chain starts when its notification is raised, while earlier chains still run, up to 32 chains at once in each session. A chain raised past that starts when one of them ends. Plan-approval and diff-review prompts also raise `permission_prompt`, but they run their chains inline, outside the queue and its exit wait. A slow chain does not delay the chains raised after it until 32 are running. The next command or turn can start while `task_complete` hooks run. At exit the queue's chains get 1.5s in total, before `SessionEnd`, and a chain still running after that is stopped. A first hook slower than 1.5s cuts off the rest of its chain, although each hook's timeout defaults to 5s. This wait can start `SessionEnd` up to 1.5s later.

On the daemon route, `Notification` hooks share one pool with the other observe hooks of every session that `grok host-server` runs. Up to 32 dispatches in that pool run at once and 64 more wait for a place. A dispatch raised past those 64 runs none of its hooks, and a note names the hooks it skipped.

### Environment Variables

Grok sets several environment variables on every hook process. These are useful when writing context-aware or plugin-aware hook scripts.

#### Runner-injected variables (always available)

These variables are set by the hook runner for **every** hook:

| Variable              | Description |
|-----------------------|-------------|
| `GROK_HOOK_EVENT`     | The name of the event that triggered the hook (e.g. `pre_tool_use`, `session_start`, `post_tool_use`, `session_end`, `stop`, `notification`). |
| `GROK_HOOK_NAME`      | The configured name of this specific hook (includes the plugin prefix for plugin-provided hooks). |
| `GROK_SESSION_ID`     | The unique identifier of the current Grok session. |
| `GROK_WORKSPACE_ROOT` | Absolute path to the root of the current workspace. It keeps the spelling of the event's `cwd`, with symlinks unresolved and no trailing slash. |
| `CLAUDE_PROJECT_DIR`  | The workspace root, the directory every hook runs in, with symlinks resolved when the hooks loaded. `GROK_WORKSPACE_ROOT` and the event's `cwd` keep the launch spelling. This one doesn't follow a link that changes after the hooks load. Set for every hook. |

These variables are **reserved**. Any values you attempt to set for them via the `env` field in your hook JSON are stripped at load time (a warning is logged), and the runner always injects the real values at spawn time.

#### Plugin hook variables

When a hook originates from a plugin, Grok additionally injects the following variables:

| Variable             | Description |
|----------------------|-------------|
| `GROK_PLUGIN_ROOT`   | Absolute path to the plugin's installed directory. |
| `GROK_PLUGIN_DATA`   | Absolute path to the plugin's writable data directory (for storing plugin state, caches, etc.). |

These values are provided by the plugin system. For the four plugin-related keys (`GROK_PLUGIN_ROOT`, `GROK_PLUGIN_DATA`, and their Claude aliases), the plugin adapter ensures the official plugin values always win over any user-declared values in the hook's `env` map.

#### User-defined environment variables

You can supply additional environment variables for an individual hook handler using the `env` field:

```json
{
  "type": "command",
  "command": "bin/my-hook.sh",
  "env": {
    "MY_SECRET": "value",
    "LOG_LEVEL": "debug"
  }
}
```

These variables are passed through to the hook process, but they cannot override the reserved runner or plugin variables listed above.

#### Using variables in `command` and `url` fields

Both `command` and `url` support `${VAR}` and `$VAR` expansion. On Windows PowerShell, known `$VAR` refs are rewritten to `$env:VAR` so they read the child environment. See the custom-hooks reference for load-time vs runtime expansion, the `env` map lookup order, and parameter-expansion modifiers (e.g. `${VAR:-default}`).

In a `url`, a reference to a Grok environment variable that is unset or empty stays as written. If the variable is still empty when the hook runs, Grok doesn't send the hook, because an empty host part would send the request to the host named in the path. The hook's run error names the variable.

---

## HTTP Hooks

Instead of a local script, call a remote endpoint:

```json
{ "type": "http", "url": "https://hooks.example.com/grok-event", "timeout": 15 }
```

The full event envelope is POSTed as JSON.

The URL must use `https://`, or plain `http://` to a loopback host (`127.0.0.0/8`, `::1` or `localhost`). A local approval service can listen on `http://127.0.0.1:<port>`. Under either scheme Grok refuses the same non-public ranges as `web_fetch`, other than loopback: `0.0.0.0/8`, `10.0.0.0/8`, `100.64.0.0/10`, `169.254.0.0/16`, `172.16.0.0/12`, `192.0.0.0/24`, `192.0.2.0/24`, `192.168.0.0/16`, `198.18.0.0/15`, `198.51.100.0/24`, `203.0.113.0/24`, `224.0.0.0/4`, `240.0.0.0/4`, `::`, `fc00::/7`, `fe80::/10`, `ff00::/8`, and the IPv4-mapped forms of the IPv4 ranges.
On a fake-ip proxy setup (Clash or Surge `fake-ip` mode), a host name resolves into `198.18.0.0/15`. Every request to it is refused. That holds with `HTTPS_PROXY` set too. Grok still resolves the host itself before the request. A managed-policy hook there fails open on every call.

Grok checks the URL when it loads the hook. A refused URL is a hook load error for that handler, reported like the errors under [Unsupported Handler Fields](#unsupported-handler-fields). The other handlers in the file still load. A managed-policy handler with a refused URL is reported the same way and still loads. The check before each request refuses it. On the shell route, a refused URL in plugin or agent-definition hooks is only logged. When at least one of the main agent's inline hooks loads, the shell route also only logs other hook load errors, until hooks are reloaded. On Linux, under a profile whose effective `restrict_network` is true (`read-only`, `strict`, or a custom profile that sets or inherits it), Grok refuses plain `http://` outside managed policy. The profile keeps hook commands off the network. An HTTP hook request leaves from Grok's own process. A managed-policy hook keeps plain `http://` to a loopback host. Refusing it would make the hook fail open on every call. When anything before the path still names a variable that loading cannot resolve, the check waits for the request. Before each request Grok expands the URL, checks it again, and resolves the host name. No address may be in a refused range. For plain `http://`, every address must be loopback. A request refused there is a hook error. The hook fails open. Without a proxy, the request connects only to the addresses Grok checked. A proxy resolves a remote host itself. A request to a loopback host, under either scheme, goes straight to it and never through `HTTP_PROXY`, `HTTPS_PROXY` or `ALL_PROXY`. Grok does not follow redirects.

---

## Managing Hooks in the TUI

### The Hooks Tab

Press `Ctrl+L` on non–VS Code family terminals to open the Extensions modal (Plugins tab), or run `/hooks` (any terminal; required on VS Code family where `Ctrl+L` is interject) to open it on the Hooks tab. In the **Hooks** tab:

| Key | Action |
|-----|--------|
| `r` | Reload all hooks from disk |
| `a` | Add a custom hook by path |
| `x` | Remove the selected hook source (asks for confirmation; press lowercase `y` to confirm) |
| `Space` | Enable or disable the selected hook |
| `f` | Cycle the status filter (All / Enabled / Disabled) |

Hooks are grouped by source: **Global**, **Project**, **Plugin**, and **Custom**.

Each hook shows:
- **Event** it triggers on
- **Command** or **URL** that runs
- **Timeout** duration
- **Status**: enabled or `[disabled]`

### Slash Commands

```
/hooks-list           # Show hooks loaded in this session
/hooks-trust          # Trust this project for hook execution
/hooks-add <path>     # Add a custom hook file or directory
/hooks-remove <path>  # Remove a custom hook
/hooks-untrust        # Revoke trust for this project
```

In the TUI pager, the individual `/hooks-*` commands do not appear in the slash-command list. The `/hooks` modal covers listing, adding, removing, and enabling or disabling hooks; project trust is managed via `/hooks-trust` (or the modal's Trust action), which writes the unified folder-trust store described above.

### Per-Hook Enable/Disable

Enable or disable an individual hook at runtime by pressing `Space` in the Hooks tab. The change takes effect immediately, without restarting the session. Disabling one copy of a hook also disables its copies that run the same handler under the same matcher for the same event and come from a source of lower authority, or of equal authority loaded after it. Global hooks load before project hooks: disabling a global hook disables its project copies, while disabling a project copy leaves the global, config and managed copies running. Enabling a copy that another copy's disable covers enables both, and the result names the other copy. When a hook stays disabled, Enable says why, for one hook or for each hook of a source. A copy under another matcher keeps its own state, and a managed-policy copy still runs.

Hooks from an agent definition are named after the whole agent name, as `agent:<name>:<event>[i].hooks[j]`. Agents whose names contain `.` or `/` got shorter names before (`agent:review.strict` hooks were named `agent:review:…`). A disable saved under the old name no longer applies: disable those hooks again once.

Earlier versions loaded copies whose command or URL read the same as one hook. Copies that run from different hook directories, or with a different `env`, are now separate hooks, and a disable saved for one of them no longer covers the others: disable such copies again.

### Mid-Session Reload

Press `r` in the Hooks tab to reload all hooks from disk. Grok re-reads every hook source, so this picks up changes you made to hook files during the session.

---

## Hooks in the Status Row and Scrollback

Hooks are quiet unless they hold the turn up or change its course:

- While the turn is blocked on a hook batch (a `PreToolUse` gate before a tool, the `UserPromptSubmit` gate, a `Stop` gate), the status row reads `Running pre_tool_use hook…` (or `Running 3 stop hooks…`) once the batch has run for about 300 ms. The timer counts from when the batch started, so a slow hook shows its full wait; a fast one never shows at all.
- A hook that ran and allowed leaves no trace. Its stdout is not shown.
- A hook that denies a tool call, blocks a prompt, or stops or continues the agent gets one annotation line with the reason. Hooks from `~/.grok`, project, and plugin files are named; hooks from managed configuration read as "a managed policy hook".
- A hook that fails (non-zero exit, timeout, crash, malformed output) gets one line: `<event> hook (<name>) failed, ignored: <reason>`, where the reason is the exit code with the first stderr line, or the timeout. "Ignored" is literal: failures are fail-open, so the tool call or turn proceeds as if the hook had allowed it.

Deny and failure lines carry the same bullet as the tool rows, so they read as part of the tool call above them.

These lines appear only when the plugins UI is enabled (the default).

---

## Example: Safe Shell Guard

Block dangerous shell commands:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "bin/safe-shell.sh", "timeout": 5 }
        ]
      }
    ]
  }
}
```

Where `bin/safe-shell.sh`:

```bash
#!/bin/sh
INPUT=$(cat)
CMD=$(printf '%s' "$INPUT" | jq -r '.toolInput.command // empty')

if ! printf '%s' "$INPUT" | jq -e '.toolInputTruncated == false' > /dev/null; then
  echo '{"decision": "deny", "reason": "Command too long to check"}'
  exit 2
fi

# Block destructive patterns
if printf '%s\n' "$CMD" | grep -qE '(rm -rf /|mkfs|dd if=|:(){ :|& };:)'; then
  echo '{"decision": "deny", "reason": "Blocked potentially destructive command"}' 
  exit 2
fi

echo '{"decision": "allow"}'
```

---

## Security Notes

- Global hooks (`~/.grok/hooks/`) run with your user permissions; treat them like shell scripts.
- Project hooks require folder trust (`/hooks-trust` or `--trust`, the same gate as repo-local MCP/LSP) to prevent supply-chain attacks from malicious repos.
- HTTP hooks send session data; only use trusted endpoints.
- A `PostToolUse` hook decides what the model reads for that tool call — it can add instructions or replace the output outright — so trust one the way you trust a `PreToolUse` gate. The scrollback and the transcript keep the real output, so a replacement is always visible to you.

---

## Best Practices

1. **Keep hooks fast**: long-running hooks block the UI. Use background processes (`&`) or async where possible.
2. **Use explicit `deny` to block**: hooks fail open on an error.
   A hook that crashes will not block the tool.
   The exceptions are a reply past a read limit and an `updatedInput` that can't be applied (see [How a Hook Resolves](#how-a-hook-resolves)).
   To enforce policy, your hook must run to completion and emit `{"decision":"deny","reason":"..."}` on stdout. Always handle errors inside your script so it can return an explicit decision.
3. **Use absolute paths or relative to hook file**: scripts in `bin/` next to the JSON file are portable.
4. **Test with the modal**: press `Ctrl+L` (non–VS Code family) or run `/hooks` to verify hooks are loaded and matching before relying on them.
5. **Version control project hooks**: commit `.grok/hooks/` (but never secrets).

---

## Troubleshooting

- **Hook not running?** Press `Ctrl+L` on non–VS Code family (or run `/hooks` anywhere) to see if it is loaded and matched.
- **Hooks ignored?** The folder may be untrusted. Run `/hooks-trust` (or relaunch with `--trust`).
- **Script not found?** Check the path is relative to the `.json` file, or to the workspace root in a Claude settings file, and executable (`chmod +x`). A bare name such as `my-guard`, with no file of that name next to the `.json` file, is looked up on `PATH`.
- **See errors?** Capture logs by launching with `RUST_LOG=debug GROK_LOG_FILE=/tmp/grok.log grok`, then check `/tmp/grok.log`.
