# IA-ORACLE-LAYOUT + IA-ORACLE-PAINT, Win-001 release trace

Base: local `dev` `93bb98ba180c39d482987ad67f5e03d0c168c98e`.
Code: `e7ddac71d907477c96349173ec613cdcb74d0372`.
Command: `cargo test -p amazing --release --locked --bin amazing incremental_oracle_mutation_trace -- --ignored --nocapture` with `CARGO_TARGET_DIR=C:\Users\server\target\integrator-assist`.
Result: 1 passed, 0 failed; 20 alternating class mutations per fixture. Timings include mutation through final pixel oracle and presentation, in microseconds. Each candidate frame was compared byte-for-byte with the direct full-render result.

| Fixture | Candidate p50 | Candidate p95 | Direct full p50 | Direct full p95 | Style calls | Layout update calls | Full layouts | Paint view calls | Full paint updates | Tile uploads |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Article width class | 251650 | 266614 | 59549 | 63495 | 20 | 20 | 20 | 20 | 0 | 0 |
| Effects scroller width class | 223907 | 248500 | 48227 | 56286 | 20 | 20 | 20 | 20 | 20 | 240 |
| Opacity class | 105595 | 110244 | 25006 | 26859 | 20 | 20 | 0 | 20 | 0 | 0 |

ADR 0010's 20 ms article and 31 ms effects p95 targets are missed. The opacity property-only target of zero style, layout, and paint calls is also missed. The trace uses the driver oracle (including separate full recomputation, resource discovery, and byte comparison), so its numbers are not directly comparable to ADR 0010's lower-level compositor baseline. Current layout `update` rebuilds width changes. Current paint `update` requires complete before/after views. The driver also builds an independent full layout for every oracle frame.

The effects fixture recorded 21 known one-channel pixel differences (initial frame plus 20 mutations), all at `(38,178)`: compositor `[95,95,95,255]`, full painter `[96,96,96,255]`. The coordinator presented the full painter on each loss. Article and opacity frames had no pixel losses. The first-difference diagnostic is routed to compositor/replaced-content in `.agents/integrator-assist/questions.md`.
