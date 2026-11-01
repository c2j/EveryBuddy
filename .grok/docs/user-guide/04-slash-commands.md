# Slash Commands

Type `/` in the prompt to open the command menu. It fuzzy-matches as you type, and picking a command runs it immediately.

Commands come from two places: **shell builtins**, handled by the agent backend (xai-grok-shell), and **pager builtins**, handled by the pager frontend (xai-grok-pager). Both show up in the same menu, and any enabled skill with `user-invocable: true` appears there too. If a skill reuses a built-in name such as `login`, the built-in keeps `/login` and the skill stays available as `/plugin-name:login` — the menu badges both so the collision is visible.

Every command below lists its aliases where it has them. A few commands only appear when a feature or session state enables them; those cases are called out inline. The menu is also filtered by render mode — see [`/minimal` and `/fullscreen`](#minimal-and-fullscreen).

---

## Session Management

### `/new`

Start a fresh session and clear the current conversation. Alias: `/clear`.

### `/resume`

Open the session picker to reload a previous session from disk. `/resume <session-id-or-title>` resumes that session directly when the ID or title picks exactly one; on the daemon route, it says why instead when the session the daemon finds was started under another sandbox profile, unless `requirements.toml` pins the profile (see [Resuming Sessions in Sandbox Mode](18-sandbox.md#resuming-sessions)). Otherwise the picker opens filtered by the text, including when the session is kept in another profile's own session store. See [Resuming Sessions](17-sessions.md#from-the-tui).

### `/dashboard`

Open the [Agent Dashboard](23-dashboard.md): live roster of top-level sessions in this pager (peek, reply, dispatch, pin, rename, stop, attach). Aliases: `/agents-dashboard`, `/sessions`.

Not `/config-agents` (alias `/agents`), which manages agent *definitions* and personas. Hidden in minimal mode; disable with `GROK_AGENT_DASHBOARD=0` or `[dashboard].enabled = false`.

### `/compact`

Compress conversation history to reclaim context-window space.

Grok also auto-compacts once the context window hits 85% (tune it with `[session] auto_compact_threshold_percent`).

### `/context`

Show the context window split into System prompt, Messages, Reasoning/overhead, and Free.
Rows for Tool definitions, Skills, and MCP servers are already counted in those totals.

### `/session-info`

Show session details — auth method, model, turn count, and context usage. Aliases: `/status`, `/info`. Click a value or drag to select and copy; `c` copies the session ID and `y` copies the whole block.

### `/fork`

Branch the current session into a new agent, keeping history up to this point.

### `/rewind` (alias: `/undo`)

Roll the conversation back to an earlier turn and discard everything after it. `/undo` is the same command.

### `/copy`

Copy the most recent response's source markdown to the clipboard. Pass a number to copy the Nth-latest response instead, or a file path to write the text to a file rather than the clipboard (handy over SSH, where the local clipboard is often unreachable).

```
/copy
/copy 2
/copy out.txt
/copy 2 ~/exports/last-reply.md
```

Every copy is also written to a backup file — `~/.grok/last-copy.txt` by default, or `GROK_COPY_FILE` if set. Confirmed copies toast briefly (e.g. `Copied!`). Unverified OSC 52 deliveries and clipboard-unreachable fallbacks name the backup path so you can recover the text.

A file path must name a new file. If the name is taken, `/copy` writes nothing and says so; the file stays as it was.

### `/export`

Export the conversation to a file or the clipboard. As with `/copy`, a file name that is taken is refused and the file stays as it was.

### `/quit`

Quit the application. Alias: `/exit`.

When quitting would stop workflow runs (see [Quitting while workflows run](03-keyboard-shortcuts.md#quitting-while-workflows-run)), the first `/quit` only shows a notice. Send `/quit` again, or `exit` or `:q`, as your next message within 30 seconds, from the same tab, to quit.

### `/home`

Leave the current session and return to the welcome screen. Alias: `/welcome`.

### `/delete`

Delete the current session's history. Confirms first. Stops any running turn, background tasks, and subagents before wiping history. Returns to the welcome screen, or to the dashboard when you opened the session from the dashboard.

To delete a session you are not in, open `/resume` or the welcome session list and press `d` then `y`. In the dashboard list, press `Ctrl+X` twice or click `[✗]`.

### `/rename`

Rename the current session. Alias: `/title`.

```
/rename new session title
/rename --auto
```

`--auto` unpins a manual title and lets auto-titling resume. It applies to Build sessions only — chat conversations have no local auto-titler. It must be the only argument (`/rename --auto Something` is an error). A session cannot be named `--auto` via this command; use the dashboard rename editor (`Ctrl+R`) for that pathological case.

---

## Model and Mode

### `/model <name>`

Switch models. Accepts a model ID or display name (case-insensitive). When the model offers more than one context window, you can add a window size next. For reasoning models you can add an effort level last. The picker asks in the same order: model, then window, then effort. Alias: `/m`.

```
/model grok-4.6
/model Grok 4.6
/model Reasoning X high
/model Reasoning X 500k high
```

Some agents resolve model ids themselves and list other settings for their models, such as a faster serving tier; the built-in agent does neither. With such an agent, `/model` sends an id it does not list as typed, and a setting is picked in the id as `name[setting=value]`, such as `grok-4.7[fast=true]`. The same id works with `-m` and as the saved default, and the picker offers the model's setting as a last step after the effort. The agent refuses a setting or value its model does not list. Such an agent may also take its own flat ids, such as `grok-4.7-high-fast`, each naming one variant; a window or effort typed after one overrides the id's own; a model without effort levels reports the effort as not applied. Models an agent lists from a custom endpoint have no flat ids, so there only model names and the bracket form work. An id the agent cannot use is refused with its flat ids. A model's own name keeps the session's settings on that model, as `/effort` and `/context-window` send it, even where it is also a flat id; `name[fast=false]` picks the standard tier.

```
/model grok-4.7[fast=true]
/model grok-4.7[fast=true] high
/model grok-4.7-high-fast
```

### `/effort <level>`

Set reasoning effort on the **current** model without reselecting it. Levels are `low`, `medium`, `high`, and `xhigh`, and it only applies when the active model supports reasoning effort.

```
/effort high
```

### `/context-window <size>`

Set the context window for the current model. The command is listed only when the model supports more than one size. It accepts the short label or the raw token count. The choice lasts for the session. It carries over to a new model that supports the same size. A size smaller than the current usage starts an auto-compact.

```
/context-window 500k
/context-window 256000
```

### `/always-approve` and `/auto`

Both are real toggles for the permission mode: they stay in the menu, and running the mode you're already in turns it back off.

| Command | When off | When already on |
|---|---|---|
| `/always-approve` | Skip all permission prompts | Back to ask |
| `/auto` | Auto-review: a classifier approves safe tools (dangerous ones may still prompt) | Back to ask |

Running one while the other is active switches modes — for example, `/auto` while always-approve is on switches to Auto-review. `/auto` only appears when the Auto-review permission-mode feature is enabled. You can also change mode with `Shift+Tab` (cycles Normal / Plan / Auto-review (when enabled) / Always-approve), `Ctrl+O`, or `/settings`.

### `/multiline`

Toggle multiline input. When it's on, `Enter` inserts a newline and `Shift+Enter` (or `Alt+Enter`) sends the message. Mid-turn, a bare `Enter` on an empty composer still force-sends the top queued follow-up. Alias: `/ml`. With multiline off, the composer footer shows the newline chord once the draft is non-empty.

### `/history`

Open prompt-history search: fuzzy-search this session's prompts newest-first, then press `Enter` or `Tab` to drop a match back into the prompt.

For quick recall, press `↑` on an empty prompt instead. With prompts queued, that moves focus into the queue pane, highlighting the last row; otherwise the panel opens with your most recent prompt already filled in, and `↑`/`↓` step through entries (each lands in the input), `↓` past the newest entry closes the panel, and typing edits the recalled prompt in place.

### `/compact-mode`

Toggle compact display — less padding and tighter spacing for denser output.

### `/density`

Set how much of each tool call the transcript shows. The levels run from most to least detailed:

| Label | Level | Edits | Shell commands | Grouping |
|---|---|---|---|---|
| Detailed | `detailed` | Diff shown | Output preview shown | Only reads, searches, and other tools fold |
| Diff focus | `compact-shells` | Diff shown | One-line row | Only reads, searches, and other tools fold |
| Balanced | `compact-ungrouped` | One-line `+N/-M` row | One-line row | Only reads, searches, and other tools fold |
| Grouped | `compact-grouped` | One-line row | One-line row | Runs of edits, of commands, and of other tools each fold separately |
| Compact (default) | `compact-all-grouped` | One-line row | One-line row | Every run of tool calls folds into one row |

A folded run shows one summary row, such as `Explored 3 files, 2 searches` or `Edited 2 files, ran 1 command +12 -4`; select it and press `Enter` to expand it. Finished thinking inside a run folds with it, and a lone tool call with nothing else to fold keeps its own row. A background command's `Task started` and `Task completed` rows fold wherever shell commands do, into one row even with nothing else beside them, and a group counts each background task once; a monitor keeps both its rows, and a failed or killed task its `Task failed` or `Task killed` row. The `/density` menu lists the labels with the same descriptions as the picker in `/settings` and marks the active one; typing a label or a level name works too, and `verbose` and `minimal` are accepted as aliases for `detailed` and `compact-all-grouped`. The choice is saved to `[ui] tool_call_density` and is also the first row under Appearance in `/settings`.

Until you pick a level, a config from before this setting starts at the closest one: `[ui] collapsed_edit_blocks = true`, or `expanded_by_default = false` under `[scrollback.blocks.edit]` in `pager.toml`, starts at `compact-ungrouped`; otherwise `collapsed_edit_blocks = false`, `expanded_by_default = true` or `[ui] group_tool_verbs = false` starts at `compact-shells`. Grouping itself can't be turned off: reads and searches fold at every level, so `group_tool_verbs = false` gets the level that groups least. `GROK_COLLAPSED_EDIT_BLOCKS` and `GROK_GROUP_TOOL_VERBS` map the same way.

```
/density Diff focus
/density compact-all-grouped
```

### `/vim-mode`

Toggle vim-style keys (`j`/`k`, `h`/`l`, `g`/`G`, `y`/`Y`, and so on) in the scrollback, the dashboard and pickers; `/vim-mode nav` or `/vim-mode off` sets them. With them off (the default), a bare letter or `Shift+letter` in the scrollback just focuses the prompt and types the character. The setting persists to `[keys] vim`.

### `/edit-prompt`

Open an external editor for the prompt, in either render mode. Grok resolves `$VISUAL`, then `$EDITOR`, then `vi`; command values may include quoted arguments. Saving replaces the draft without sending it, and saving an empty file clears it. Typing `/edit-prompt` necessarily replaces the composer's contents, so the editor starts from an empty draft; to edit an **existing** draft, choose **Edit Prompt in External Editor** from the command palette (or press `Ctrl+G` in minimal mode), which preserves the text and refuses pasted, file-reference, or image chips without flattening them.

```
/edit-prompt
```

### `/minimal` and `/fullscreen`

Switch the current session to the other render mode, in place. `/minimal` (offered while you're in fullscreen) switches to the experimental scrollback-native mode; `/fullscreen` (offered while you're in minimal; alias `/full`) switches back to standard fullscreen mode. The switch happens inside the running process — nothing restarts, so a running turn keeps streaming and your composer draft, queued prompts, and permission mode all carry over; a marker (committed line in minimal, toast in fullscreen) reminds you how to switch back. Both are session-scoped — they don't touch `config.toml` — and the `--minimal` / `--fullscreen` CLI flags are session-scoped the same way. To make plain `grok` open in a given mode by default, use `/settings` → **Default screen mode** or set `[ui] screen_mode`. (If the in-place transition misbehaves in an exotic terminal, `GROK_SCREEN_MODE_SWITCH=exec` restores the old behavior of relaunching the pager onto the same session.)

A handful of commands only work in one of the two modes, because the surface they drive doesn't exist in the other: `/find`, `/jump`, `/timeline`, `/theme`, `/tutorial`, and `/dashboard` are fullscreen-only, while `/expand` is minimal-only. (`/workflow runs` is different: it opens the run pane in fullscreen and degrades to a text overview in minimal rather than refusing.) Those are hidden from the command menu and the palette in the mode they can't run in. If you type one out anyway, Grok says why — and points you at whichever is actually useful. When the other mode is the only way to get it, that's the mode switch: `/theme isn't available in minimal mode (minimal renders with your terminal's own palette). Run /fullscreen to switch this session.` When this mode already does the job another way, it names that instead: `/expand isn't available in fullscreen mode: press Tab to focus the scrollback, then → on the block.` Everything else works in both. Note that `--no-alt-screen` still counts as fullscreen here, so it keeps the fullscreen-only commands.

### `/plan`

Enter plan mode.

```
/plan [description]
```

### `/view-plan`

Open a preview of the current saved plan. Aliases: `/show-plan`, `/plan-view`.

---

## Memory

`/flush` and `/dream` require memory enabled through `GROK_MEMORY=1`, `[memory] enabled = true`, `[memory_v2] enabled = true`, or managed remote settings. `/memory` is available whenever a memory store is configured for the session, including when `[memory] enabled = false` turned memory off, so you can browse saved notes and turn memory on for the session from inside the modal. It is hidden only when no memory store is configured, or when `--no-memory` / `GROK_MEMORY=0` turned memory off for the whole process. `/remember` is always offered, but it saves a note only while memory is on for the session.

### `/memory`

Browse, view, and manage saved memories. Alias: `/mem`. Inside the modal, `t`
turns memory on or off for the session, `x` deletes the selected note, and `s`
shows content-free queue, lease, and retention diagnostics.
The `t` toggle is session-scoped: it does not edit `config.toml`, and new
sessions follow the config again. It cannot override `--no-memory` or
`GROK_MEMORY=0`.

```
/memory
```

### `/flush`

Wait until memory capture has saved every finished or stopped turn of this session. Capture normally runs in the background after each turn. Use `/flush` before you close Grok or run `/dream`. The status line shows "Flushing memory…" while it runs, and a scrollback line reports the outcome (for example "Memory flushed through turn 12").

### `/dream`

Run memory consolidation now. Dream folds inbox observations into organized topics. The status line shows "Consolidating memory…" while it runs, and a scrollback line reports what happened: how many observations were merged into how many topics, that there was nothing to consolidate, or that another session is already consolidating.

### `/remember`

Save a note to global memory now. Grok shows the note for review first, and the next Dream folds it into a topic.

```
/remember the staging deploy uses the eu-west cluster
```

---

## Hooks and Plugins

`/hooks`, `/plugins`, `/marketplace`, `/skills`, and `/workflows` all open the same extensions modal, each on its own tab.

### `/hooks`

Open the extensions modal on the Hooks tab, where you can view loaded hooks, add or remove custom ones, and toggle them individually. The modal does not grant project trust — see [10-hooks.md](10-hooks.md) for the trust model.

The shell also advertises individual `/hooks-list`, `/hooks-trust`, `/hooks-add`, `/hooks-remove`, and `/hooks-untrust` commands; in the pager these are folded into the `/hooks` modal.

### `/plugins`

Open the extensions modal on the Plugins tab to view installed plugins, install new ones from the marketplace, and manage trust.

The shell additionally supports subcommands (`/plugins list`, `/plugins install <source>`, `/plugins uninstall <name>`, `/plugins update`, `/plugins reload`). In the pager, the modal does the same work visually.

### `/marketplace`

Open the extensions modal on the Marketplace tab to browse and install plugins.

### `/skills`

Open the extensions modal on the Skills tab to view installed skills.

---

## Media Generation

### `/imagine <description>`

Generate an image from a text description.

```
/imagine a golden sunset over a calm ocean with silhouetted palm trees
```

### `/imagine-video <description>`

Generate a video from a text (or image) description. It plans shots, generates source images, and animates them with `image_to_video`.

```
/imagine-video a cat playing piano in a jazz club
```

---

## Scheduling

### `/loop [interval] <prompt>`

Run a prompt on a recurring interval. Give the interval as `30m`, `1 hour`, or `every 2 days`; leave it out and Grok will ask.

```
/loop 30m check deploy status
/loop check deploy status every hour
```

Intervals are `Ns` (seconds, minimum 60), `Nm` (minutes), `Nh` (hours), or `Nd` (days); anything under 60 seconds is raised to the minimum. Recurring tasks expire after 7 days, and you can cancel one with `scheduler_delete` using the job ID reported when the loop is created.

---

## Workflows and Goals

Workflows work differently on the two routes. `/deep-research` and `/workflows` work on both. The `/workflow` section and the paragraphs marked "On the shell route" describe the shell route, where a workflow is a Rhai script (`.rhai`) and the model's `workflow` tool starts, pauses, resumes and stops runs. On the daemon route a workflow is a JavaScript module, the model starts runs with a background `execute` cell but can't stop them, and no command pauses a run: see [Workflows on the daemon route](#workflows-on-the-daemon-route).

### `/goal`

Set, manage, or check an autonomous goal. Grok works across rounds and only marks the goal complete after an independent evidence review confirms the claim; if that review can't reproduce the result or has no usable evidence, the goal stays active or pauses with concrete gaps.

> **Note:** Goals work best for long-running, hard tasks with measurable success criteria.

Weak:

```text
/goal Migrate the auth module to the new API
```

Strong:

```text
/goal Migrate the auth module to the new API, verified by the auth test suite passing, without changing public endpoints.
```

```
/goal Migrate the auth module to the new API
/goal status
/goal pause
/goal resume
/goal clear
```

Arguments are `<objective> [--budget <tokens>]`, or one of `status`, `pause`, `resume`, `clear`. The `--budget` here is a **token** budget for the goal run, separate from the agent-count budgets that workflows use. `/goal` appears when goal mode is enabled for the session. Which driver runs it depends on background workflows: with them on, the host evaluates each model round and runs adversarial verification on completion candidates; with them off, the legacy model-facing `update_goal` path reports progress and triggers verification.

### `/deep-research <query>`

Kick off a background research workflow. It plans a bounded set of questions, gathers structured claims with source evidence, cross-checks each claim on an independent verifier shard, and renders only the claims that survive, with their verified source locators. Failed shards, dropped claims, and researcher uncertainties are reported as coverage limitations, and the report is marked **Partial** whenever any remain.

```
/deep-research Compare the migration risks of PostgreSQL 17 and MySQL 9
```

The command returns right away — follow progress in `/workflow runs`, and the final report appears in the conversation on its own.

On the shell route, workflows use an absolute cumulative `agent_budget` cap on logical child-agent calls: every `agent()` call and every item in a `parallel()` panel spends one slot, while schema-correction retries don't. The default is 128, explicit values run 1–1,024, and a panel that would cross the remaining budget is rejected before any of its children launch. Model-launched workflows set `agent_budget` on the `workflow` tool; named slash launches accept `--agent-budget N` or an `agent_budget` field in their JSON args. Named launches can also set child reasoning effort with `--effort LEVEL` or JSON `effort`, without changing the current session's `/effort`; a child script's own `effort` option takes precedence. Separately, a host-configured cap (32 by default) bounds how many children run at a time per run; larger panels queue and still act as a barrier. `budget()` reports the cap as `total`, admitted calls as `spent`, `reserved` (always zero), and `remaining`.

### `/workflow`

On the shell route, launch a saved workflow, or manage a running one by its session-unique display name. Launch the same workflow twice and the display names are numbered (`review-changes`, `review-changes-2`); you never need the internal run IDs. Bare `/workflow` prints a text overview of this session's runs.

Type `/workflow` and a space to autocomplete saved workflow names (built-in, project, and user) plus the manage verbs `runs`, `pause`, `resume`, `stop`, and `save`. Picking a name fills it in and offers launch flags before you add args; it does not launch until you press Enter. `pause` / `resume` / `stop` / `save` then list this session's run handles, and with one run to act on Enter sends the command for that run at once (it stops, pauses, resumes or saves that run). A `pause` or `stop` that would list several runs and is not exactly one run's name opens on the text as you typed it, so Enter sends it unchanged instead of picking a run; ↑/↓ picks one. When a match operator in that text (`!x`, `^x`, `x$`) rules out the text itself, the menu closes instead, and Enter still sends the text unchanged.

Shell-route examples. On the daemon route `--effort`, `pause` and `save` fail; see [Workflows on the daemon route](#workflows-on-the-daemon-route).

```
/workflow review-changes --agent-budget 256 --effort high {"target":"origin/main...HEAD"}
/workflow review-changes {"target":"origin/main...HEAD","agent_budget":256,"effort":"high"}
/workflow runs
/workflow pause review-changes
/workflow resume review-changes
/workflow stop review-changes-2
/workflow save review-changes
```

`/workflow runs` opens the live **Workflow Runs** dashboard in the fullscreen TUI — active and retained runs, not a catalog of saved definitions. Each row shows the run's display name, phase, agent roster, progress, and result. In the conversation, **Enter**, **Ctrl+F**, or a double-click on a run's `Workflow` line opens that run's detail view, or, for a run the dashboard no longer lists, a viewer with the run's state. On the shell route, inside a run's detail view, `p` pauses, `r` resumes an ordinary pause, and `x` stops. Budget-limited runs can't bare-resume: `r` returns the shell's rejection (raise the cap with a model/tool resume that passes a higher `agent_budget`), while `x` still stops. `s` saves the run's script, but it's hidden for known built-ins and numbered duplicate handles — for those, choose a new unique `meta.name` and save the edited script explicitly. In minimal mode and non-TUI clients, `/workflow runs` prints the same text overview as bare `/workflow`.

On the shell route, project workflows live in `.grok/workflows/*.rhai`; user workflows live in `~/.grok/workflows/*.rhai`. A same-process pause/resume continues the original immutable script, args, and `agent_budget` cap from committed host-call results — to iterate, edit the returned script copy and launch it as a new run.

On the shell route, a budget-limited run is different: it only resumes through a model/tool resume request that supplies an `agent_budget` above the admitted agent count. A bare `/workflow resume <name>` can't raise the cap, so it rejects budget-limited runs. Runs interrupted by a process restart aren't resumed at all, because external effects have no stable cross-process identity. And resume is not exactly-once: an external effect whose result wasn't committed before a same-process pause can run again.

### `/workflows`

Open the extensions modal on the **Workflows** tab — a browse-only catalog of the saved workflows Grok discovered (built-ins, project `.grok/workflows/`, and user `~/.grok/workflows/`), with each entry's source, description, and path. On the shell route, the same catalog is listed for the model under the skill listing in the session preamble. Launch one with `/workflow <name>` (or its own slash command), then watch it in `/workflow runs`.

### Workflows on the daemon route

On the daemon route a workflow is a JavaScript module. Rhai scripts don't run, and `budget()`, `meta.name`, `/workflow pause`, `/workflow save` and `--effort` don't apply.

**Where workflows live.** A project workflow is `.grok/workflows/<name>.js` at the root of the git repository (outside a repository, in the session folder); Grok reads it only once you trust the folder. Until then the model can't import or start it either: Grok refuses the import and names the folder to trust. Your own workflows are `~/.grok/workflows/<name>.js` (`$GROK_HOME/workflows`). Only `.js` files run. The built-in `deep-research` keeps its name over both. In `/workflow list` and for `/<name>` and `/workflow <name>`, a project workflow keeps its name over one of yours, so give your own and the project's workflows different names. `/workflow list` lists every workflow and marks the ones that can't start, then lists each file it left out and why, such as a broken front matter or code above it. In a folder you have not trusted, it names the project workflows it did not read. A workflow named like a Grok command, such as `doctor` or `release-notes`, starts only with `/workflow <name>`, and `/workflow list` shows it that way.

**Writing one.** The file opens with a front-matter comment, then exports a default async function that takes the run's args and returns its result. Save this as `.grok/workflows/survey.js` and start it with `/survey how errors are reported`:

```js
/**
 * ---
 * name: survey
 * description: Survey a topic in this repository with two agents that read the code and one that merges their notes
 * phase: Explore -- Two agents read the code, one angle each
 * phase: Report -- One agent merges their notes
 * ---
 */

export default async function survey(args) {
  const topic = args?.query ?? "how this project is built and tested";
  const angles = ["where it lives in the code", "how it is tested"];
  const notes = await Promise.all(
    angles.map((angle, i) =>
      host
        .agent(`Read the files in this repository that matter for: ${topic}. Report ${angle}, with file paths.`, {
          label: `Explore/angle-${i + 1}`,
          readonly: true,
        })
        .wait(),
    ),
  );
  const found = notes.filter((note) => note.status === "completed").map((note) => note.text);
  const report = await host
    .agent(`Merge these notes into one short report that keeps the file paths:\n\n${found.join("\n\n---\n\n")}`, {
      label: "Report/merge",
      readonly: true,
    })
    .wait();
  return report.text;
}
```

The front matter:

- The first line is exactly `/**`, with no spaces around it and no byte-order mark before it (only blank lines and a `#!` first line may come before it), every line up to the closing ` */` starts with ` *`, and the keys sit between two `---` lines. Any other line above `/**`, such as `// @ts-check`, `"use strict";` or an `import`, stops the file from loading, and `/workflow list` says why.
- `name` (required): lowercase letters and digits joined by single hyphens, at most 64 bytes, and the same as the file name without `.js`.
- `description` (required, at most 1024 bytes): what `/workflow list` shows, and the run's objective when you start it with no args.
- `when_to_use` (optional, at most 2048 bytes): when the workflow is the right one.
- `phase: <title> -- <detail>` (optional, the detail optional): the phases the run view lists, in order. At most 64 phases, with unique titles.
- Each value is one line and can't contain `*/`. Any other key, or a key other than `phase` given twice, is refused.

The module runs with every permission refused and reaches files, commands and agents only through the global `host`. `host.agent(prompt, options)` starts one agent, and `.wait()` gives its outcome: `status` (`"completed"` when it finished) and `text`, its answer. The options:

- `label`: the agent's name in the run view. `"Phase/name"` puts it under the phase whose title comes before the `/`, so use your `phase:` titles. With no `/`, the agent has no phase.
- `readonly: true`: the agent reads and searches files, but writes nothing and runs no command, `git` included, and anything that would ask you is refused. Leave `readonly` out for an agent that must run a command such as `git diff`: it then asks you for approval like any agent.
- `schema`: a JSON Schema the agent's answer must match. The outcome's `structured` holds the parsed answer, and an answer that doesn't match gets one more turn to fix it.

`Promise.all` over several `host.agent` calls runs those agents at the same time, up to a limit per run; the rest wait for a slot.

**MCP tools.** The model's `execute` cells, and the workflows it starts from them, call the session's MCP tools through `host.mcp`: `host.mcp.listServers()`, `host.mcp.listTools(server)` and `host.mcp.call(server, tool, args)`, with the server names `listServers()` returns. They reach only the MCP tools the turn offers, so under `--disallowed-tools CallMcpTool` each call is refused with `this turn's tools do not include MCP tools`. Each call goes through the same rules, hooks and approval cards as the model's `CallMcpTool`, except that:

- A background run's calls get no Auto-review, so in Auto mode they ask you wherever the classifier would decide. A cell's call during a turn is reviewed as the turn's `CallMcpTool` calls are, but a call Auto-review blocks is refused with its reason, which tells the model how to ask you through `CallMcpTool`.
- When Auto-review checks a turn's `CallMcpTool` calls and the session leaves Auto mode, a background run that turn started has its cells' MCP calls refused for as long as the session stays out of Auto mode.
- `additionalContext` from its `PreToolUse`, `PostToolUse` and `PostToolUseFailure` hooks is dropped.
- During a background run, an MCP server's request for your input is cancelled.
- A run you start with `/<name>` or `/workflow <name>` gets no MCP tools in its cells, though its `host.agent` agents call them under the session's tool options.

When the module's own `host` call needs approval, such as a file write your rules don't allow, your rules and permission mode decide it first, as for any tool call. Otherwise, when no turn is running, Grok asks you on an approval card under the session, labelled `Workflow "<name>" (background run):`. A file write's change shows in the conversation while that card is open, and stays there, marked done, rejected or not run, once it closes; a change too large to preview shows only the file's name, and reopening the session drops it. Rejecting that card or dismissing it with `Ctrl+C` only refuses that call, and text you type while it is open is not sent to the run. A question no one answers within 5 minutes refuses the call. Only the Grok window or `grok -p` run that owns the session on this machine asks; the desktop app and SDK clients refuse a call your rules leave open. While a turn is running, that turn asks instead, as for its own tool calls: the card has no label and shows no file change, rejecting it or dismissing it with `Ctrl+C` ends the turn and refuses the call, and if the turn finishes before you answer, Grok asks again on the run's own labelled card. The run's other kinds of question, such as a web fetch, are refused.

**Starting a run.** Type `/<name> [--agent-budget N] [args]` or `/workflow <name> [--agent-budget N] [args]`, and Grok answers `Started /<name> in the background. Its result appears here when it finishes.` The default export gets `null` with no args, a JSON object as that object, and any other text as `{ "query": text, "objective": text }`. You can also ask Grok to run a workflow. The model has no `workflow` tool on this route: it starts the run with a background `execute` cell that imports the workflow. It can pass any JSON value as the args, but it can't set the agent budget, so its runs get the default budget. Before the run starts, Grok asks you to approve it on a card titled `Workflow: <name>` (`Workflow: background script` when the cell imports no workflow), unless your permission mode or rules allow the `workflow` tool, as `--yolo` does. If you reject the card, the run doesn't start. Hooks see the start as an `execute` call. Workflows start only in Agent mode, not in Plan or Ask mode. In a session on this machine, Grok reads the workflow folders again when each turn ends, so a workflow written or removed during a turn, by the model's file tools, a shell command, an `execute` cell or an agent, is offered as `/<name>` (or no longer offered) once that turn ends. A file you change in your editor between turns is picked up when the next turn ends, or when you open `/workflows`.

**Watching it.** `/workflow runs` opens the run view (in minimal mode it prints text). It lists the phases from the front matter, each pending until an agent's label enters it, with each agent under its phase; Enter on a phase opens an agent's transcript. In the conversation, **Enter**, **Ctrl+F**, or a double-click on a failed run's `Workflow` line that shows why it failed opens the whole failure message instead of the run view. Bare `/workflow` prints this session's runs with their run ids, status, agents used and elapsed time.

**Run names.** Each run has a run id (`wf_…`) and a name: the workflow's name, or `<name>-2`, `<name>-3` and so on when the session already has a run of that name. `/workflow stop` and `/workflow resume` take either. The model has no tool that lists runs. Instead, each message you send tells it this session's runs that are live or recently ended.

**Stopping, and the agent budget.** `/workflow stop <run>` (a run id or name), or `x` in the run view, stops a run. Only you can stop a run: the model has no tool that stops one, so if you ask it to, it gives you the `/workflow stop` command to type. A stopped run ends for good and has no result. You can't pause a run on this route: `/workflow pause` is refused, and the model can't pause one either. Each `host.agent` call spends one agent from the run's budget: 128 by default, or `--agent-budget N` from 1 to 1,024 when you start the run with `/<name>` or `/workflow <name>`. A run that needs more agents than its budget pauses as budget limited. Only you can let it go on, with `/workflow resume <run> --agent-budget N` (N above the current budget, in Agent mode); the model can't resume a run or raise its budget. `/workflow stop <run>` ends it instead. `/workflow stop` also works in Plan or Ask mode. Quitting Grok stops every run that hasn't finished (see [Quitting while workflows run](03-keyboard-shortcuts.md#quitting-while-workflows-run)).

**Results.** The value the default export returns is the run's result. When the run ends, a line in the conversation says it finished, or why it failed, and the model gets the result in a new turn, after any turn in progress, and reports it. The daemon also saves the result as a text file named after the run id and gives the model its path.

**Legacy `.rhai` workflows.** `/workflow list` also lists a `.rhai` file in either folder. `/<name>` or `/workflow <name>` asks the model to convert it into `<name>.js` beside it and start that, while `features.workflow_conversion` is on (the default). `/migrate-workflow <name>` converts one, and starts it only when you pass args. The `.rhai` file stays as it is. If it changes after the conversion, starting the module says so, and `/migrate-workflow <name>` converts it again.

**Headless runs and turning workflows off.** A headless run (`grok -p`) stops its daemon, and with it every run still going, when it exits; [Workflows and Background Work](14-headless-mode.md#workflows-and-background-work) covers how a headless run waits for workflow runs and sets its exit code. `[workflows] enabled = false` or `GROK_WORKFLOWS=0` turns workflows off on this route too.

---

## Other

### `/theme`

Switch the color theme. Alias: `/t`.

### `/feedback [message]`

Report an issue or send feedback. Bare `/feedback` opens the feedback form in every mode, including `--minimal`. Its **Write** tab is a report box: `Enter` sends, `Esc` closes. Its **Drafts** tab (`Ctrl+Tab` switches) holds reports saved for later — failed sends and feedback the agent drafted for you — and `Enter` loads one into Write so you can review, pick a type, and send it. `/feedback <message>` sends the message immediately, in any mode; if the send fails, the message is saved to Drafts.

```
/feedback
/feedback Something isn't working correctly
```

### `/btw`

Send an aside to the agent without interrupting the current task. In minimal mode (`--minimal`), the answer shows up in a dismissible panel above the prompt: `Esc` dismisses it, a finished answer is saved into native scrollback, and a late reply to an already-dismissed panel is dropped. The side question and its answer aren't part of the main turn.

`/btw` can also appear mid-message: the whole message (minus the token) becomes the side question and nothing goes to the main turn. Only `/btw` works this way; other commands must start the message. For a multi-line side question, use `Alt+Enter` (over SSH) or `Shift+Enter`, a trailing `\`, or `/ml`. Do not rely on `Cmd+Enter`: Apple Terminal inserts a newline locally via CoreGraphics; a delivered `SUPER+Enter` (Kitty) also inserts a newline rather than sending; over SSH Cmd never arrives and the chord sends.

```
/btw also check the error handling
fix the retry loop first. /btw what does WBC stand for?
```

### `/mcps`

Open the MCP servers management modal.

### `/doctor`

Check the current session for terminal, clipboard, color, input, notification, and sandbox issues. Doctor shows what it found and how to resolve each issue. Run `/doctor fix` to list available automatic fixes; other findings include manual steps. `/terminal-setup`, `/terminal-check`, and `/terminal-info` remain aliases.

### `/release-notes`

View release notes for the current version. Alias: `/changelog`.

### `/docs`

Browse the built-in How-to Guides, open the online Build docs, or jump straight to a guide by title. Aliases: `/howto`, `/guides`.

```
/docs
/docs web
/docs Getting Started
```

- Bare `/docs` (or `/docs how-to`) opens the How-to Guides picker.
- `/docs web` opens https://docs.x.ai/build/overview in your browser.
- `/docs <title>` opens a specific guide by case-insensitive title match.

### `/tutorial`

Open the onboarding tutorial: a short list of topics (your first prompt, attaching context, navigation, slash commands, worktrees, plan mode, customization, switching from another agent tool) — each a ~30-second read, with `→` flowing straight to the next topic. Nothing auto-shows — this command (or the command palette) is the way in.

```
/tutorial
```

Aliases: `/tour`, `/onboarding`

### `/import-claude`

Open the Claude import modal to bring over `~/.claude` settings: permissions, environment variables, MCP servers, hooks, and paths.

---

## Agents and Personas

### `/config-agents`

Open the agents modal to view and manage agent definitions, set the default, and switch the active one. Alias: `/agents`.

Not the live multi-session [Agent Dashboard](23-dashboard.md) (`/dashboard` / `Ctrl+\`).

### `/personas`

Create, edit, and delete personas. A subagent can apply a persona to shape how it behaves.

---

## Account and Billing

### `/login`

Log in or re-authenticate without leaving the session.

### `/logout`

Log out and return to the login screen.

### `/usage`

View credit usage or manage billing. Alias: `/cost`.

```
/usage
/usage manage
```

Inside a session this opens the usage modal with the account allowance plus that session's context and token totals. From the [Agent Dashboard](23-dashboard.md#dispatch-input) the same modal opens over the dashboard; there is no session there, so only the **Usage limit** tab carries data.

For persisted per-turn token and cost totals of any local session, use `grok usage <session-id> [turn]` from the shell. See [Session Management](17-sessions.md#the-grok-usage-subcommand).

### `/privacy`

Open Settings on **Coding data, retention, and training**, where you choose
**Opt in** or **Opt out**. Takes no arguments.

```
/privacy
```

This setting doesn't touch `[features] telemetry`, `trace_upload`, or your external OTEL settings — see [Monitoring Usage](24-monitoring-usage.md#related-settings). A personal account can opt in or out. A team admin signed in as the team can opt the whole team out but cannot opt it back in, so once the team is opted out the chooser offers only **Opt out**; other team logins see the row locked. Team admins can enable or disable Zero Data Retention for their team ([how to enable ZDR](https://docs.x.ai/developers/faq/security#how-to-enable-zdr)). When the choice isn't yours to make, the row shows `ZDR`, `· Admin Managed`, or `· Personal Account Only` and doesn't open the chooser. ZDR locks coding-data sharing; it does not mute external OTEL or `user.email` — see [ZDR and this stream](24-monitoring-usage.md#zdr-and-this-stream).

---

## Configuration and UI

### `/settings`

Open the settings modal to view and change configuration interactively. Aliases: `/config`, `/preferences`, `/prefs`.

### `/timestamps <off|minimal|on>`

Set which rows show the clock time: `off`, `minimal` (only your prompts and each turn's `Worked for …` line), or `on` (every message, the default). Same as the **Timestamps** row in `/settings`.

---

## Skills as Slash Commands

Any enabled skill with `user-invocable: true` in its SKILL.md frontmatter shows up as a slash command. (Turn a skill off via `/skills` and it stops being advertised.) So a skill at `~/.grok/skills/commit/SKILL.md` runs as:

```
/commit fix typo in README
```

Skills from plugins work the same way. When two skills share a name across scopes, qualify it:

```
/local:commit      # Project-scoped skill
/user:commit       # User-scoped skill
```

Built-in commands always win the bare name. Name a skill "compact" and `/compact` still runs the built-in — the skill stays available as `/local:compact` (or `/acme:compact` for a plugin). Both appear in the slash menu: the built-in is tagged `built-in` and the skill is tagged `skill · local` / `skill · acme`.

---

## Autocomplete

The menu supports fuzzy search: start typing after `/` to filter. Each entry shows the command name, its description, an argument hint when it takes arguments, and its source (builtin, skill scope, or plugin name). Press `Tab` or `Enter` to accept the highlighted command.
