# EvoBuddy rutis live bridge

EvoBuddy-specific A2 bridge. It mounts the **generic** rutis-dsh agent
composition and exposes the real messageFeedback service to the WebKit
component host.

```sh
cargo build
./target/debug/evobuddy-rutis-bridge
```

Prints a live session and listens on `http://127.0.0.1:39472`.

This crate deliberately lives in EvoBuddy, not in `vendor/rutis-0.12`:

- scripted LLM/session choice is an EvoBuddy test concern
- `/tmp/evobuddy-rutis-sessions` is an EvoBuddy path
- HTTP bridge is EvoBuddy-specific

rutis only carries the generic pieces: the persistent AgentProbe and the
session-persistence/message-feedback services in its agent composition.
