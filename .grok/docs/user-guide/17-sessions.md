# Session Management

Grok saves every conversation to disk automatically. Whether you work in the TUI, in headless mode, or over agent stdio, Grok records the exchange as a session. You can resume, rewind, or compact it. This document describes how to manage sessions.

---

## What Sessions Are

A session is a persistent conversation with full history. It includes:

- All user prompts and agent responses
- Tool calls and their results
- TODO/task list state
- Rewind points for undoing later turns
- Token usage and turn counts
- Subagent sessions (when enabled)

Sessions are identified by a unique session ID (a UUIDv7 when Grok generates it; a client may supply its own ID with `-s`) and stored on disk under `~/.grok/sessions/`. Set `GROK_HOME` to override the base directory; when it is unset, Grok uses `~/.grok`.

---

## Storage Layout

Grok stores each session in its own directory, grouped by working directory. It URL-encodes the working directory to name the group. When the encoded name exceeds 255 bytes, it instead uses a slug plus a hash and records the original path in a `.cwd` file inside the group.

```
~/.grok/sessions/<encoded-cwd>/<session-id>/
  summary.json            # metadata: summary/title, timestamps, model ID, message counts
  updates.jsonl           # ACP session update stream (conversation + tool calls)
  chat_history.jsonl      # raw chat messages sent to the model
  system_prompt.txt       # the rendered system prompt, as sent to the model
  prompt_context.json     # the inputs the system prompt was rendered from
  tool_definitions.json   # function tools sent on the latest model call (no MCP server__tool entries)
  plan.json               # TODO/task list state
  rewind_points.jsonl     # rewind points for /rewind undo
  signals.json            # session signals (token usage, tool/turn counters)
  feedback.jsonl          # user feedback and ratings
  compaction_checkpoints/ # saved state from compaction (manual or auto)
  subagents/              # per-subagent metadata (meta.json); the child sessions live in the normal sessions tree
```

`summary.json` is the index entry. It records the session summary and generated title, the model ID, the creation and update timestamps, the message counts, and a parent session reference for forked or restored sessions. It also records the latest last-turn summary and session recap so listing surfaces can show them. `updates.jsonl` is the authoritative conversation log that drives `/resume` and session restore. `tool_definitions.json` omits MCP `server__tool` entries because the model reaches those through `search_tool` and `use_tool`, which are listed. Per-turn token and cost totals are available through `grok usage`.

### Session titles

The session title shown in the dashboard and `/resume` is generated automatically from the conversation. The prompt border shows a title only after a manual `/rename`, alongside the `Stashed` caption when a draft is stashed. Title generation starts right after your first prompt so a session always has a title, and then the title is regenerated from the whole conversation at a couple of early turns and frozen. This lets the title move past a vague first prompt to reflect what the session is really about, while staying stable afterward so you don't lose track of your sessions. A manual `/rename` always wins: once you rename a session, automatic generation never overrides it. Use `/rename --auto` to hand the title back to automatic generation.

---

## Starting and Ending Sessions

### New Session

The TUI creates a new session each time you launch. To explicitly start fresh mid-session:

```
/new
```

This clears the current context and begins a new conversation. Alias: `/clear`.

### Exit

End the session and quit Grok:

```
/quit
```

Alias: `/exit`. To leave the current session but stay in Grok, use `/home` to return to the welcome screen.

### Delete the current session

```
/delete
```

Confirms, then permanently removes the session history. Returns to the welcome screen, or to the dashboard when you opened the session from the dashboard. From `/resume` or the welcome session list, press `d` then `y`. In the [Agent Dashboard](23-dashboard.md) list, `Ctrl+X` twice (or hover `[✗]`) permanently deletes.

---

## Resuming Sessions

### From the TUI

Use the `/resume` command to browse and resume previous sessions:

```
/resume [session-id-or-title]
```

Without an argument, this opens a session picker that lists recent sessions for the current workspace. Select a session to resume it. A session ID, or a title that picks exactly one session by the [`grok --resume` rules](#from-the-command-line), resumes that session directly. If Grok was started with `--restore-code`, a remote session's ID says why instead when that session was restored in this directory: as with `grok --resume`, a remote session's code is checked out only into a worktree. On the daemon route, if the session the daemon finds was started under another sandbox profile, `/resume` says why instead, unless `requirements.toml` pins the profile (see [Resuming Sessions in Sandbox Mode](18-sandbox.md#resuming-sessions)). The daemon looks in the session store of the profile Grok runs under: `strict` and each custom profile keep their own, and the other built-in profiles share one (see [Built-in Profiles](18-sandbox.md#built-in-profiles)). Anything else (a title several sessions share, a title no session has, an ID that isn't found, a session kept in another profile's store, or a session that can't be resumed from this directory) opens the picker with your text already typed as its search. In chat mode, `/resume <text>` always opens the picker that way, even for a session ID. On the workspace dashboard, `/resume <text>` opens the dashboard's picker filtered by the text instead.

Typing in the picker narrows the sessions it has already listed to those whose session ID or title holds what you type, or whose title has each word you type at the start of one of its words, in any order and whatever separates them, so `login-fix` keeps a listed "Fix login bug". Chinese, Japanese, Thai and other scripts written without spaces between words run several words into one, so a word you type may also start inside one of the title's words, at a character of such a script or right after one: `登录 问题` keeps a listed "修复登录问题", and `rust 重写` keeps "使用Rust重写" but not "使用Trust重写". When each word you type is a single character outside those scripts, as in `c++` or `C#`, a title must hold the text as typed or have the same words in the same order, so `C#` keeps a listed "C++" but not "C notes". It also searches your conversation content as you type; content matches appear under an "Extended search results" heading. Press `Ctrl+/` to search immediately without the brief pause.

For the live top-level sessions in this pager (parent and forks) — switch, rename, peek, dispatch, or close — use the [Agent Dashboard](23-dashboard.md): `/dashboard` (aliases `/sessions`, `/agents-dashboard`) or `Ctrl+\`.

### From the Command Line

Resume a specific session by ID or title:

```bash
grok --resume <session-id-or-title>
```

A value that is not a session ID is matched against session titles for the current directory, ignoring letter case (a simple lowercase comparison) — handy after `/rename`. When no title matches exactly, a title with the same words in the same order matches too, whatever separates them, so `grok --resume fix-login-bug` finds "Fix login bug"; quote a name that contains spaces. If several sessions match, a single manually renamed session wins over auto-generated duplicates; otherwise the command errors and lists up to ten matches, newest first, each with its ID, title, and last update time. When no title matches, the error suggests up to five similar titles (every word you typed starts a word of the title, or starts inside one at a character of a script written without spaces between words or right after one) with their IDs. UUID-shaped values are always treated as session IDs, never titles. Scripts should prefer IDs.

On the daemon route, a session ID also finds a session the daemon holds at the same place in the repository's main checkout or one of its `grok -w` worktrees (at its root, or in the subfolder you run from), and opens it in that folder.

Run `grok --resume` without a value to resume the most recent session for the current directory.

### From the Welcome Screen

When you launch `grok`, the welcome screen lists recent sessions for the current directory. Select one to resume it.

---

## Forking and Renaming Sessions

### Fork

Branch the current session into a peer agent that starts from a copy of the conversation:

```
/fork [--worktree|--no-worktree] [directive]
```

Pass an optional `directive` to set the new session's first prompt. Use `--worktree` or `--no-worktree` to choose whether the fork runs in a new git worktree; omit both to be asked each time. The `--at <turn>` flag is not supported in this version.

A `/fork` keeps its source's model and effort, even when grok was launched with `-m` or `--effort`.

### Rename

Rename the current session's title:

```
/rename <title>
/rename --auto
```

Alias: `/title`. `/rename --auto` clears a manual title and re-enables auto-titling.

---

## The /rewind Command

`/rewind` (alias `/undo`) rewinds the conversation to an earlier turn, dropping later turns. File changes made after that turn are left as-is on disk.

```
/rewind
/undo
```

When you run `/rewind` or `/undo` (or press **Esc Esc** within 800ms while idle with an empty prompt and conversation messages), Grok:

1. Shows a list of rewind points (one per user prompt)
2. Lets you select which point to rewind to
3. Truncates the conversation history to that point

When **Confirm before rewind** is on (default in `/settings`), every pick asks for confirmation (Yes / Yes, and don't ask again / No). **Yes, and don't ask again** turns that setting off. With the setting off, picks run immediately.

**Important:** `/rewind` does not restore files on disk. Only conversation history is truncated.

---

## The /compact Command

`/compact` compresses the conversation history to save context window space. Use it in long sessions where early messages are no longer relevant.

```
/compact
```

### Auto-Compact

Grok automatically compacts the conversation when the context window approaches its limit. You will see a notification when auto-compact triggers. The `context_window` setting on your model configuration controls when this threshold is reached.

---

## The /session-info Command

View details about the current session:

```
/session-info
```

This shows:

- Session title (when set)
- Shell version
- Auth method (OAuth vs API key; API-key sessions also suggest `grok login` for SuperGrok)
- Session ID
- Working directory
- Model (with a model hash for coding models)
- API backend and sandbox profile (when set)
- Context window usage (used and total tokens, with the percentage used)

On the Session info tab, click a value to copy it, or drag to select a range (same highlight as the tool viewer). `c` copies the session ID and `y` copies the whole block. Copy uses the same clipboard route as the rest of Grok, including `grok wrap`.

---

## Headless Session Management

In headless mode, you manage sessions through command-line flags:

```bash
# New session each time (default)
grok -p "Hello"

# Resume an existing session by ID or title (errors if it does not exist)
grok -p "Continue where we left off" -r <session-id-or-title>

# Continue the most recent session in the current directory
grok -p "What were we doing?" -c
```

In headless mode, resume an existing session with `-r`/`--resume`, which errors if the session does not exist, or continue the most recent session in the current directory with `-c`/`--continue`. A non-ID value is matched against session titles for the current directory, ignoring letter case; without an exact match, the same words joined by any separators also match, so `-r fix-login-bug` finds "Fix login bug" (a sole manually renamed match wins among duplicates; remaining duplicates error with up to ten of them, newest first, with IDs and last update times; a miss suggests up to five similar titles with their IDs; UUID-shaped values always take the ID path) — scripts should pass the session ID from JSON output (see below) to `-r`.

Use `-s`/`--session-id` only to **create** a new session with a **UUID** (errors if the value is not a UUID, or if that ID already has a session under the target session directory). It does **not** resume an existing session — that was the old hidden upsert behavior; use `-r`/`-c` instead. Combine `-s` with `-r`/`-c` only when also passing `--fork-session` (forks history into a new ID; optional `-s` names the child UUID). This matches Claude Code’s anti-overwrite model (client preflight under the write cwd; sequential use is reliable, concurrent same-ID is best-effort).

To read the session ID back, request JSON output:

```bash
grok -p "Hello" --output-format json | jq -r '.sessionId'
```

---

## Agent stdio Session Management

When building with ACP, sessions are managed via protocol methods:

```typescript
// Create new session
const { sessionId } = await connection.request("session/new", {
  cwd: "/path/to/project",
  mcpServers: [],
});

// Load existing session
await connection.request("session/load", {
  sessionId: "existing-session-id",
  cwd: "/path/to/project",
  mcpServers: [],
});

// Change a live option (model or reasoning_effort).
// session/new and session/load already return the typed configOptions list.
await connection.request("session/set_config_option", {
  sessionId,
  configId: "model",
  value: { value: "grok-4.6" },
});
```

The agent persists all session updates automatically. Clients can reconnect and load previous sessions by ID. See [Agent mode](15-agent-mode.md#session-config-options) for the option IDs, value shape, and leader-mode snoop.

---

## The grok sessions Subcommand

List or search sessions from the command line. `grok sessions` requires a subcommand:

```bash
# List recent sessions for the current directory
grok sessions list

# Limit the number of results (default 20)
grok sessions list --limit 50

# Search sessions by keyword (matches titles and prompts)
grok sessions search "rate limit"
```

`grok sessions list` shows sessions for the current working directory, grouped by worktree label. Each row lists the session ID, the creation and update dates, the source status, and the summary. `grok sessions search` combines a local SQLite index with remote results.

---

## The grok usage Subcommand

Print persisted token and cost usage for a session. Use this instead of reading session files:

```bash
# Session totals plus every recorded turn
grok usage <session-id>

# One turn
grok usage <session-id> 3
```

Output is JSON with `sessionId`, `updatedAt`, `session`, and `turns`. A specific turn uses the same envelope with one element in `turns`. Session totals cover the whole conversation, including history inherited by resume or fork. `costUsdTicks` is 10¹⁰ ticks per USD (divide by `1e10` for dollars). A missing turn number is an error. Interactive credit and billing stay on `/usage` in the TUI.

---

## Worktree Sessions

When working with subagents or session forks, Grok can create isolated git worktrees per session. Each worktree gets its own copy of the working directory, so file changes in one session do not affect another.

Worktree sessions are managed internally through the `x.ai/git/worktree/*` extension methods. Key operations:

- **Create**: Create a new worktree for an isolated session
- **Apply**: Merge worktree changes back into the main working directory
- **Remove**: Clean up a worktree when the session is done

Resume a session in a fresh worktree with `grok -w -r <session-id>`.

A worktree also applies the `deny` and `ask` rules of the main checkout's ignored `.claude/settings.local.json` (see [Where Permission Rules Live](22-permissions-and-safety.md#where-permission-rules-live-scopes)).

A worktree also uses its main checkout's saved "never allow" answers, and its saved "always allow" answers while `[features] worktree_inherits_main_approvals` is on, as it is by default. Grok Desktop and hosted workspaces are the exception: their tool-approval gate checks only the worktree's own answers (see [Persistence Is Per Project](22-permissions-and-safety.md#persistence-is-per-project)).

`grok worktree create [NAME]` creates the same worktree `grok -w [NAME]` would, without starting a session. It prints the directory the session would have opened in. That path is the only output on stdout, so `cd "$(grok worktree create my-fix)"` works. By default the worktree starts from HEAD with your uncommitted changes copied over. Pass `--ref <ref>` to start from a clean checkout of a branch, tag, or commit. If the new worktree lacks the directory you ran from, such as a `--ref` checkout that predates it or an ignored directory, the command prints the worktree root.

### Manage Grove redirections

A Grove worktree can redirect ignored artifact directories such as `target` and `node_modules` to storage outside the projected tree. The redirect commands take the mount path as their first argument.

```bash
grok worktree redirect list /path/to/worktree
grok worktree redirect list /path/to/worktree --json
grok worktree redirect add /path/to/worktree target bind
grok worktree redirect del /path/to/worktree target
grok worktree redirect fixup /path/to/worktree
grok worktree redirect unmount /path/to/worktree target
```

`list` prints `repo_path`, `type`, `mechanism`, `target`, `source`, and `state`. Run `unmount` without a repo-relative path to take down every redirect on the mount. Use `fixup --force` to replace Grove-owned residue. Use `fixup --strict` to refuse a populated plain directory.

To enable the safe allowlist on a new Grove clone, pass `--redirect-ignored`. Add repeatable entries with `--redirect-dir`. A redirect directory stays inactive unless redirects are enabled by the command or Grove configuration.

```bash
grok clone https://example.com/org/repo.git --redirect-ignored
grok clone https://example.com/org/repo.git \
  --redirect-ignored --redirect-dir build --redirect-dir '**/node_modules'
grok clone https://example.com/org/repo.git --no-redirects
```

`GROVE_REDIRECTS=0` remains a runtime kill switch. Grok does not save the kill switch as the clone's redirect choice.

### Checking Disk Usage

`grok du` (alias: `grok disk-usage`) reports what the grok home (`~/.grok`) uses on disk. It lists each top-level directory, largest first, then each worktree with its size, type, age, label, and path. Worktrees the registry does not track appear as `untracked`. Pass `--json` for the same report as machine-readable output.

```text
Disk usage for ~/.grok
    412.3 GB  worktrees
      1.2 GB  sessions
    412.0 MB  (top-level files)
    413.9 GB  total
  Worktree clones share storage with their source, so the total can exceed real disk use.

Worktrees
        SIZE  TYPE                AGE        LABEL  PATH
    380.0 GB  session             12d ago    my-fix ~/.grok/worktrees/xai/worktree-abc
     32.3 GB  untracked (session) 40d ago           ~/.grok/worktrees/xai/worktree-old

To reclaim space, run `grok worktree gc --max-age 7d --dry-run`, then the same command without `--dry-run`. Without `--max-age`, gc expires nothing, and it keeps a worktree whose work it cannot find elsewhere, naming each one.
Untracked rows are not in the registry, so gc never visits them. Remove one with `grok worktree rm --dry-run <path>`, then without `--dry-run`.
```

After the grok-home table, `grok du` may print **Redirections**, **Orphaned redirections**, and **Unattributed redirect directories**. Those bytes live in Grove escape jails, not in the grok-home total. An empty scan prints nothing. Reclaim a live jail with `grok worktree clean-artifacts`. Purge live jails plus proven orphans with `grok du --clean --yes`. Delete only proven orphans with `grok du --clean-orphaned --yes`. The running Grove daemon classifies and measures the jails, and only those in its own data dir, which the report names. Jails in other Grove data dirs, or in every dir when no daemon is running, are listed read-only under their data dir with sizes only. `--clean`, `--clean-orphaned` and `grok worktree clean-artifacts` ask the daemon to do the deleting; without a running daemon they refuse, so start it with `grove daemon` and retry.

`AGE` is the value `grok worktree gc` measures: time since the worktree was last accessed, or since it was created when that is more recent. Session and agent activity update it; a shell or editor left open in the directory does not. An untracked worktree has no registry entry, so its age comes from the newest file underneath it.

Sizes are physical block counts on Unix and logical file sizes elsewhere, matching what `grok worktree show` reports. A worktree clone shares storage with its source and each copy counts in full, so the total can exceed both `du -sh` and the space actually in use. When the total exceeds the used space on the volume, the report says so. `--json` carries the same figures as `volume_capacity_bytes` and `volume_available_bytes`. For a Grove worktree, `grok worktree show` instead asks the Grove daemon that owns it for the disk usage of its backing (upper, git and other) and its redirections, and never walks the mount; when no daemon answers, it prints `unavailable` with the reason, and a path no Grove mount serves (for example after `detach`) also gets a `Local Size:` line with the directory's own size.

The report measures a single filesystem, the one holding the grok home. A directory on any other filesystem stays out of the total and is counted in `other_filesystem_dirs`, and its worktree rows show `-` for size (`null` in `--json`). A top-level symlink to a directory, such as a relocated `worktrees`, is counted in `unfollowed_dir_symlinks`; its target stays out of the total, though the rows below it are still sized. Directories and entries the report could not read are counted in `unreadable_dirs` and `unstatable_entries`. Run `RUST_LOG=debug grok du` to name each one.

Every worktree row in `--json` also carries `created_at`, `last_accessed_at`, and `last_modified_at` in unix seconds, plus `repo_name` and `git_ref`. Registry fields are `null` for untracked rows. `git_ref` is the branch recorded when the worktree was registered, not the branch checked out now.

When the registry is unavailable, every row appears as `untracked` and the report names the reason. The `--json` `registry` field carries the same value: `read`, `absent`, `busy`, `unopenable`, or `corrupt`. A `busy` registry is held by another process, so retry. An `unopenable` one has a permission or I/O problem, so check the file. A `corrupt` one is the only case that calls for deletion: remove the file the report names, then run `grok worktree db rebuild`. The rebuild registers only the worktrees under the grok home's `worktrees` and `worktree_pool` folders, plain or Grove. It skips anything it finds outside them, such as another grok home's Grove worktree or one created at a path outside those folders, and counts those under `Skipped`.

To reclaim space, run `grok worktree gc --max-age 7d`, which removes tracked worktrees older than the age you give. Without `--max-age`, gc expires nothing, and it visits only worktrees the registry tracks. One exception applies on macOS and Linux with the grok home on a local disk. If grok exits or is killed while copying a worktree, the next gc run removes it. This happens with or without `--max-age`. The gc report counts it under `Unfinished copies removed`. A Grove or overlay worktree that was already mounted when grok exited stays, and gc lists it as kept (`mounted`). So does a copy killed before it wrote its `.git`, such as a plain copy's first files: gc lists it as kept (`no-repo`) on every run. Remove that folder with `grok worktree rm <path>`, and the next gc run stops listing it. Remove an untracked worktree with `grok worktree rm <path>`. Both commands take `--dry-run` and report what they would do: gc counts the worktrees it would remove, and `rm` names the path and marks a worktree with unsaved work, for example `(unsaved work: dirty)`. That includes `reflog-only-commits`: commits that only a linked worktree's own reflog still names (after an amend, rebase, or reset), or only a `FETCH_HEAD` line a fetch asked for, such as `git fetch <url> <branch>`. Removing the worktree loses those commits. Without `-f`, `rm --dry-run` also marks a worktree that a running process or a session open in the grok TUI works inside (a headless run counts only with `GROK_TRACK_HEADLESS` set), as `(in use: a process or a session open in the grok TUI works inside it)`. It prints `would keep:` for each worktree that `rm` would keep. A Grove worktree is judged by its Grove daemon, which can also answer `(removal refused: ...)`. It prints `unsaved work unknown` when it cannot judge: a FUSE or NFS mount, such as a Grove mount (git never runs inside one), a directory that is not a git worktree, or a check that could not finish or tell. A Grove clone is marked `unsaved work not checked`.

Each run judges as many worktrees as it can in about a minute, because the same pass runs on a timer beside your session and reading a whole working tree is not free. Anything it did not reach is counted as `Not judged this pass` and waits for the next run, so on a machine with a lot to reclaim, run gc again until that number is zero.

Before removing an expired worktree, gc checks whether the removal would destroy work: uncommitted, untracked or ignored files, a commit no surviving ref holds, or state kept only in that worktree's git directory. A worktree it cannot check is kept as well. The report counts kept worktrees and names the reason, separately from the ones a live process or a session open in the grok TUI held back. `--force` does not skip the check.

Without `-f`, `grok worktree rm` keeps a worktree that a running process or a session open in the grok TUI works inside, or one with unsaved work. A running creator alone does not keep it, because one Desktop or TUI process creates worktrees for many sessions and outlives them. `rm` prints `kept: <path>  (reason); pass -f to remove it anyway` for each worktree it keeps, then exits 1. Sometimes grok can't tell whether the folder is still mounted. The line then says `-f` won't remove it either. For a worktree in use, the line also names any unsaved work that `-f` would discard.

Without `-f`, `rm` also keeps a worktree when its own check failed or did not finish, since the check may have missed unsaved work. The kept line names:

- `unsaved work unknown: check-failed` (for example a folder it can't read)
- `unsaved work unknown: gate-timed-out` (the check took longer than 30 seconds)
- `unsaved work unknown: check did not finish: ...` (the check stopped before it answered; not a timeout)
- `unsaved work unknown: mount table unreadable, not inspected`
- `unsaved work unknown: path could not be resolved in time, not inspected`
- `unsaved work unknown: unreadable symlink (...), not inspected`

Other notes do not keep a worktree by themselves:

- `in use unknown`, whatever reason follows it, such as the process scan's `check did not finish: ...`
- an `unsaved work unknown` reason other than those six, such as `not a git worktree` or `FUSE or NFS mount, not inspected`
- an unknown answer from a Grove worktree's daemon, even `unsaved work unknown: gate-timed-out`

When no running daemon has the Grove worktree, or asking its daemon fails, `rm` refuses instead, as described below. A Grove clone is not checked. `rm -f` skips these checks and removes the worktree you name. It still fails when grok can't read the mount table, because grok won't delete a folder that may still be mounted.

On jj 0.46.0, `jj workspace add` in a repository that keeps jj and git in the same checkout also makes the new workspace a git worktree. Without `-f`, `rm` keeps that workspace, and the line says `unsaved work: ignored-content: .jj`.

A session counts as open until its tab is closed or grok quits. `grok worktree gc --force` ignores open sessions. Without it, a `gc --max-age` run that cannot read grok's list of open sessions expires nothing and its report says why under `Age expiry skipped`. When a new session finds malformed JSON in `~/.grok/active_sessions.json`, Grok preserves the original as `active_sessions.json.corrupt`, salvages complete session rows, and registers the new session. Rows damaged by the corruption may be missing. While the backup exists, age-based gc remains disabled so those missing rows cannot expose another running session's worktree to expiry. Reopen other running sessions, then remove `~/.grok/active_sessions.json.corrupt` to re-enable age expiry. Grok leaves well-formed JSON with an incompatible session schema untouched. On Windows no session counts. Grok cannot check that the process still runs there. gc does not read the list. `rm` still reads it. With no process list on Windows its `removed:` line already says `in use unknown`. A list it cannot read adds its reason there.

`rm` also refuses a name or path that the registry does not track, with or without `-f`. It still removes an untracked folder at `~/.grok/worktrees/<repo>/<name>` or `~/.grok/worktree_pool/<repo>/<name>`. For example, once the `docs` worktree is gone, `grok worktree rm docs` reports `not a grok worktree: docs; grok removes only worktrees it made`. It leaves your checkout's `docs` folder alone. To delete a folder grok did not make, use `rm -r` or `git worktree remove`. A repository's own checkout (such as `.` in a repository's root) and an unregistered path with `..` are refused too, with or without `-f`.

A Grove worktree gets the same check from its Grove daemon, mounted or not; gc never reads its mount or storage itself. The daemon also keeps a worktree its write journal saw change within the age window (`recently-written`, even with `--force`), one with files open through its mount (`in-use`), and one it would refuse to remove, such as a worktree another was forked from or one with a fetch running. The removal is a single daemon step that checks again first, so a write that lands after gc's check keeps the worktree. These removals are counted as `Grove worktrees removed`, and the automatic pass makes them too. When no Grove daemon answers, or none of the running daemons has a row for the worktree, gc leaves it and counts it as `Skipped (no Grove daemon)`; start the daemon that owns it and run gc again. A worktree no daemon will ever own again (its data dir is gone) stays in that count on every pass until `grok worktree rm --force` drops grok's entry for it. `grok worktree rm --dry-run` asks the daemon the same question for a Grove worktree and marks the line, for example `(unsaved work: dirty)` or `(removal refused: ...)`. `grok worktree rm` removes a Grove worktree only through the daemon that has it, and a refusal from that daemon is final. When no running daemon has the worktree, `rm` refuses: the daemon that owns it may be stopped, so start it and retry. It also refuses when a daemon's check fails; restart or upgrade that daemon and retry. `rm --force` then drops only grok's registry entry and says so; it leaves the directory and Grove's storage untouched. gc never drops a Grove worktree on its own guess.

Ignored files count as work, with one exception: a directory the repository's own ignore rules exclude and that either carries a tool's cache tag or is named like one of its output directories (`target`, `node_modules`, `.venv`, and the rest). A name alone is never enough, so a hand-written `build/` nobody excluded still keeps the worktree.

A commit that only a worktree's own reflog names, which is what a `reset --hard` or an amend leaves behind, gets a lasting name under `refs/grok/reclaimed/<worktree>/<reclaim-ts>/<object>` in the repository the worktree came from. Git counts a reflog as reachability when it prunes, so without that name removing the worktree is what would make the commit unreachable. Names also cover a commit named only by a `FETCH_HEAD` line a fetch asked for, such as `git fetch <url> <branch>`, a stopped notes-merge resolution, a tag that only the worktree reaches, and any tree or blob it names directly. Browse the named commits with `git log --glob=refs/grok/reclaimed`, and list every name, including those at tags, trees and blobs, with `git for-each-ref refs/grok/reclaimed`. Recover one with `git branch <name> <commit>`. `git branch` needs a commit, so for a name at a tag, tree or blob use `git tag <name> <id>`. A notes-merge resolution commit has a fixed 1970 date, so `git log` lists it under 1970.

Those names do not accumulate. Each gc pass drops the ones that no longer hold anything: its commit is now reachable from a real ref, its tag is now reachable from a ref, or it is more than 30 days old. A name at a tree or blob can stay until it is 30 days old, even once a ref reaches it. The report counts them as `names_collected`.

A plain `grok worktree list` (no `--json`, `--repo` or `--type`) names each unfinished copy that `grok worktree gc --dry-run` counts. It says that gc removes it. It leaves out a copy gc keeps (`mounted`, `no-repo`, `check-failed`) and a tree a running process is inside. If the whole listing does not finish within 10 seconds, for example on a hung network mount, list names none. It also names none when the registry is still busy after a short wait at open. A failed registry lookup for one copy leaves out only that copy.

---

## Session Storage Details

### Persistence Format

Grok stores the conversation as newline-delimited JSON (JSONL). Each line in `updates.jsonl` is a self-contained ACP session update event. This format supports:

- Incremental writes (append-only during a session)
- Efficient streaming reads (for session restore)
- Easy debugging (each line is valid JSON)

The smaller state files -- `summary.json`, `plan.json`, and `signals.json` -- are plain JSON rather than JSONL. JSONL is the source of truth for session content; `grok sessions search` additionally maintains a local SQLite FTS5 index over session titles and prompts for fast keyword search.

### Session Metadata

`summary.json` records, among other fields:

- `info` -- the session ID and working directory
- `session_summary` and `generated_title` -- the session summary and its model-generated title
- `title_is_manual` -- true when the title was set by a manual `/rename` (so automatic generation leaves it alone)
- `created_at` and `updated_at` -- creation and last-update timestamps
- `num_messages` and `num_chat_messages` -- update and chat-message counts
- `current_model_id` -- the model in use
- `parent_session_id` -- the source session for a fork or restore
- `agent_name` -- named agents persist this only; an inline `--agent-profile` session also persists `agent_profile` JSON
- `last_turn_summary` -- an ultra-short summary of the most recent turn
- `last_recap` -- a bounded preview of the latest session recap

### Disk Usage

Session history (`updates.jsonl`, `chat_history.jsonl`) dominates disk usage in long sessions. Use `/compact` to reduce history size.

---

## Tips

- Use `/new` to start fresh when your current context is no longer relevant.
- Use `/compact` proactively in long sessions to keep the context window effective.
- Use `/rewind` to undo mistakes; it rewinds the conversation to an earlier turn (file changes from removed turns are left as-is).
- In headless mode, capture the `sessionId` from JSON output and pass it to `-r` to build multi-step automations that maintain context.
- Check `/session-info` to see how much of your context window has been used.
