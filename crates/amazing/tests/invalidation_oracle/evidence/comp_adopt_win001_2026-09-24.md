# IA-COMP-ADOPT Win-001 release trace, 2026-09-24

Source: `58df3b150914e865c7998243fcce7b0b3dedd549` on `lane/ia-comp-adopt`. The Win-001 lane tree was refreshed from `63198e8c` and the three changed `crates/amazing` files were synced afterwards; their SHA-256 hashes matched the Mac worktree. The test ran with `CARGO_TARGET_DIR=C:\Users\server\target\integrator-assist`:

```text
cargo test -p amazing --locked --release --bin amazing incremental_oracle_mutation_trace -- --ignored --nocapture
test result: ok. 1 passed; 0 failed
```

| Fixture, 20 frames | Candidate p95 | Direct full p95 | Style/layout/paint-view calls | Paint full | Uploads | Layer rounding | Parity losses |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Article class | 149.999 ms | 32.077 ms | 20/20/60 | 0 | 0 | 0 | 0 |
| Effects scroller | 120.875 ms | 24.111 ms | 20/20/60 | 20 | 240 | 40 (2 per frame) | 0 |
| Opacity class in document `<style>` | 39.637 ms | 16.868 ms | 0/0/0 | 0 | 0 | 0 | 0 |

The compositor classified each effects difference as a layer-boundary pixel with no channel differing by more than one. The oracle counted those pixels separately and presented the retained frame without a parity fallback. Article and effects p95 remain above ADR 0010's 20 ms and 31 ms targets; the trace includes the full recomputation oracle on every frame. The property-only opacity frame met the zero stage-work target.
