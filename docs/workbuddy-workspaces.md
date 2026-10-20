# WorkBuddy workspaces → EvoBuddy task workspaces

## What WorkBuddy does (observed)

Target: `WorkBuddy AI` 5.8.4, Electron 37.10.3, bundle
`/Applications/WorkBuddy AI.app`.

REA evidence:

- `list_electron_targets` — `ev_96827bbdc37e284091b7d516f86587e5607f98ae83f6314e9c68876fa53167df`
- `inspect_electron_page` — `ev_af65973b54d901ef6c70568b9001236676a494da457f109d417e4185fd35401b`
- live window observation — `ev_c582105691b035754ba0786d686f9e734ebb6d663cf4c1e4d227faec95c8021c`
- workspace module analysis — `ev_98f389ac9f08b3c7c9dc88148c798676eeab7487596890c838a42af3f3e4eea0`

The live UI has a sidebar section:

```text
Workspaces (1)
  evobuddy-m21-work...
```

and the selected task is associated with that workspace. The renderer contains
`workspace-preparing-*.js` (a “Preparing your workspace” progress component) and
`keep-workspace-dialog-*.js` (name/cancel/save dialog when keeping a workspace).

The app's filesystem also contains session-scoped workspace directories:

```text
~/.workbuddy-ai/workspace/sessions/<session-id>/
  .modify_backup_meta/
  excluded_backup_roots.json
  snapfile/
```

So the observed model is: an agent task runs in a dedicated workspace, not the
user's project root; workspace lifecycle is separate from the task transcript.

## What EvoBuddy implements

- Workspaces are first-class entities with `id`, `name`, `root`, timestamps and
  `Temporary`/`Kept` state.
- `Task.workspace_id` references a workspace; the path is never a grouping key.
- `~/.evobuddy/workspaces.json` stores the entities and `tasks.json` stores
  `workspace_id`.
- Every first prompt creates a Temporary workspace:

```text
$HOME/.evobuddy/workspaces/<task-slug>-<timestamp>/
```

- The ACP answer source receives the resolved workspace root and launches or
  relaunches the ACP agent with that `cwd` (and sends it in `session/new`).
- The sidebar shows `Tasks` for unassigned tasks and `Workspaces` with workspace
  headers and their child tasks.
- The workspace picker lists real workspaces in both the landing footer and the
  chat toolbar. It can assign/unassign the current task, create a Temporary
  workspace, or explicitly leave a new task unassigned.
- A workspace header menu provides New task here, Rename/Keep, and Delete.
- Deleting a workspace confirms first, unassigns its tasks, and removes its
  managed root.
- Empty Temporary workspaces are cleaned at startup and after detaching a task.
- Legacy `workspace` paths migrate to `Kept` workspaces; existing tasks reopen
  the same root after restart.

Example verified from a fake ACP run:

```text
acp: initialize cwd=/Users/c2j/.evobuddy/workspaces/hi-1792448780285
```

```text
Workspaces (1)
hi-1792448780285
```

![EvoBuddy task workspace](/tmp/workspace-evobuddy.png)

## Differences / remaining work

- WorkBuddy's "keep workspace" naming dialog is not implemented; EvoBuddy creates
  a deterministic workspace automatically and persists it.
- WorkBuddy keeps backup metadata/snapshots inside the session workspace;
  EvoBuddy currently creates the directory only.
- WorkBuddy can have temporary/kept workspace semantics; EvoBuddy currently keeps
  every task workspace.
