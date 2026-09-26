# Merged oracle and shared blend, Win-001 effects check

Source: local `dev` `aed4230b` (IA-ORACLE-LAYOUT/PAINT, RC-20, shared `blend_source_over`).
Command: `cargo test -p amazing --release --locked --bin amazing incremental_oracle_mutation_trace -- --ignored --nocapture` on Win-001 with the integrator-assist target directory.
Result: 1 passed, 0 failed; 20 mutations per fixture and direct full-render byte comparison on every frame.

| Fixture | Candidate p95 | Direct full p95 | Pixel parity losses |
| --- | ---: | ---: | ---: |
| Article width class | 136.244 ms | 31.586 ms | 0 |
| Effects scroller width class | 125.125 ms | 25.768 ms | 21 |
| Opacity class | 57.767 ms | 13.550 ms | 0 |

The effects fixture still differs at `(38,178)`: compositor `[95,95,95,255]`, full painter `[96,96,96,255]`, on the initial frame and all 20 mutations. The oracle presents the full painter on every loss. Shared integer source-over blending alone did not remove this nested-opacity difference. The compositor follow-up is recorded in `.agents/integrator-assist/questions.md`.
