# Browser module driver gate

Implementation commit: `432ba91123c3e9f9b127889cb1ddd46ff8a73e40`.
Fixed base: `680822e9`, with DOM `4ce0126d`, JS source-context `f86b814d`,
and typed/cache module failures `ae89a0af` adopted separately.

Boat pool-ia native source/target gate: release driver build, workspace nextest
**3564 passed / 17 skipped**, workspace doctests **4 passed**. All ten new
module adapter regressions passed, including delayed imports after document
base mutation and realm-isolated iframe namespaces.

Pinned WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, all 111 testharness
pages below `html/semantics/scripting-1/the-script-element/module`, 5000ms,
8 workers:

| Classification | Before | After |
| --- | ---: | ---: |
| Passing pages | 0 | 35 |
| Failing pages | 52 | 39 |
| Timeout pages | 53 | 30 |
| Error pages | 5 | 6 |
| Unsupported pages | 1 | 1 |
| Passing subtests | 2 | 110 |

**35 gained pages, 108 gained named subtests, zero lost passes.**
`before.json` includes only the DOM decoding dependency; classifications and
counts were identical to the original base before adopting that dependency.
`after.json` carries binary/selection hashes and the complete named-subtest
comparison. Execution preserves the pinned upstream bytes; fixture files are
read-only. Timeouts and synthetic HTTP endpoint limitations remain visible.

Reproduce on Boat after native source/target setup:

```sh
bash scripts/ensure-wpt.sh
cargo build -p amazing --release --locked
cargo nextest run --workspace --locked --no-fail-fast
cargo test --workspace --locked --doc
python3 crates/amazing/evidence/module-driver/run-corpus.py after \
  --baseline crates/amazing/evidence/module-driver/before.json
```

The runner requires `OUT` and `CARGO_TARGET_DIR` from the lane wrapper. General
async graph scheduling, import maps, worker modules and HTTP fetch policy are
separate follow-ups; this gate measures synchronous graph driver adoption.
