# Workspace model

Status: accepted; model and UI implemented for v0.7.0.

## Why this exists

EvoBuddy currently has one field:

```rust
Task.workspace: Option<String>
```

It is used for three unrelated things:

1. the agent's execution directory;
2. the sidebar's Workspaces grouping;
3. a stable identity across restarts.

That conflates a **sandbox** with a **workspace**. It causes concrete bugs:

- every task gets an auto-generated path like `hi-1792464933188`, which is not a
  user-facing workspace;
- grouping by path makes `Tasks` empty and `Workspaces` contain raw paths;
- there is no owner, name, lifecycle, or kept/temporary state;
- two tasks cannot intentionally share a workspace;
- deleting/renaming a workspace has no representation.

Workspace must become a first-class object before more workspace UI is built.

## Definition

A **Workspace** is a named, persistent working environment that owns zero or
more tasks and provides the filesystem root in which their agents run.

It is **not**:

- the user's original project directory (agents never run there directly);
- a per-turn temporary directory;
- the task transcript.

The agent root may be a copy, restore, or session directory inside the
workspace, but the workspace is the stable identity.

## Data model

```rust
pub struct Workspace {
    pub id: String,          // stable, e.g. ws-01J...
    pub name: String,        // display name, e.g. "EvoBuddy M21"
    pub root: String,        // absolute path; implementation detail
    pub created_at: u64,     // Unix seconds
    pub updated_at: u64,
    pub state: WorkspaceState,
}

pub enum WorkspaceState {
    Temporary,               // auto-created for one task, eligible for cleanup
    Kept,                    // user named/kept it
}
```

Task reference:

```rust
pub struct Task {
    // ...
    pub workspace_id: Option<String>, // None = unassigned, shown under Tasks
}
```

Rules:

- `workspace_id` is the only grouping identity. Paths are never grouping keys.
- A workspace may contain zero tasks (kept, currently empty).
- A task has at most one workspace.
- A workspace may have many tasks; the UI renders them as child rows.
- `Workspace.root` is resolved through `WorkspaceManager`; callers never join
  paths themselves.
- A task's agent always runs with `cwd = workspace.root`; if no workspace is
  assigned, no agent is started until one is created or selected.

This is intentionally a **shared workspace** model. WorkBuddy's
`workspace-preparing` and `keep-workspace-dialog` behaviour, plus a workspace
row containing a child task, fit this model. If product requirements later need
strict one-workspace-per-task, that is a lifecycle policy, not a schema change.

## Persistence

Split the current `tasks.json` into two files:

```text
~/.evobuddy/workspaces.json
~/.evobuddy/tasks.json
```

`workspaces.json`:

```json
[
  {
    "id": "ws-1792464933188",
    "name": "EvoBuddy M21",
    "root": "/Users/c2j/.evobuddy/workspaces/evobuddy-m21-1792464933188",
    "created_at": 1792464933,
    "updated_at": 1792464933,
    "state": "kept"
  }
]
```

`tasks.json` stores `workspace_id`, never a group path:

```json
{
  "title": "Create Fibonacci number Python script",
  "workspace_id": "ws-1792464933188",
  "turns": [],
  "updated_at": 1792464933
}
```

### Migration from the current file

For each legacy task with `workspace: "..."`:

1. Create one `Workspace` per unique path, using the path basename as the
   initial name and the task timestamp for `created_at`.
2. Set `Workspace.state = Kept`, because the user already has the directory.
3. Set `Task.workspace_id` to that workspace id.
4. Keep the legacy `workspace` key readable for one release, but stop writing it.

If migration is not possible, the task stays unassigned rather than silently
losing its root.

## Workspace lifecycle

```text
new task
  -> create Temporary workspace + root
  -> agent runs with cwd = root
  -> user keeps it: rename, state = Kept
  -> cleanup only removes Temporary workspaces with no running tasks
```

A task may also be created unassigned:

```text
new task
  -> workspace_id = None
  -> Tasks section, no agent until a workspace is selected
```

The product should choose one default. Recommendation: create a Temporary
workspace on the first prompt for parity with WorkBuddy's
"Preparing your workspace"; expose "Keep workspace" afterwards.

## UI invariants

Sidebar:

```text
Tasks (n)              tasks with workspace_id == None
Workspaces (m)         every Workspace entity
  📁 name              workspace row
     child task        tasks with workspace_id == this id
```

- Counts come from entities, not path uniqueness.
- A workspace row is rendered even when it has no tasks.
- A task appears exactly once: under Tasks or under its workspace.
- Dates are `Task.updated_at`; workspace rows use `Workspace.updated_at`.
- The composer's workspace picker lists `No workspace`, workspace names, and
  `Add workspace…`. Selecting a name assigns that workspace to the current task.

## Agent and filesystem boundary

- `WorkspaceManager::resolve(task)` is the only place that maps
  `workspace_id -> root`.
- ACP `session/new` receives the resolved root.
- dsh/agent session metadata records both `workspace_id` and resolved root for
  audit, but only `workspace_id` is durable.
- Tool output paths remain workspace-relative where possible; absolute paths are
  display-only.

## Migration plan

1. Add `Workspace`, `WorkspaceState`, `Task.workspace_id`, `App.workspaces`.
2. Add `WorkspaceManager` and split persistence; migrate legacy paths.
3. Change sidebar grouping to workspace ids; render Tasks and children.
4. Make the composer picker dynamic; add workspace creation/keep dialog.
5. Remove `Task.workspace` after one migration release.

Each step must preserve the current task transcripts and must not rewrite an
existing workspace root.

## Decisions (confirmed)

1. **Sharing**: yes. Multiple tasks may share one workspace.
2. **Default**: yes. The first prompt creates a Temporary workspace.
3. **Keep flow**: Task topbar menu, agreed. UI is not implemented yet.
4. **Cleanup**: implemented as: remove a Temporary workspace when it has no
   tasks, at app startup and after a task is detached. Kept workspaces, and
   Temporary workspaces still referenced by a task, are never removed.

## Implemented in v0.6.0

- `Workspace`, `WorkspaceState`, `Task.workspace_id`, `App.workspaces`
- `workspaces.json` plus `tasks.json` migration
- `WorkspaceManager` (`workspace::create`, `adopt`, `find`, `display_name`)
- Runner resolves the workspace and passes its root to ACP
- Sidebar Tasks/Workspaces grouping by id, child task rows
- Dynamic workspace picker, including "Add workspace…" and assigning the
  selected workspace to a new task
- Existing paths migrate to `Kept` workspaces; new tasks create `Temporary`
  workspaces

Still open:

- Keep/rename dialog: implemented from the workspace header menu.
- Workspace deletion: implemented with confirmation; tasks become unassigned.
- Temporary cleanup: empty Temporary workspaces are removed at startup and
  when a task is detached.
- Workspace picker: present in both the landing footer and chat toolbar.
- Explicit unassigned entry: `New unassigned task` under Tasks, and the
  `No workspace` picker choice.
