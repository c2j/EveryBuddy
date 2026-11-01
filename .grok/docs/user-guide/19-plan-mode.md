# Plan Mode

Plan mode is a structured planning phase: the agent explores the codebase and designs an implementation approach before writing any code. Use it for tasks with genuine ambiguity about the right approach, where getting your input before coding prevents significant rework.

---

## What Plan Mode Does

When plan mode is active, the agent:

1. Reads and searches the codebase to understand existing patterns and architecture
2. Designs an implementation approach and writes it to the plan file
3. May use `ask_user_question` to clarify specific questions
4. Calls `exit_plan_mode` to present the plan for your approval

For the main agent, a headless `--permission-mode plan` run holds plan and rejects every edit except the plan file, on the native route and the daemon route. The tool call fails with a short message that only the plan file can be edited. The native route names that file's path. Interactive plan mode (no hold) does the same on the grok toolset. On the compatibility toolset (its write and string-replace tools) and the daemon route it allows any markdown edit and rejects the rest. This holds in every permission mode, including always-approve. Separating planning from implementation lets you review and correct the approach before any code is written.

On the daemon route, how a headless run treats a kept plan is described in [Kept plans on the daemon route](14-headless-mode.md#kept-plans-on-the-daemon-route).

---

## How to Enter Plan Mode

### Agent-Initiated Entry

The agent enters plan mode when it determines a task has genuine ambiguity. It calls the `enter_plan_mode` tool, which requires your approval before plan mode activates. If you decline, the agent stays in normal mode.

**Good triggers for plan mode:**

- "Add user authentication to the app" -- genuinely ambiguous (session vs JWT, token storage, middleware structure)
- "Redesign the data pipeline" -- major restructuring where the wrong approach wastes significant effort
- "Add caching to the API" -- multiple reasonable approaches (Redis vs in-memory vs file-based)
- "Add real-time updates" -- architectural decision (WebSockets vs SSE vs polling)

**Not appropriate for plan mode:**

- "Add a delete button to the user profile" -- clear implementation path
- "Fix the typo in the README" -- straightforward
- "Update the error handling in the API" -- start working, ask specific questions if needed
- "Can we work on the search feature?" -- user wants to get started, not plan

### User-Initiated Entry

You can enter plan mode yourself in three ways:

- **`--permission-mode plan`** -- The flag applies to the TUI and to `grok -p`. `grok agent stdio` and `grok agent serve` ignore `plan`. The welcome session, `/new`, a fresh worktree, a cloud session, the settings placeholder, a named id (`--session-id`, or a worktree created with a preferred session id), a resume that falls back to a fresh session, and a foreign pick start in plan only when plan can start (plan tools on, not a chat, and for a new id no connected `/remote` computer), and start in Ask otherwise, with always-approve and auto off. `--chat` does not create the welcome session. A named id stays on this machine. A dashboard spawn follows its mode chip. A Plan chip that cannot start becomes Ask, and that agent starts in Ask. A TUI resume or fork keeps the saved mode. A replacement of a session that is in plan starts in plan when plan can start, and in Ask when it can't. Any other replacement keeps the mirror and does not start in plan. A session bound to a remote machine does not start in plan mode. Under the flag, a `/new` on a remote tab and a session opened on the connected computer start in Ask. An empty session that is in plan is remade in plan when plan can start, and in Ask when it can't. Only a session that is not in plan keeps an explicitly picked Always-approve or Auto. Moving onto a computer starts any other empty session in Ask, and moving back off starts it in plan when plan can start and in Ask when it can't. Headless sets plan mode on the session it opens when that run has a prompt, including a resume or a fork, and a headless `exit_plan_mode` is not approved, so plan holds for the run. A memory flush with no prompt sends no hold, and a resumed plan approval stays parked. On the daemon, `-r <id> --fork-session` forks and plan holds for that run. A fork by title or `--continue` is refused (`the daemon forks a session only by session id`), and `--session-id` together with `--fork-session` is refused.
- **`/plan`** -- Enter plan mode. When the session is idle, plan mode waits for your next prompt. When a turn is running, plan mode activates immediately. Run `/plan <description>` to enter plan mode and start a turn with that description in one step.
- **Shift+Tab** -- Cycle the session mode: Normal, then Plan, then Auto-review, then Always-approve, then Normal. The cycle skips Auto-review when the auto-mode gate is off. From Normal, a single press lands on Plan.

After a plan exists, run **`/view-plan`** (aliases `/show-plan`, `/plan-view`) to reopen its saved preview.

---

## The Plan File

The plan is written to `plan.md` inside the session directory (`~/.grok/sessions/<cwd>/<session-id>/plan.md`, where `<cwd>` is an encoded directory name, not the literal path).

The plan file contains:

- A **Context** section explaining why the change is being made
- The recommended approach (not every alternative)
- The paths of critical files to modify
- Existing functions and utilities to reuse, with their file paths
- A verification section describing how to test the changes end to end

---

## Plan Approval

When the agent finishes planning, it calls the `exit_plan_mode` tool. The tool reads the plan file from disk, and the TUI opens a scrollable preview of the plan with an action bar along the bottom.

If the agent exits without writing a plan (empty or missing `plan.md`), the same approval surface still opens with a clear empty-state message so you can approve and start implementing, request changes (send the agent back to planning), or quit. In minimal mode the empty notice is committed into scrollback and the controls strip header reads **No plan written yet**.

### Reviewing the Plan

Scroll the plan with the arrow keys or `j`/`k`. The action bar shows these shortcuts:

| Shortcut | Action                                                                                               |
| -------- | ---------------------------------------------------------------------------------------------------- |
| `a`      | Approve the plan and start building. With pending comments, this reads `approve w/ comments` and sends them alongside the approval. |
| `s`      | Request changes. Focus moves to the prompt so you can type revision notes; press `Enter` to send them. |
| `c`      | Comment on the selected line or line range.                                                          |
| `y`      | Copy the full plan to the clipboard.                                                                 |
| `q`      | Quit plan -- abandon the plan without approving and turn plan mode off.                              |

Press `Tab` to move focus between the plan preview and the prompt.

When the review opens while you are typing and no permission prompt is waiting, focus starts in the prompt. There, `a` types a letter until you leave. Press `Tab`, `Shift+Tab` or `Esc` to move to the preview, then press `a` to approve. An open `/feedback` form, an edited queued prompt, or the block viewer's search, filter, or goto bar counts as typing. While the tasks pane or dock holds the preview closed, focus stays where it was.

The review after a turn waits while `/feedback`, a question, an elicitation, or a queued-prompt edit is open. A running command such as `/compact` holds it until the command ends. A model turn that starts keeps the wait, and the review opens when that turn ends, on the preview if the composer is empty. Closing `/feedback` or a queued prompt edit while a turn runs works the same way. The review that opens when the plan is written (`exit_plan_mode`) never waits. A permission card holds a review that was already waiting. It does not stop a review from opening underneath it. A prompt put back in the composer, because the agent never acknowledged it, holds the plan review while that prompt sits in the composer, a question, an elicitation, a permission card, an open inline comment, or a queued-prompt edit. The hold ends once the composer and those stashes are empty, after a send or a clear. A review that becomes due while that prompt is still there waits the same way. Clearing the composer counts as typing, so the review opens in the notes. A `/remember` note stays open over a review the agent opened with `exit_plan_mode`, and over the review that opens after the turn.

While the plan approval view is open, `Ctrl+P` (command palette → model) still works for switching model before you press `a` to approve.

### Providing Feedback

The approval view has three focus states:

- **Preview**: Scroll the plan and select lines to comment on.
- **Commenting**: Add an inline comment to the selected line range (press `c`, or `Enter` on a line).
- **Prompt**: Type freeform revision notes.

Press `Tab` to switch between the preview and the prompt. When you send feedback -- inline comments, freeform notes, or both -- the agent receives it and revises the plan. Plan mode stays active so you can iterate. A complete pager command typed in the prompt (for example `/feedback <text>` or `/compact`) runs as a command instead of being sent as notes; the review stays open. Pressing `a` while such a command sits in the prompt is refused until you run it with Enter or delete it.

### Leaving the Approval View

Press `Esc` to return focus from the prompt to the plan preview. To dismiss the approval without approving or sending feedback, press `q` to quit the plan. Quitting abandons the proposed plan and turns plan mode off.

---

## Plan Mode Lifecycle

The plan mode state machine has four states:

| State          | Description                                                    |
| -------------- | -------------------------------------------------------------- |
| `Inactive`     | Normal operating mode. No plan mode constraints.               |
| `Pending`      | Client toggled plan mode ON, but no prompt has been sent yet.  |
| `Active`       | Plan mode is active for the main agent. A headless hold allows only the plan file. Interactive plan mode allows only the plan file on the grok toolset, and any markdown file on the compatibility toolset and the daemon route. |
| `ExitPending`  | User toggled plan mode OFF while a turn is in-flight.          |

Transitions:

```
Inactive    --> Active   (enter_plan_mode tool called and approved -- skips Pending)
Inactive    --> Pending  (you toggle plan mode on while the session is idle)
Inactive    --> Active   (you toggle plan mode on while a turn is running)
Pending     --> Active   (your first prompt activates plan mode)
Active      --> Inactive (exit_plan_mode approved, or you toggle plan mode off when idle)
Active      --> ExitPending (you toggle plan mode off while a turn is in-flight)
ExitPending --> Inactive (after the turn completes)
```

Plan mode state is persisted to disk and survives process restarts. Transient states (`Pending`, `ExitPending`) are collapsed to `Inactive` on restart since they depend on in-flight interactions.

---

## Edits During Plan Mode

During active plan mode, edits to the plan file are auto-approved without prompting, so the agent can iterate on the plan freely. Under a headless plan hold, the main agent's edits to any other file are rejected on both routes, with a short message that only the plan file can be edited. The native route names that file's path. Interactive plan mode rejects non-plan-file edits on the grok toolset, and rejects non-markdown edits on the compatibility toolset and the daemon route.

This enforcement is independent of the permission mode:

- **Always-approve (yolo) stays armed underneath plan mode.** Non-edit tools (bash commands, reads, MCP tools) still auto-run, but the main agent's file edits are blocked until you approve exiting plan mode. Once the plan is approved, always-approve resumes for implementation.
- Bash commands are not inspected for file writes — plan mode blocks the edit tools, not shell redirection.
- Subagents are not covered by the parent session's plan-mode edit gate. Each subagent starts with a fresh plan-mode tracker (`Inactive`), so a `general-purpose` (or other write-capable) subagent can edit files while the parent is still in plan mode — and it inherits the parent's permission mode (including always-approve). Read-only types such as `explore` remain limited by their own toolset. A subagent started with Task follows the allow rules. On the native route the `workflow` tool is not gated in plan mode. The daemon route refuses a workflow in Plan.

The status flag shows `plan` while plan mode is active. If always-approve is enabled underneath, its flag reappears when plan mode exits.

---

## Plan Mode and Compaction

When `/compact` runs during an active plan mode session, the plan mode state is preserved. The compacted context includes a reminder that plan mode is active, so the agent continues planning after compaction.

---

## When Plan Mode is Appropriate

**Use plan mode for:**

- Tasks with significant architectural ambiguity (multiple reasonable approaches)
- Unclear requirements that need exploration before implementation
- High-impact restructuring where the wrong approach wastes significant effort

**Skip plan mode for:**

- Tasks with a clear implementation path
- Bug fixes where the fix is obvious once you understand the bug
- Adding features that follow existing conventions
- Straightforward modifications (renaming, formatting, adding tests)
- Research and exploration tasks (use subagents instead)
