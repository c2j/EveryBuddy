# rutis upstream: persistent agent probe + feedback services

Status: draft issue/PR material. Local clean branch:
`vendor/rutis-0.12` → `evobuddy/upstream-generic` @ `cafcb35a`, based on
`v0.12.1`.

## Issue title

`rutis-dsh: expose a persistent agent session for feedback and embedding`

## Problem

`AgentProbe.run()` creates one agent, runs a turn, returns the derived
messages, and disposes the session in `finally`. That is the right shape for a
one-shot loop test, but an embedding that needs to attach a feedback surface to
an assistant message cannot use it:

- the session is gone after `run()`
- no `sessionId` is returned
- no assistant `messageId` is exposed
- the message-feedback host service cannot be exercised against that session

## Proposed change

1. Add to `AgentProbe`:
   - `runPersistent(provider, model, prompt) -> JSON`
   - `liveSnapshot() -> JSON`
   - `disposePersistent()`
2. Keep the existing `run()` behavior unchanged.
3. Add `session-persistence-jsonl` and `message-feedback` to the generic agent
   composition so the persistent path has a durable session and the canonical
   feedback service.
4. Add a test that:
   - runs a persistent turn
   - checks the returned `sessionId` and assistant `id`
   - compares `liveSnapshot()`
   - disposes and verifies the probe is empty
   - runs a second session with a different id

## API sketch

```ts
async runPersistent(provider: string, model: string, prompt: string): Promise<string>
liveSnapshot(): string
async disposePersistent(): Promise<void>
```

The returned JSON is:

```json
{
  "sessionId": "persistent-...",
  "messages": [{ "id": "...", "role": "assistant", "content": [] }]
}
```

## Compatibility

`run()` keeps its current contract. `runPersistent()` is additive. The agent
composition gains two services; the existing tests are updated for the new
config fields. If upstream prefers not to make these mandatory, the alternative
is an optional mount/group for persistent/feedback compositions.

## Validation

```sh
cd crates/rutis-dsh/dsh && npm ci
cd ../..
cargo test -p rutis-dsh --test agent
```

Result: existing agent test passes, new persistent test passes, one real-model
test remains ignored.

## PR summary

```text
rutis-dsh: expose persistent agent sessions for feedback services

- add AgentProbe.runPersistent/liveSnapshot/disposePersistent
- add session-persistence-jsonl and message-feedback to the agent composition
- test a persistent turn, snapshot, dispose, and fresh-session id

The existing one-shot probe and its test are unchanged.
```
