# Amazing Architecture

A from-scratch, zero-dependency (std-only) web browser engine. This document
records decisions that are **locked** — changing them later is a project-level
event, not a refactor.

## Component graph

```
amazing-net ──bytes──▶ amazing-html ──tree ops──▶ amazing-dom
      │                                            │
      │                              styles        │ tree mutations
      │                                 ▼          ▼
      │                             amazing-css   amazing-js  ◀── DOM/JS bridge (Phase 7)
      │                                 │
      │                                 ▼ computed style tree
      │                            amazing-layout ──layout tree──▶ amazing-paint ──pixels──▶ GUI shell
      └──image bytes──▶ amazing-media
amazing-media ──intrinsic size via PageMedia──▶ amazing-layout
amazing-media ──RGBA8 via PageMedia───────────▶ amazing-paint
amazing-media ──validated PCM WAV─────────────▶ GUI shell (WinMM adapter)
```

Dependency direction always points "downward" in the pipeline; nothing in a
lower layer may import a higher one except through the `amazing` driver binary.

## Locked decisions

### 1. Single-threaded first

The entire engine v1 runs on one thread with an explicit, cooperative event
order: *network → parse → style → layout → paint*, then JS microtask/job queue
drains at defined points (HTML event loop semantics, WHATWG §8.1).

Rationale: correctness and debuggability before throughput. Every real engine
parallelized *after* its data model was stable, not before.

### 2. Where parallelism will land later (seams reserved now)

These are noted so Phase 0 code doesn't accidentally foreclose them, not as
v1 features:

- **Style resolution** — per-subtree work-stealing over the arena once
  selector matching exists; requires the style-sharing candidate check to be
  pure (no interior mutability).
- **Layout fragmentation** — block formatting contexts partition naturally;
  inline FC does not. Parallelize BFC subtrees only.
- **Paint tiling** — rasterizer already operates on rectangular tiles of the
  pixel buffer; tiles are embarrassingly parallel.
- The DOM arena (`Vec<Node>` + u32 indices) makes all of the above possible:
  subtrees are contiguous-ish index ranges, snapshots are cheap clones, and
  there is no shared mutable `Rc` state to fight over.

### 3. Arena-allocated DOM

`amazing_dom::Dom` = flat `Vec<Node>`, references are `NodeIdx(u32)`.
No `Rc<RefCell<..>>` anywhere in the engine core. Consequences we accept:

- Node removal = freelist insertion; indices stay stable until reused.
- Borrow rules are manual (the tree builder holds `&mut Dom`; delegates use
  indices). This is deliberate — it keeps mutation auditable.

### 4. Hand-written parsers follow spec state machines

HTML tokenizer states and CSS Syntax token names are copied verbatim from the
specs. Error-recovery behavior *is* the interop contract — never invent a
"simpler" grammar.

### 5. unsafe policy

No `unsafe` outside platform-boundary modules (windowing FFI, Phase 8+).
Every `unsafe` block carries a comment justifying: what invariant upholds
safety, why std cannot express it. Engine crates compile with `unsafe_code`
denied (enforced via workspace lint config as crates appear).

### 6. TLS exception (decided at Phase 5)

Working assumption: Amazing uses a vetted TLS implementation rather than
hand-rolled TLS 1.3. Rationale lives in `crates/amazing-net/src/lib.rs`;
revisit only if the constraint set changes.

## Conformance tracking

| Phase | Suite | Gate |
|-------|-------|------|
| 1 | html5ever/html5lib tokenizer + tree tests | report % |
| 2–3.5 | WPT css/* subdirs per phase | report % |
| 4 | screenshot diffs vs reference renderer | visual review |
| 6+ | Test262 | report % |

Numbers go in each phase's completion report. Estimates are forbidden.
