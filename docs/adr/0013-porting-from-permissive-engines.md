# ADR 0013 — Porting code from permissively licensed engines

Status: accepted (2026-09-27). Approved by the project owner in the orchestrator session
("Go ahead" on option 1: port with attribution). Policy text lives in `AGENTS.md`,
"Porting from other engines"; this ADR records the decision and its boundaries.
Owner: integrator. Applies to every lane.

## Context

Amazing's end goal is a complete, independent browser written in Rust. On 2026-09-27 the
engine passed 69.8% of the pinned Test262 on `dev` while several open-source Rust engines had
already solved the same spec areas: `lucid-softworks/lumen` (MIT, std-only, zero dependencies)
measured 53,304/53,458 (99.7%) on our own pinned Test262 checkout; `boa-dev/boa` and
`Hans-Halverson/brimstone` (MIT) report >90% and >97% of the language; `boa-dev/temporal`
(Apache-2.0) implements Temporal; `DioxusLabs/taffy` (MIT) implements block/flex/grid layout.
Lanes were re-deriving those algorithms from the spec text at a much slower rate.

Three options were considered:

1. Port code with attribution into our own crates, keeping our architecture.
2. Replace `amazing-js` with an existing engine (lumen was the candidate).
3. Stay strictly from scratch and use other engines only as reading material.

## Decision

Option 1. Lanes may copy and adapt source from **MIT or Apache-2.0** projects into Amazing's
crates. This is porting, not depending:

- No new crate dependencies. Engine cores stay std-only; upstream dependencies (ICU4X,
  temporal_rs types, GC crates) are rewritten onto our own types.
- Our architecture stays: value model, bytecode VM (ADR 0003), DOM/WebIDL bindings, compositor.
- Every file with ported code carries a header naming upstream repo, path, commit and license;
  `THIRD_PARTY_NOTICES.md` gets an appended row per port (append-only; the file is union-merged).
- MPL-2.0 (Servo, Stylo, WebRender, Nova) and GPL/LGPL sources are reference-only.
- Ports obey every other rule: files under 4,000 lines, zero lost passes on the lane's gate.

## Consequences

- "From scratch" now means *our own architecture and no third-party runtime dependencies*,
  not *every line originally authored here*. Ported regions are traceable by header.
- Option 2 was rejected: replacing the JS engine would discard the VM, value model and every
  binding built on them, and re-plumbing would cost more than the conformance it buys.
- The lumen checkout at `f6c9860d` on venus-001 (`~/scratch/lumen`) serves as a behavioural
  oracle for the JS lanes.
- License compliance is a merge check: the integrator verifies the header and notice row for
  every merged port.
