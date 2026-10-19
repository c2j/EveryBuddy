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
  - 2 built-in skills (`code-review`, `release-notes`)
  - user skills under `~/.evobuddy/skills/<name>/SKILL.md`
  - 3 expert profiles
  - connectors discovered from `~/.evobuddy/connectors/*.json`
  - scheduled queue in `~/.evobuddy/scheduled.json`
- `Experts · Skills · Connectors` screen lists experts, skills, and connectors;
  skills have Enable/Disable, experts have Use/Clear.
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

![EvoBuddy catalog](/tmp/catalog.png)

Example scheduled run observed in the log:

```text
scheduler: running 'Scheduled hello'
```

and the task state was written with a new workspace:

```json
{
  "session_id": "task-0-1792450448474",
  "title": "Say scheduled hello",
  "workspace": "/Users/c2j/.evobuddy/workspaces/say-scheduled-hello-1792450448473"
}
```

## Limits

- Connectors are listed/configured only; no MCP server is launched yet.
- Skill enablement affects the prompt, not ACP's native skill resolution.
- Experts are prompt profiles, not separately installed agent packages.
- Scheduled jobs must be added to the JSON file; there is no creation dialog yet.
