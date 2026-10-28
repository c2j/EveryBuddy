#!/usr/bin/env python3
"""Build an EvoBuddy tasks.json with long ACP-style transcripts.

Used by the EB_SWITCH_TEST harness (local-verification recipe 27): it
reproduces the transcript size a real deep-research task reaches, which is what
makes a task switch cost measurable milliseconds of UI-thread time. The harness
saves over the state file when a task closes, so copy this fixture to a scratch
path per run rather than pointing `EB_STATE` at the generated file itself.

Usage: make_heavy_state.py <path> [tasks=3] [rounds=10]

Each round is a user turn plus an agent turn of eight blocks, so `rounds` is
half the turn count.
"""
import json
import sys

WORDS = ("duckdb analytics columnar database memory pipeline scan aggregation "
         "parquet csv json delta lake warehouse embedding table join index "
         "partition benchmark ingest query planner execution").split()


def prose(target, paragraphs=6):
    out = []
    n = 0
    for p in range(paragraphs):
        line = []
        length = 0
        while length < target // paragraphs:
            w = WORDS[(n + length) % len(WORDS)]
            line.append(w)
            length += len(w) + 1
            n += 1
        out.append(" ".join(line).capitalize() + ".")
    return "\n\n".join(out)


def blocks(round_index):
    text = prose(2400)
    return [
        {"Thought": {"text": prose(300, 1)}},
        {"Tool": {"status": "Completed", "primary": "read_file",
                  "detail": f"src/pipeline_{round_index}.rs"}},
        {"Tool": {"status": "Completed", "primary": "run_command",
                  "detail": "duckdb -c 'SELECT count(*) FROM scans'"}},
        {"Tool": {"status": "Completed", "primary": "search", "detail": "12 results"}},
        {"Text": text},
        {"Diff": {"path": f"reports/scan_{round_index}.sql", "added": 18, "removed": 3,
                  "lines": ([{"Add": f"  SELECT col_{i} FROM scans"} for i in range(12)]
                            + [{"Remove": f"  SELECT old_{i} FROM scans"} for i in range(3)]
                            + [{"Context": "  FROM t JOIN u USING (k)"}])}},
        {"Exec": {"command": f"duckdb -c 'PRAGMA memory_limit=\"1GB\"' --round {round_index}",
                  "output": "\n".join(f"{i}: {prose(60, 1)}" for i in range(30)),
                  "exit_code": 0}},
        {"Text": prose(900, 2)},
    ]


def task(index, rounds):
    turns = []
    for r in range(rounds):
        turns.append({"User": {"text": f"task {index} question {r}: " + prose(180, 1),
                               "time": "now"}})
        turns.append({"Agent": {"name": "Grok Build", "status": "Finished",
                                "blocks": blocks(r + index), "streaming": False}})
    return {
        "title": f"Heavy research task {index}",
        "session_id": f"session-{1700000000 + index}",
        "workspace_id": None,
        "agent_route": None,
        "last_route": None,
        "updated_at": 1761000000 + index,
        "pinned": False,
        "archived": False,
        "turns": turns,
    }


def main():
    path = sys.argv[1]
    tasks_count = int(sys.argv[2]) if len(sys.argv) > 2 else 3
    rounds = int(sys.argv[3]) if len(sys.argv) > 3 else 10
    tasks = [task(i, rounds) for i in range(tasks_count)]
    with open(path, "w") as handle:
        json.dump(tasks, handle, indent=1)
    size = sum(len(json.dumps(t)) for t in tasks)
    print(f"wrote {path}: {len(tasks)} tasks, {len(tasks[0]['turns'])} turns each, "
          f"{size // 1024} KB of transcript")


main()
