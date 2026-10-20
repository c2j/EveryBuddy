---
name: release-notes
description: Turn merged changes into concise user-facing release notes.
version: 1
examples: ["Write release notes for this change", "Summarize the breaking changes"]
triggers: [release, changelog, notes]
---
When writing release notes:

1. Group changes by user impact, not by commit order.
2. Omit internal refactors unless they change behavior.
3. State breaking changes and migration steps explicitly.
4. Keep each bullet short and verifiable; never invent a feature.
