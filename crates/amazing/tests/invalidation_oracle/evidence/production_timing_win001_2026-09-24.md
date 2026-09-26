# IA-COMP production timing, Win-001, 2026-09-24

Code: `715273819a6f810d918db1a17c5befd57d67c490`. The Win-001 lane tree retained the fixed `63198e8c` base, with its changed driver files resynced. SHA-256 for `cmd/render.rs` was `a0a24fdb410011048c2ed124cf7ef9dce3e097f6983c26276e091d2cfb3de3e7`; for `render_oracle.rs`, `3f8a8868807f07d8008085b5e50dab8691591fe424ef2fec07535b7141da4532`. Both matched the Mac worktree.

The ignored release trace runs 20 alternating class mutations per fixture. Each candidate timer covers its DOM mutation and the oracle-free retained rendering call through its presented surface. The direct full renderer is timed separately on an identical document and mutation. Pixel parity classification runs after both timers. No full oracle computation or comparison runs inside the candidate timer.

```text
cargo test -p amazing --locked --release --bin amazing incremental_production_mutation_trace -- --ignored --nocapture
test result: ok. 1 passed; 0 failed
```

| Fixture | Candidate p50 / p95 | Direct full p50 / p95 | Style/layout/paint-view calls | Paint full | Uploads | Layer rounding | Losses |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Article class | 71.364 / 77.389 ms | 31.417 / 32.705 ms | 20/20/20 | 20 | 80 | 0 | 0 |
| Effects scroller | 88.560 / 97.082 ms | 28.006 / 31.101 ms | 20/20/20 | 20 | 240 | 40 | 0 |
| Opacity class in document `<style>` | 37.759 / 40.775 ms | 16.301 / 19.246 ms | 0/0/0 | 0 | 0 | 0 | 0 |

Article and effects candidate p95 remain above ADR 0010's 20 ms and 31 ms targets. The production path avoids the diagnostic oracle but still rebuilds the paint view and full paint for their layout-changing mutations. The opacity frame meets the property-only zero stage-work target; its elapsed time includes CSS input collection and software presentation.

The Win-002 locked gate on the same code passed: `amazing` nextest **184 passed / 5 skipped**, workspace nextest **2,377 passed / 8 skipped**, workspace doctests **2 passed / 0 failed**.
