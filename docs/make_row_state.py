#!/usr/bin/env python3
"""Sidebar row-states fixture for `EB_ROWS` (ui-spec §10-40).

Writes the tasks.json the harness needs: one row per sidebar state, each with
an `updated_at` that lands on a specific rung of the product's time ladder.
Timestamps are relative to *now*, so the archive is only good for the run that
generated it - which is exactly how the harness uses it.

    python3 docs/make_row_state.py /tmp/eb-rows/tasks.json
"""

import json
import sys
import time

DAY = 86_400


def agent_turn(status=None, blocks=(), streaming=False):
    return {
        "name": "EvoBuddy",
        "status": status,
        "blocks": list(blocks),
        "streaming": streaming,
    }


def task(title, updated_at, turn, **flags):
    return {
        "title": title,
        "session_id": None,
        "workspace_id": None,
        "agent_route": None,
        "last_route": None,
        "updated_at": updated_at,
        "pinned": flags.get("pinned", False),
        "archived": flags.get("archived", False),
        "failure_unseen": flags.get("failure_unseen", False),
        "unread": flags.get("unread", False),
        "turns": [
            {"User": {"text": "prompt", "time": "now"}},
            {"Agent": turn},
        ],
    }


def main(path):
    now = int(time.time())
    # Newest first in the list, because the sidebar reverses the archive.
    rows = [
        # 1. A terminal failure nobody has opened: red badge, no timestamp.
        task(
            "row: failed unseen",
            now - 30,
            agent_turn(status="Failed: boom"),
            failure_unseen=True,
        ),
        # 2. The same failure after the task was opened: plain row, time shows.
        task("row: failed seen", now - 3 * DAY, agent_turn(status="Failed: boom")),
        # 3. Streaming: a bare turn spins as Working, an open plan as Planning.
        task(
            "row: working",
            now - 45,
            agent_turn(blocks=[{"Text": "still going"}], streaming=True),
        ),
        task(
            "row: planning",
            now - 2 * DAY,
            agent_turn(
                blocks=[
                    {
                        "Plan": {
                            "entries": [
                                {"content": "step", "status": "in_progress"}
                            ]
                        }
                    }
                ],
                streaming=True,
            ),
        ),
        # 4. Settled: the ladder rungs, one row each.
        task("row: just now", now - 20, agent_turn(blocks=[{"Text": "done"}])),
        # The unread dot is the one mark that shares its slot with the clock.
        task(
            "row: unread",
            now - 5 * 60,
            agent_turn(blocks=[{"Text": "done"}]),
            unread=True,
        ),
        task("row: yesterday", now - DAY, agent_turn(blocks=[{"Text": "done"}])),
        task("row: 3d", now - 3 * DAY, agent_turn(blocks=[{"Text": "done"}])),
        task("row: 6d", now - 6 * DAY, agent_turn(blocks=[{"Text": "done"}])),
        task("row: 8d", now - 8 * DAY, agent_turn(blocks=[{"Text": "done"}])),
        # 400d lands in last year, which is the only way to reach the date rung.
        task("row: 400d", now - 400 * DAY, agent_turn(blocks=[{"Text": "done"}])),
    ]
    with open(path, "w", encoding="utf8") as handle:
        json.dump(rows, handle, indent=1)
    print(f"wrote {len(rows)} sidebar rows to {path}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
