# Schedule timer templates

`evobuddy schedule --run-due` generates a due daily plan and exits. It does not open a window and does not start a session. Approved items wait until the next time the window is open.

These files are examples. The app does not install them.

Edit the program path before use. The command reads `~/.evobuddy/` unless `EB_STATE` points at a `tasks.json`.

## Linux

```text
mkdir -p ~/.config/systemd/user
cp evobuddy-plan.service evobuddy-plan.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now evobuddy-plan.timer
```

## macOS

```text
cp com.evobuddy.plan.plist ~/Library/LaunchAgents/
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.evobuddy.plan.plist
```

## Windows

```text
schtasks /create /tn EvoBuddyPlan /sc daily /st 08:30 /tr "\"C:\Path\evobuddy.exe\" schedule --run-due"
```

## Exit codes

`0` nothing due or a plan was written; `1` state files unreadable (corrupt
`tasks.json`); `2` usage error; `3` the schedule files stayed busy after a
short retry — the run was skipped and the next timer run will retry.
