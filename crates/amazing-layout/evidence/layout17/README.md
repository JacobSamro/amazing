# LAYOUT-17 Windows gate

Source: local `dev` paint merge `d70aec65` incorporated at lane head `ff3a7569`,
with LAYOUT-17 implementation commit `b34ac9e1`. The win-002 `win-run.sh`
archive was taken from that worktree; `win002-gate.log` is the raw output.

- `amazing-layout` nextest: 285 passed, one ignored benchmark.
- `amazing-layout` doctests: 0 passed, 0 failed.
- Workspace nextest: 2,362 passed, seven skipped.
- Workspace doctests: 2 passed, 0 failed.
- Ignored release diagnostic, 500 stable siblings and 40 contained width
  mutations: p95 **14.766 ms** on win-002. This times layout update only;
  the ADR 0010 mutation-to-frame target still needs driver integration.

The layout damage contract is `LayoutTree::damage()` plus
`LayoutTree::box_for_ref()`. The adapter handoff is in replaced-content's
`questions.md`.
