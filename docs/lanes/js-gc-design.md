# amazing-js cycle collection: design (ticket r4-gc-design)

Status: design only, no code in this ticket. Base: origin/dev `4c0ccfc41`.
Inputs read: parked branch `lane/native-graph-gc-rebase` (`72efe31a7`, report in `.agents/resume-gc/report.md`,
`docs/lanes/native_graph_collection_source_plan.md` on that branch), the current value model, boa `core/gc`
(`amazing-ref/boa` @ `9871775a4`), lumen's cycle collector (`amazing-ref/lumen` @ `a0f0314de`,
`crates/lumen/src/interpreter.rs`, "garbage collection" section and `gc_check`).

## 1. What we have today

- Objects are `Obj(pub Rc<RefCell<JsObject>>)` (`value.rs:299`), environments `EnvHandle(Rc<RefCell<Env>>, ..)`
  (`env.rs:25`), bindings `BindingCell(Rc<RefCell<Binding>>)`. About 457 mentions of `Obj`, 250 of `EnvHandle`,
  73 `Rc<RefCell` in amazing-js; 117 non-test places build an `Obj` with `Obj(Rc::new(RefCell::new(..)))`, 74 build an
  `EnvHandle`/`Rc<RefCell<..>>` environment. The VM registers, call frames, native Rust locals, host TLS maps and the
  DOM wrapper caches all hold plain strong `Rc` clones.
- Memory is reclaimed only by refcount. Any cycle (closure <-> its environment, object <-> method closure, class
  prototype <-> constructor, promise reaction chains, generator frames) leaks for the life of the realm. Measured:
  `closures` benchmark peaks at 102 MB against 15 MB for the other programs (`js-perf-baseline.md`).
- `WeakRef`, `WeakMap`, `WeakSet` and `FinalizationRegistry` are thread-local sidecars keyed by pointer holding
  `Rc::downgrade` handles (`builtins_std_collections.rs`). They only observe refcount death, so a target that is part
  of a cycle is never "collected" and `$262.gc()` (`binding_collect_garbage`) cannot free anything meaningful.
  FinalizationRegistry callbacks therefore never fire for cyclic targets. This is where the parked branch's +66
  Test262 files came from (WeakMap/WeakSet/WeakRef/FinalizationRegistry with symbol keys and cleanup).

## 2. Root cause of the parked branch's blowup

The branch (12 WIP commits, 166 files) made every runtime `Rc` a sealed `ManagedCell` and required a *complete,
exact inventory of every strong slot* so that collection could be proven safe ("borrowed or genuinely opaque state
defers the entire collection"). Costs, from `managed_rc.rs` on that branch:

1. `ManagedCell::new` takes a fresh `AllocationId`, registers the allocation in a registry and walks the value to
   place every alias slot (`place_slots`): per-allocation hash-registry work and memory.
2. Every `clone()` calls `custody::current_alias(allocation)`; every `borrow_mut()` runs
   `TraceVisitor::alias_slots(&*value)` *before* the mutation (and again after) to snapshot all slots the value
   holds. For an array or object with N properties each mutation is O(N) and allocates a Vec of N slot records.
   That makes `a[i] = x` loops quadratic and gives the 10-20x memory: `toSpliced-dense` 326 MB -> 4.5 GB,
   `dst-offset-caching-1` 52 s / 326 MB -> 253 s / 6.9 GB, JSON mega-huge 12 s -> 247 s, full Test262 1196 s -> 10073 s.
   All 17 lost files were deadline or OOM, none semantic.
3. The design needed exactness to be sound: a missing slot in the inventory was a safety bug, so the whole graph
   had to be sealed, which forced the 166-file change and the 4 interdependent unit failures nobody could localise.

The cost is structural: the proof obligation ("every edge known on every mutation") sits on the hot path. The fix is
a collector whose safety does not depend on an exact inventory.

## 3. Options considered

| Option | Verdict |
|---|---|
| Arena + handles + mark-sweep (boa `boa_gc`: `Gc<T>` boxes in a thread-local list, `Trace`/`Finalize`, root counts on handles, ephemerons, weak maps; ~4.3 kLOC) | Right end state for a tracing GC, but needs every `Obj`/`EnvHandle` user (457+250 sites, the DOM bindings, all natives holding Rust-stack references) to become rooted handles or to scan the Rust stack. Exactness again: a missed root is use-after-free. Rejected for now; revisit only if cycle collection proves insufficient. |
| Sealed Rc with custody (the parked branch) | Rejected, section 2. |
| **Trial-deletion cycle collector over the existing `Rc` (lumen's design)** | Chosen. |

Lumen's observation: for a candidate heap node, `Rc::strong_count - (references from other candidate heap nodes)` is
the number of *external* holders (Rust stack, VM registers, TLS tables, host maps, anything not traced). Nodes with
external holders are roots; everything reachable from them is live; the rest is only referenced from inside
unreachable cycles and can be reclaimed by breaking its outgoing edges so ordinary `Rc` drops free it. No root
enumeration, safe to run in the middle of evaluation.

**The key safety property:** an edge the tracer does not know about is counted as an *external* holder, so its target
becomes a root. A missing or opaque edge therefore costs a leak (status quo), never a use-after-free. The only way to
corrupt the heap is to report an edge that does not hold a strong count (for instance tracing through a `Weak`, or
tracing a field twice). That is a small, local, testable invariant ("the tracer reports exactly the strong `Rc` fields
it owns"), checkable by a debug assertion that compares `strong_count` deltas before and after clearing a garbage
node, instead of an inventory of the whole program. This is what makes incremental, zero-loss steps possible.

## 4. Design

### 4.1 Nodes and registry
Collectable nodes (phase order in section 6): `JsObject` (`Obj`), `Env` (`EnvHandle`), `BindingCell` is not a node
(it is owned by an `Env`/arguments map and traced through them; shared cells are counted as edges to the cell's
value). Later: generator/async frames (`GeneratorState`), `PromiseState` reaction lists, `ProxyState`, `ClassElements`,
module records, `DisposableStack`.

Construction goes through two constructors, `Obj::alloc(JsObject)` and `EnvHandle::new` (the 117 + 74 construction sites
become a mechanical replacement; the public `Obj(pub Rc<..>)` field stays, nothing is sealed). Each registers
`Weak<RefCell<_>>` in a per-thread `Vec` and bumps a live counter. A dead `Weak` pins its `RcBox` until pruned, so the
registry is pruned (retain upgradable entries) when it exceeds 2x the live count at last prune (lumen's
`scope_registry_prune`); worst-case overhead is the pinned `RcBox` size (about one `JsObject`) per dead entry, bounded by
the 2x rule. If the memory benchmark shows this pinning matters, step S0b replaces the `Weak` vector with an intrusive
index stored in the node and removed in `Drop` (needs a small, audited `unsafe` deref inside the collector only).

### 4.2 Collection (synchronous, whole registry)
1. Snapshot the registry into `Vec<NodeRef>`; assign dense indices via a pointer-keyed map.
2. For every node, `try_borrow()`; on failure mark it a root (someone is inside it) and do not trace it.
3. Trace each node's strong outgoing edges (`NativeTrace::trace(&mut Tracer)`), counting `internal[target] += 1` for
   registered targets. Edges to unregistered targets (strings, numbers, host `Rc`s) are ignored.
4. `external = Rc::strong_count - internal`. Nodes with `external > 0` are roots. Mark from the roots over the
   recorded edges (no second trace needed).
5. Garbage = unmarked. Before clearing, run the weak step (4.5) so weak targets in garbage are observed dead.
6. Break cycles: for each garbage node, `mem::take` its outgoing strong fields into a local `Vec<Dropped>`; drop that
   vector only after every garbage node has been emptied. Nothing in this phase calls author code: no finally, no
   iterator close, no disposal, no finalizer. Rust `Drop` impls reached (`BinaryBuffer`, `SharedBlock`, host handles)
   must not call into JS; audit list in S1.
7. Debug builds assert that each garbage node's `strong_count` dropped to the expected value after step 6.

Soundness conditions: single mutator thread; no borrow on garbage nodes (guaranteed, a live borrow implies a live
holder, which makes the node a root); tracer never traces `Weak` and traces each strong field exactly once.

### 4.3 Triggers and cost control
- `$262.gc()` and an explicit `Interpreter::collect_garbage()` call the collector (S1).
- Automatic trigger at allocation safe points (VM call entry and loop back-edge, `Obj::alloc` slow path) when
  `live > gc_next`; after a collection `gc_next = clamp(2*live, 64k, limit)` (geometric, so total collection work is
  O(allocations)). Opt-in via `AMAZING_JS_GC=1` until the gates in section 7 pass, then default on.
- Large leaf optimisation: a node whose property table holds no object/closure/env values is flagged non-traced after
  the first scan until mutated (`shape_version` already changes on layout change); million-element numeric arrays are
  then skipped. Required before auto-trigger is default, measured against the five pathological Test262 files.

### 4.4 Roots: the VM stack, natives and hosts
Nothing is enumerated. VM registers, call frames, `Vec<JsValue>` argument lists and native locals are `Rc` clones and
show up as external counts. `GeneratorState`, suspended async frames and promise reaction closures are traced nodes
only from S2; until then they are opaque and everything they reference is a root (leak, as today).
OS-thread coroutines (`ThreadGenerator`, tree-walker fallback only) move values across threads under a serialised
hand-off. The registry is per thread and the collector only considers the registry of the thread it runs on; edges from
other threads' nodes are never traced and therefore count as external. Collection is skipped while a parked coroutine
thread holds borrowed state of this thread (counter `live_thread_generators > 0` defers), the same condition the parked
branch needed but without its custody machinery.

### 4.5 WeakRef / WeakMap / WeakSet / FinalizationRegistry
Current sidecars stay (they already use `Weak`), so refcount-death behaviour is unchanged in S1. In S3:
- Weak targets are not traced edges. After marking (4.2 step 4) and before clearing, resolve ephemerons to a fixpoint:
  a `WeakMap` value is live iff its key is marked; ephemeron values are marked while iterating. Weak targets that are
  garbage are then dead: `WeakRef.deref()` returns undefined (the sidecar's `Weak` fails to upgrade after clearing),
  WeakMap/WeakSet entries are removed.
- `KeepDuringJob`: `WeakRef` construction and `deref` push the target on a per-job kept-alive list, treated as an
  external root, cleared at the end of the job (the parked branch implemented this from boa's `weak_ref.rs`; reuse that
  logic, MIT, with attribution).
- `FinalizationRegistry`: registry cells hold holdings and unregister tokens (token weak, holdings strong edge from the
  registry node, so cycles through holdings are handled). After the sweep, cells whose target died are queued; cleanup
  callbacks run later through the real job queue in the registering realm, never inside the collector. This is the
  protocol the parked branch adapted from boa's `finalization_registry/mod.rs`.
- Symbol keys (`CanBeHeldWeakly`): non-registered symbols are weak-holdable; symbol identity needs no node, its
  lifetime is the `Rc` count of the symbol payload (S3 detail).

### 4.6 DOM wrappers and host objects
Wrappers are ordinary `JsObject`s carrying engine-private slots (node id / `HostHandle`); the DOM itself is Rust data
outside the JS heap. Wrapper caches, event listener tables, timers and observer registries in `crates/amazing` hold
strong `Obj` clones in TLS maps, so wrappers and listener closures are external roots: not collected (today's behaviour,
safe). To collect cycles that pass through the DOM (node -> listener -> closure env -> wrapper -> node) the host exposes
edges with a trait defined in amazing-js: `trait GcHostRoots { fn trace_strong(&self, t: &mut Tracer); }` registered per
realm. Registered holders subtract their strong clones from the external count exactly like heap nodes. Host maps that
should be weak (wrapper cache keyed by node) switch to `Weak<RefCell<JsObject>>` in the host crate. Only after S4 do
DOM-heavy pages see memory benefits; S1-S3 must not change any host crate.

## 5. Risks

- Registry cost on allocation-heavy code (closures, class_inherit, method_calls benchmarks): one `Weak` clone + push per
  object/env. Target <= 3% on the 12-program bench; fall back to the intrusive index (S0b) if exceeded.
- Collection pause scales with live heap (million-element arrays, big DOMs): geometric trigger plus leaf skip; manual
  `$262.gc()` only until gates pass.
- A tracer that double-counts an edge can free live data. Mitigation: per-type unit tests asserting
  `trace` visits exactly the strong `Rc` fields (compare `Rc::strong_count` before/after a structural clone in a
  test-only helper), debug assertion in 4.2 step 7, and the differential: Test262 with `AMAZING_JS_GC_STRESS=1`
  (collect at every allocation safe point, slow, run on selected directories).
- `Drop` side effects: any `Drop` reachable from a garbage node must be audited (`BinaryBuffer`, `SharedBlock`,
  `ArrayJoinGuard` are the three `impl Drop`s in amazing-js today; none runs JS).
- Cross-thread values (section 4.4) are conservative by construction.

## 6. Incremental steps (each ends with a gate; zero lost Test262 files/variants in default and `AMAZING_JS_VM=0`)

| Step | Change | Behaviour change | Gate |
|---|---|---|---|
| S0 | `Obj::alloc` / `EnvHandle::new` constructors with registry, live counter, prune; `collect()` computes stats only (counts roots/garbage, frees nothing) behind `AMAZING_JS_GC=count` | none | Test262 full (zero lost, time within +3%), 12-program bench within +3%, memory bench |
| S0b | (only if S0 shows pinning cost) intrusive registry index | none | same |
| S1 | `NativeTrace` for `JsObject` (props, proto, closures, bound targets, home objects, private elements, `boxed_primitive`) and `Env` (vars, parent, binding object); cycle break; `$262.gc()` and `collect_garbage()` call it; opaque exotics left untraced | cyclic garbage freed on explicit gc only | gate + `closures`-style leak test + stress directories |
| S2 | Trace `Exotic` payloads: Map/Set, Promise reactions, Proxy, Generator frames and registers, ClassElements, DisposableStack, mapped Arguments, module records | more cycles freed | gate + stress over `language/`, `built-ins/Promise`, `Proxy`, generators |
| S3 | Weak semantics: ephemeron fixpoint, kept-alive list, FinalizationRegistry cleanup jobs, symbol keys (port from boa `weak_ref.rs`, `finalization_registry/mod.rs` with the upstream commit header and `THIRD_PARTY_NOTICES.md` row) | WeakRef/FR observe collection; expected +~66 files, no loss | gate on `built-ins/{WeakMap,WeakSet,WeakRef,FinalizationRegistry}` + whole suite |
| S4 | `GcHostRoots` trait, host wrapper/listener/timer registries trace themselves; weak wrapper cache | DOM-involved cycles freed | WPT dom smoke (`dom-b-audit`) zero lost, driver tests |
| S5 | Automatic trigger on by default, leaf skip, tuned thresholds | memory bounded for long-running pages | full gates + memory/time benchmarks below |

Each of S1-S4 is shippable alone because unknown edges only ever leak.

## 7. How to measure

Memory and time are both gates; the parked branch failed on them, so they are measured first (S0), not last.

1. **Memory benchmark** `scripts/js-bench.py` gains programs under `crates/amazing-js/benches/programs/`:
   `cycle_churn.js` (create and drop 1M two-node object cycles and 1M closure/env cycles, print checksum) and
   `weak_churn.js` (WeakRef + FinalizationRegistry over cyclic targets). Report peak RSS (already via `wait4`) with
   `AMAZING_JS_GC` off/on. Acceptance for S1: `cycle_churn` peak RSS flat (<= 1.5x baseline-live) instead of linear;
   existing `closures` (102 MB today) falls to the 15 MB class.
2. **Time**: the 12-program bench, median of 7, default mode: no program slower than +3% with GC code present and the
   collector idle (S0), and no program slower than +5% with auto-trigger on (S5).
3. **Test262**: full pinned suite file-by-file and variant-by-variant diff (same single-box harness used for
   vm-coverage/vm-default, plus the 16-shard gate for merge); total wall time <= baseline +5% (1196 s on the Boat box
   in the parked report); peak RSS and wall time on the five pathological files from the parked report
   (`staging/sm/Array/toSpliced-dense`, `Date/dst-offset-caching-1`, `JSON/parse-mega-huge-array`,
   `TypedArray/prototype/copyWithin/coerced-values-*-detached`, `regress/regress-610026`) within 1.2x of dev
   (326 MB / 21-52 s class), recorded as `bench_pathological` in the evidence.
4. **Soundness**: `AMAZING_JS_GC_STRESS=1` (collect at every safe point) over `language/{expressions,statements}`,
   `built-ins/{Promise,Proxy,Reflect,Object,Map,Set,Weak*}`; per-type trace tests; debug refcount assertion.
5. **Hosts**: WPT dom smoke (`crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py`, 14,558 passing subtests
   today) and driver tests with GC on for S4.

Evidence stays compact per AGENTS.md (counts, lost list, hashes; raw on the box).
