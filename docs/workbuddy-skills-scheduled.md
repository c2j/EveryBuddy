# WorkBuddy skills / connectors / schedule → EvoBuddy

## WorkBuddy evidence

Live window: WorkBuddy AI 5.8.4 (Electron 37.10.3), evidence
`ev_c582105691b035754ba0786d686f9e734ebb6d663cf4c1e4d227faec95c8021c`.
The sidebar exposes `Experts · Skills · Connectors` and `Scheduled`.

On disk, WorkBuddy keeps a materialized skill cache:

```text
~/.workbuddy-ai/.skill-list-cache.json
```

It contains 47 entries with `name`, `filePath`, `description`, `source`,
`type`, `disable`, `disableModelInvocation`, `slug`. A sample entry is
`expert-manager` / `source=builtin` / `type=prompt`, whose `filePath` points to:

```text
.../resources/plugins/workbuddy-builtin/skills/expert-manager/SKILL.md
```

Connectors have their own state at:

```text
~/.workbuddy-ai/connectors/<account>/connector-states.json
~/.workbuddy-ai/connectors/enterprise-registry.json
```

So the observed model is: skills are filesystem instructions with cached
enablement/metadata; experts are a skill/profile layer; connectors are separately
configured integrations.

## EvoBuddy implementation

- `crates/eb/src/catalog.rs`
  - built-in `assets/skills/*/SKILL.md`
  - user skills under `~/.evobuddy/skills/<name>/SKILL.md`
  - YAML frontmatter (`name`, `description`, `triggers`, `version`), body
    preview, duplicate/empty/malformed errors shown in the UI
  - 3 expert profiles
  - connectors discovered from `~/.evobuddy/connectors/*.json`
  - scheduled queue in `~/.evobuddy/scheduled.json`
- `Experts · Skills · Connectors` has tabbed Experts / Skills / Connectors views,
  search, All/Built-in/User/Enabled filters, Installed count, Add Skill, and
  Featured/Recommended card grids. Skills have +/✓ install toggles, Details, and
  visible parse errors; experts have Use/Clear. Enabled skill text is budgeted into
  the ACP prompt and invalid skills are skipped.
- Enabled skills and the active expert are prepended to the ACP prompt.
- A 5-second scheduler claims due jobs and submits them through the normal
  `Runner::submit` path, so they receive a task, workspace, dsh session, and ACP
  turn just like typed prompts.

Example catalog:

```text
Experts
  Code reviewer — Prioritise correctness, regressions, and missing tests. [Use]
Skills · 0/2 active
  code-review — Review a change... [Enable]
Connectors
  local files — builtin workspace access
  shell — agent exec via ACP
```

![EvoBuddy skills surface](/tmp/skillui4.png)

Clicking `Details` opens a market-style detail sheet with version, usage/install
placeholders, example prompts (Copy), skill ID, Install toggle, and full
instructions.

![Skill detail](/tmp/skill-detail-final.png)

Expert cards also have `Details`, opening a sheet with role, tags, version,
usage, three example prompts (Copy), Summon, and a stable `ex_` ID.

![Expert detail](/tmp/expert-detail-career.png)

Example scheduled run observed in the log:

```text
scheduler: running 'Scheduled hello'
```

and the Scheduled queue UI shows due jobs with Run now/Delete:

![Scheduled queue](/tmp/scheduled-polish2.png)

The task state was written with a new workspace:

```json
{
  "session_id": "task-0-1792450448474",
  "title": "Say scheduled hello",
  "workspace": "/Users/c2j/.evobuddy/workspaces/say-scheduled-hello-1792450448473"
}
```

## Connector wiring

`~/.evobuddy/connectors/*.json` is translated into ACP `mcpServers` at
`session/new`. Both a single server object and a Claude-style
`{"mcpServers": {...}}` wrapper are accepted; stdio uses
`command`/`args`/`env`, remote uses `url`/`headers`.

The Connectors screen also validates config (`args` array, `env` object, http(s)
url, missing transport) and exposes a `Test` button. The test performs an MCP
`initialize` handshake: stdio by spawning the command, HTTP by a bounded curl
POST. Results are shown on the card.

Verified with a fake ACP server:

```text
acp: session/new with 1 MCP server(s)
```

and with an MCP fixture:

```text
MCP initialize ok
```

## Limits

- An MCP server is launched by the ACP agent, not by EvoBuddy; EvoBuddy only
  supplies the configuration.
- Skill enablement affects the prompt, not ACP's native skill resolution.
- Experts are prompt profiles, not separately installed agent packages.
- Scheduled creation is a one-minute quick-add; there is no full cron editor.
