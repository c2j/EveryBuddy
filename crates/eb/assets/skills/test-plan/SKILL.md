---
name: test-plan
description: Turn a change into a focused, runnable test plan.
version: 1
triggers: [test, coverage, regression]
---
When producing a test plan:

1. Start from the changed behavior and its failure modes.
2. List the smallest set of unit/integration cases that prove it.
3. Name concrete commands or files where the project provides them.
4. Include one negative case and one restart/persistence case when relevant.
5. Do not claim a test ran unless its output is available.
