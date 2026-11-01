# Cross-Session Memory

Memory lets Grok recall facts, decisions, and patterns from earlier sessions. Grok saves what it learns as Markdown notes and reads the relevant ones in later sessions, so a new session can reuse context.

---

## What Is Memory?

Without memory, each Grok session starts fresh: the model knows nothing about previous sessions. When you enable memory, Grok can:

- Recall project conventions you explained before.
- Reuse debugging steps that worked.
- Carry architectural decisions forward across sessions.
- Avoid re-asking questions it already has answers to.

Memory is experimental and disabled by default.

### How memory is organized

Memory has two scopes. Global memory holds facts that apply across all your
projects; workspace memory holds facts about one repository. Clones and
worktrees of the same repository share one workspace scope.

Each scope keeps its knowledge as ordinary Markdown files. `topics/` holds
curated notes, one file per subject, and is what Grok reads at the start of a
session. New facts captured after each turn, including a turn you stop before it
finishes, land as small observations that a later consolidation pass (`/dream`)
folds into topics. A bounded generated
index of both scopes is injected into the model's context once per session so
it can decide which topics to open.

Notes in the older `~/.grok/memory/` location are imported once: the first
time a workspace is opened, each section of the old `MEMORY.md` becomes a
topic, and sections whose name already matches a topic are appended to it
under a "From earlier sessions" heading. The old files are left in place and
are not used after that.

Memory product telemetry contains only fixed enums, booleans, counts, and
durations. It never includes prompts, statements, topic names, keywords,
paths, model output, or free-form errors.

---

## Enabling Memory

### Config (Persistent)

```toml
# ~/.grok/config.toml
[memory]
enabled = true
```

`[memory_v2] enabled = true` works too. When both keys are set, `[memory_v2] enabled` wins.

### Environment Variable

```bash
export GROK_MEMORY=1
grok
```

### Force-Disable

To disable memory for the process even when TOML or remote settings enable it:

```bash
export GROK_MEMORY=0
```

### Mid-Session Toggle

Toggle memory on or off during a session without restarting: open `/memory`
and press `t`.

The toggle is session-scoped. It does not change `config.toml`, and it works in both directions: a session that started with memory on can turn it off, and a session that started with `[memory] enabled = false` can turn it on. New sessions follow `config.toml` again. Turning memory off removes the memory instructions from the system prompt and stops capture, but keeps existing files on disk. Turning memory on sets up memory storage, restores the memory instructions, and injects the memory index on the next turn. Turning memory on waits for any turn in progress to finish.

The toggle cannot override the process-wide force-disable (`--no-memory` or `GROK_MEMORY=0`). Those hide `/memory` for the whole session.

### Priority Order

The first setting that has a value decides:

1. `[memory_v2] enabled`, then `[memory] enabled`, in `requirements.toml`.
2. The CLI flag (`--no-memory` turns memory off).
3. `GROK_MEMORY`.
4. `[memory_v2] enabled` in effective TOML.
5. `[memory] enabled` in effective TOML.
6. A managed remote setting.
7. Otherwise memory is off.

A false value in `requirements.toml` turns memory off for the whole process.
It also hides `/memory`.
A false value from the CLI or `GROK_MEMORY` also turns memory off for the whole
process. A false value in TOML turns memory off for new sessions, even when a
managed remote setting enables it, but the `/memory` `t` toggle can still turn
it on for the current session.

Memory stays off when Grok runs in a temporary directory (for example under
`/tmp/`).

---

## How Memory Is Stored

Memory lives under `~/.grok/memory-v2/`:

| Path | Contents |
|------|----------|
| `global/` | Global scope |
| `workspaces/<project>-<hash>/` | One workspace scope per repository |

Each scope has the same layout:

| Path | Contents |
|------|----------|
| `topics/` | Curated notes, one Markdown file per subject |
| `observations/_inbox/` | New observations waiting for Dream |
| `MEMORY.md` | Generated index of the scope. Do not edit it. |

Each scope also holds Grok's own state files (a SQLite database, a search
index, and archived observations). Leave them alone.

---

## Automatic Capture

After each completed turn, Grok reviews the turn in the background and saves
new facts as observations in `observations/_inbox/`. A turn you stop early is
captured too. Capture runs silently and does not add latency to your turn.

---

## Waiting for Capture with /flush

Capture runs in the background, so the last turn may not be saved yet. Use
`/flush` to wait until every finished or stopped turn is captured:

```
/flush
```

The status line shows "Flushing memory…" while it runs. When it finishes,
scrollback reports the outcome, for example "Memory flushed through turn 12".

Use `/flush` when you want the latest work saved now:
- At the end of a productive debugging session
- Before you close Grok
- Before you run `/dream`

---

## Working with Memory

### Remember

Ask Grok to remember something, and it writes the note to memory:

```
> remember to always open PR links after pushing
```

Grok can edit a topic file or add an observation. It writes project-specific
facts to workspace memory and cross-project preferences to global memory.

You can also save a note directly with the `/remember` command:

```
/remember always open PR links after pushing
```

Run `/remember` with no text to enter remember mode, where the next line you type becomes the note. Either way, Grok opens a review panel showing the note (with an optional rewritten version you can toggle with `Tab`). The note is written only after you confirm. Grok saves it as an observation in global memory, shows `Saving memory note...`, and then `Memory note saved`. The next Dream folds it into a topic. While memory is off for the session, `/remember` says so and saves nothing.

### Forget

Ask Grok to forget something, and it finds and removes the matching entry:

```
> forget the snake_case convention
```

Forget is best-effort: the model edits the topic or note that holds the entry. For guaranteed removal, open `/memory`, select the note, and press `x`. You can also edit the file under `~/.grok/memory-v2/` yourself. To locate a file, open the `/memory` browser and press `y` to copy its path.

### Recall

Ask what Grok remembers:

```
> what do you remember?
```

Grok checks the memory index and reads the matching topic files. Use `/memory` to browse the raw files.

### Direct Editing

You can edit or add Markdown files under `topics/` and `observations/_inbox/`
in either scope. Do not edit `MEMORY.md`. Grok regenerates it. Use `/flush` to
wait for capture of the current session, and `/dream` to fold observations into
topics now.

---

## Browsing Memory with /memory

The `/memory` command opens a modal showing all memory files:

```
/memory
```

Files are grouped by scope:
- **Global**: cross-project memory.
- **Workspace**: memory for the current repository.

Each scope lists its generated `MEMORY.md` index first, then its topics, then
its inbox observations, newest first.

The modal uses a split-pane layout: the file list on the left, a read-only content preview on the right. The preview updates as you move through the list.

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Move through the file list |
| `PgUp`/`PgDn` | Jump 10 entries |
| `/` | Filter the file list |
| `Enter` | Read the selected note: the preview takes keyboard focus (arrows, `PgUp`/`PgDn`, `Home`/`End` scroll it) |
| `y` | Copy the selected file's path to the clipboard |
| `x` | Delete the selected note (press `x` again to confirm) |
| `t` | Toggle memory on or off |
| `Ctrl+F` | Toggle fullscreen |
| `Esc` | Close the modal, or leave filter or preview focus |

The filter matches note names and note contents; separate words all have to match. When you filter, the preview scrolls to the first match. If nothing matches, the list says so; `Backspace` clears the filter.

The preview pane is read-only. Scroll it with the mouse wheel, by dragging its scrollbar, or with the keyboard after `Enter`. Drag across the preview text to copy that text to the clipboard; a brief message under the file list confirms every copy. Generated `MEMORY.md` indexes cannot be deleted. Dream may learn a deleted fact again from future sessions.

When the memory modal's content area is under 64 columns, the modal shows the file list only and hides the size column; press `Enter` to read the selected note full-width and `Esc` to return to the list.

When memory is off, the modal explains why and, when possible, offers `t` to turn it on.

You can also open `/memory` from the command palette.

---

## Memory Notifications

When you save a note with `/remember`, Grok confirms in the scrollback:

```
Memory note saved
```

Background work (capture after each turn and automatic Dream) runs silently and does not post a scrollback message. `/flush` and `/dream` report their outcome in scrollback when you run them yourself. Use `/memory` at any time to browse what Grok has stored.

---

## Dream Consolidation with /dream

The `/dream` command folds inbox observations into topics:

```
/dream
```

Dream merges new observations into existing topics, creates new topics, and
removes duplicates. Processed observations are archived. The status line shows
"Consolidating memory…" while it runs. Scrollback then reports how many
observations were merged into how many topics, that there was nothing to
consolidate, or that another session is already consolidating. `/dream`
requires memory to be enabled.

### Auto-Dream

Dream also runs automatically in the background once enough inbox observations
have built up, or once the oldest one has waited long enough.

---

## How Memory Affects Prompts

When memory is on, the system prompt gets a `<memory>` section. It tells the
model where the global and workspace memory directories are, and that it can
read and edit Markdown files under `topics/` and `observations/_inbox/` with
its normal file tools.

Grok also injects the generated `MEMORY.md` index of both scopes once per
session (up to 8 KiB per scope). The index lists topic titles and paths, the
topics the agent reads most first. When the titles do not fit, the remaining
topics are listed by file name. Pending observations are not listed. The model
uses the index to decide which topic files to read before it starts work in an
area.

Memory can be out of date. The prompt tells the model to check facts against
the current repository and to trust current evidence when the two disagree.

---

## CLI Commands

The `grok memory` command manages memory from the shell. It has one subcommand, `clear`:

```bash
# Clear workspace memory for the current directory. This is the default scope.
grok memory clear

# The same scope, stated explicitly
grok memory clear --workspace

# Clear global memory
grok memory clear --global

# Clear both workspace and global memory
grok memory clear --all

# Skip the confirmation prompt (-y is the short form)
grok memory clear --yes
```

`clear` deletes the whole scope directory under `~/.grok/memory-v2/`. Grok
creates it again the next time a session starts.

To edit memory from the shell, open the files in your editor directly. For
example, `$EDITOR ~/.grok/memory-v2/global/topics/<topic>.md`.

---

## Configuration Reference

### Core Settings (`[memory]`)

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Enable memory. `[memory_v2] enabled` wins when both are set. |

### Memory Settings (`[memory_v2]`)

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Enable memory |

Batch Dream, retention, and token-cap settings are in the
[config reference](26-config-reference.md#memory).

### Index Settings (`[memory.index]`)

| Key | Default | Description |
|-----|---------|-------------|
| `max_chunk_chars` | `1600` | Maximum chunk size in characters |
| `chunk_overlap_chars` | `320` | Character overlap between chunks |

### Pruning Settings (`[compaction.pruning]`)

You configure pruning under `[compaction]`, not `[memory]`, because it is a compaction behavior.

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `true` | Enable tool-result pruning |
| `keep_last_n_turns` | `3` | Number of recent turns whose tool results are never pruned |
| `soft_trim_threshold` | `4000` | Character threshold above which old tool results are soft-trimmed |
| `soft_trim_head` | `1500` | Characters kept from the start of a soft-trimmed result |
| `soft_trim_tail` | `1500` | Characters kept from the end of a soft-trimmed result |
| `hard_clear_age_turns` | `10` | Turn age after which tool results are replaced with a placeholder |

---

## Troubleshooting

### Memory Not Working

1. Verify memory is enabled: check `grok inspect` output.
2. Check `GROK_MEMORY`, `[memory_v2] enabled`, and `[memory] enabled` in effective TOML.
3. Check for `GROK_MEMORY=0` or `--no-memory` overriding config.
4. Check that you are not running in a temporary directory.

### Memory Not Appearing in Sessions

The memory index is injected once per session. If you started a session before enabling memory, start a new session with `/new`, or turn memory on with `t` in `/memory`.

### Viewing Memory Files

Use `/memory` in the TUI to browse all memory files with a preview. You can also access them directly:

```bash
ls ~/.grok/memory-v2/
cat ~/.grok/memory-v2/global/MEMORY.md
ls ~/.grok/memory-v2/workspaces/
```

### Debug Logging

```bash
RUST_LOG=debug GROK_LOG_FILE=/tmp/grok.log grok
grep "memory" /tmp/grok.log
```
