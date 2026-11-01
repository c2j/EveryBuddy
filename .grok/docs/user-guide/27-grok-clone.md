# grok clone

`grok clone` fetches a Git repository into a Grove content store and mounts a
projected working tree (NFS on macOS, FUSE on Linux). Each invocation reads
`GROK_CLONE` / `GROVE_CLONE` in this process, then grok enable-all
(`GROK_GROVE` or `[cli] grove` in `~/.grok/config.toml`), then `[clone] enabled`
in Grove config (`~/.config/grove/config.toml`). The Grove daemon does not
authorize Clone IPC.

This does **not** enable Grove for session / `-w` worktrees. Those use a
separate gate (`GROK_WORKTREE_TYPE` and `[cli] grove_worktree` in
`~/.grok/config.toml`; see [Configuration reference](26-config-reference.md)).
`GROK_WORKTREE_TYPE` and `[cli] grove_worktree` do **not** enable `grok clone`.
`GROK_CLONE` / `GROVE_CLONE` / `[clone] enabled` do **not** enable session /
`-w` Grove for other checkouts. A worktree created from inside a Grove clone
uses Grove without any flag; `GROK_WORKTREE_TYPE=copy` or
`[cli] grove_worktree = false` keeps it on copy.

To turn **both** surfaces on without touching the specific knobs:

```bash
export GROK_GROVE=1
# or in ~/.grok/config.toml:
# [cli]
# grove = true
```

Specific knobs still win: `GROK_WORKTREE_TYPE=copy` keeps session worktrees on
copy while clone can stay on; `GROK_CLONE=0` keeps `grok clone` off while
worktrees can stay on.

```bash
grok clone <url> [dir] [--branch NAME] [--full-history]
```

## History

**Default is a depth-1 bootstrap** of the selected branch (`blob:none` +
`--depth=1`). Only that branch is advertised as a remote-tracking ref.

Use `--full-history` when you need complete commit history, tags, or every
remote branch at clone time (the previous default).

After a depth-1 clone, these commands deepen **only the selected branch**:

```bash
git fetch --deepen=N origin
git fetch --unshallow origin
```

Fetching another branch needs an explicit depth-limited refspec. An ordinary
`git fetch origin` or `git fetch origin other` will not pull that branch's
full history through the default refspec:

```bash
git fetch --depth=1 origin refs/heads/NAME:refs/remotes/origin/NAME
```

A default shallow clone requires a Grove daemon that understands the
`clone_shallow` RPC. If the client refuses, restart or update the daemon (or
pass `--full-history`):

```bash
grove daemon --foreground
```

On macOS you can also install a KeepAlive agent:

```bash
grove doctor --install-agent
```

## Authentication

Git credentials for a clone belong to the **Grove daemon**, not to `grok login`.
The two are separate worlds:

| World | Covers | Commands | Store |
|-------|--------|----------|-------|
| Grok | the model and API | `grok login`, `grok logout` | `~/.grok/auth.json` |
| Grove Git | the remote this clone fetches from | `grove status`, `grove reload-credentials` | the daemon's credential cell, built from `auth_mode` in Grove config (`git credential` helper, a carrier token file, or `GROVE_AUTH_TOKEN`) |

`grok clone` never reads `~/.grok/auth.json` for Git. Signing into Grok does not
give the daemon a credential for the remote, and neither does
`[clone] enabled = true`: that flag is a **product gate** deciding whether
`grok clone` runs at all, not authorization for GitHub.

When Grove classifies a failure as a credential problem, the clone prints the
class and the commands that own it, without the remote URL:

```
Grove Git credentials rejected (unavailable).
Grove Git credentials belong to the Grove daemon, not `grok login`.
Check `grove status` then `grove reload-credentials`.
```

| Class | Meaning | Next step |
|-------|---------|-----------|
| `unavailable` | the daemon had no usable credential, or the remote rejected it | `grove status --json \| jq .auth.live_provider` names the live provider; fix that source, then `grove reload-credentials` |
| `expired-static` | the token expired and this deployment does not refresh tokens | start a new session, or enable `GROVE_TOKEN_ROTATION=expected` |
| `carrier-stale` | the daemon waited for a carrier rewrite and still holds the rejected token | wait for the rewrite, or `grove reload-credentials` |
| `other` | the credential provider failed for another reason | `grove status`, then `grove reload-credentials` |

Only `expired-static` and `carrier-stale` add their own line to the message. The
advice for `unavailable` depends on which provider the daemon holds, and the
clone never reads that, so it stays on `grove status` and `grove doctor`.

One credential rejection stays unclassified: when a token cannot see a private
repository, GitHub answers as if the repository did not exist. That is
indistinguishable from a typo in a public URL, so the clone reports it as a
missing repository rather than guessing at credentials.

`grove status` prints the daemon's credential health on its first line, even
with no mounts, and lists a degraded or unavailable cell under ATTENTION with
its hint, and the command that loads the fixed credentials as its fix:

```
grove 0.1.0 · daemon up 12m46s, pid 1846444 · auth degraded
...
ATTENTION
  warn  daemon  Git credentials degraded (carrier-stale): wait for carrier rewrite, or grove reload-credentials
                > grove reload-credentials
```

`grove status --json | jq .auth` prints the whole block:

```json
{
  "configured_mode": "auto",
  "live_provider": "git-delegate",
  "health": "ok",
  "reload": "supported",
  "hint": "credentials look healthy",
  "summary": "mode=auto live=git-delegate health=ok last=- reload=supported"
}
```

`configured_mode` is the configured `auth_mode`; `live_provider` is the provider
the daemon actually holds. They can disagree — that is the case
`grove reload-credentials` exists to fix. For example, with `auth_mode = auto`,
a daemon that started before a token file was written stays on `git-delegate`
until the cell is rebuilt.

```bash
grove reload-credentials
# grove: credentials reloaded → token-file
```

Reload rebuilds the cell from env and disk and prints the provider it landed on;
it prints no secrets. It cannot create a login: with `auto` or `git` and no
carrier token, configure `git credential` or `gh auth` first, then reload.

`grove doctor` reports the same fields as one finding: `auth.ok`,
`auth.degraded`, `auth.unavailable`, `auth.daemon-down`, or `auth.old-daemon`.

## Daemon

`grok clone` uses a live Grove daemon. If the control socket is down, it starts
`grove daemon --foreground` as a detached process (so exiting or Ctrl-C on
`grok` does not take the daemon or its mounts down) and waits for the socket.

The `grove` binary is resolved from `PATH`, then from the directory of the
`grok` executable (for example `~/.grok/bin/grove` next to `grok`). There is
no separate install location. macOS has no PATH package for grove; build it
from the monorepo:

```bash
cargo build -p grove --release
```

On Linux, clone needs a usable FUSE before it starts a daemon: `/dev/fuse` must
exist, and either this user can open it or a setuid `fusermount3` / `fusermount`
helper is on `PATH` (Grove mounts through either route). Missing FUSE is a hard
error with install commands, not a hang. When a daemon is already running the
check is skipped, since that daemon may hold privileges this process does not.

On Windows, clone mounts through ProjFS (Windows Projected File System),
which needs the `Client-ProjFS` optional feature; `grove doctor` reports
whether it is enabled. Build-output directories such as `target` and
`node_modules` are redirected out of the projected tree as NTFS junctions
(`[redirects] windows_junction`, on by default).

## Managing clones

grok keeps no record of a clone. The Grove daemon that mounted it owns it, and
the worktree commands ask every running daemon for its clones:

```bash
grok worktree list                 # registry worktrees, then unfinished copies, then a Grove clones table
grok worktree list --type clone    # clones only (add --json for machine output)
grok worktree show <path-or-store-id>
grok du                            # clones get their own section, outside the total
grok worktree rm <path-or-store-id>
grok worktree rm --keep-store <path-or-store-id>
grok worktree rm --dry-run <path-or-store-id>   # says what would happen to the store
```

Each clone row names the daemon that owns it, its state (`mounted`,
`unmounted`, or `orphaned store` for a store the daemon keeps with no mount),
the origin URL the daemon recorded at clone time (credentials, query and
fragment removed) and the history it produced. Sizes come from the daemon and
are shown only for mounted clones; grok never walks a clone or its store. When
no daemon is running, no clones are listed; a daemon too old to list its clones
is named with a hint to restart it.

`rm` sends the request to the daemon that listed the clone. The daemon
unmounts it, drops it from its registry and moves the store out of its
`repos/` directory in one step, then deletes it in the background (a delete
cut short is finished once the daemon that owns the data directory starts
again, including after `grove restart`). With `--keep-store` the store stays
on disk and shows up as an orphaned store; `rm` of that store path or id
deletes it later. A store id that two daemons share is refused; pass the path
instead. A store the daemon did not create under its data directory (for
example one mounted with `grove mount --store`) is never deleted.

Only one daemon runs on a data directory: it holds the directory's lock for
its whole life, and another daemon started on the same directory (for example
under another `XDG_RUNTIME_DIR` or `GROVE_CONTROL_SOCK`) refuses to start and
names the one that holds it. `grove restart` hands the lock to the new daemon. The
daemon also refuses to start when the data directory is a symlink (set
`data_dir` in the grove config to its target) or sits on a filesystem without
working advisory locks.

grok never deletes Grove files itself. `rm` refuses a path inside a Grove data
directory that no running daemon lists (start that daemon and retry), a path
inside a mount of a daemon too old to list clones (restart it), and any path it
cannot place while a running daemon does not answer.

`rm` refuses, before anything changes, while a grok worktree was made from the
clone, while Grove worktrees were forked from it, and, unless `--keep-store`
is given, while other mounts or linked worktrees keep data in its store
(entries under the store's `git/worktrees/`, including ones made with plain
`git worktree add`; stale entries that `git worktree prune` would drop do not
count). `--dry-run` reports the same refusals.

`grok worktree gc` never touches clones. Local-view clones (made from a
checkout of the same URL) created before clones were recorded are not listed;
remove one with `grove unmount --forget <dir>`.
