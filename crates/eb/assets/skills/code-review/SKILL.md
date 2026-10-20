---
name: code-review
description: Review a change for correctness, regressions, and missing tests.
version: 1
examples: ["Review the current changes", "Find the regression this diff introduces"]
triggers: [review, diff, bug, regression]
---
When reviewing code:

1. Read the diff and the surrounding call sites before judging it.
2. Separate blocking correctness/security issues from optional suggestions.
3. Check error paths, cancellation, and resource cleanup.
4. Name the missing test that would have caught each blocking issue.
5. Do not invent files or behavior that the diff does not show.
