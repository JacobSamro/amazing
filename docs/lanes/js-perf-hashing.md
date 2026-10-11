# JS performance ticket 2: fast hashing, small-object scan, cached key strings

Chosen from the ranked list in [js-perf-baseline.md](js-perf-baseline.md) (items 1 and 2: SipHash was 12-14% and
string-to-`Vec<u16>` key conversion plus malloc/free 20-23% of profile samples). Whole area, behaviour neutral.

## What changed (crates/amazing-js/src)

- `fast_hash.rs`: multiply-rotate hasher (FxHash family, written from scratch) with a per-process random seed;
  `FastMap`/`FastSet` aliases. Used for `JsObject::prop_index` and the environment records
  (`Env::vars`, global `lexicals`). Iteration order was already unspecified under `RandomState`.
- `value.rs`: objects with at most 8 own properties have no hash index and are scanned linearly
  (`slot_of`); the index is built when the ninth property arrives and dropped again when deletes bring the
  object back to 8. Fewer allocations per object and no hashing for the common small object.
- `value.rs`: `JsString::from(&str)` for short ASCII strings (at most 32 bytes) comes from a 1024-slot
  direct-mapped per-thread cache, so `length`, `prototype`, identifiers and similar share one `Rc<[u16]>`;
  other ASCII strings use a single allocation (was `Vec` then `Rc` copy). The table is `ManuallyDrop` so a
  coroutine thread exiting never touches strings another thread still holds.
- Risk accepted: the hasher is not a keyed PRF, so it is weaker against deliberate collision flooding than
  SipHash. Keys are limited to engine tables keyed by script-supplied names; the seed removes cross-run
  predictability only. Revisit if a hostile-key benchmark shows quadratic behaviour.

## Results (same box, same binary build flags, median of 5, in-process ms)

default = shipped path (top level on the tree walker); wrapped = same program inside an IIFE (bytecode loops).
Geometric mean speedup: default 1.578x, wrapped 1.473x. All checksums unchanged and equal to Node's.

| Program | default before | after | speedup | wrapped before | after | speedup | RSS MB |
|---|---|---|---|---|---|---|---|
| array_loop | 235.7 | 141.1 | 1.67x | 111.6 | 74.7 | 1.50x | 17.8 -> 16.8 |
| class_inherit | 228.8 | 146.4 | 1.56x | 167.4 | 113.2 | 1.48x | 15.1 -> 15.2 |
| closures | 264.5 | 172.4 | 1.53x | 165.5 | 117.0 | 1.42x | 102.4 -> 87.5 |
| fib | 329.3 | 234.3 | 1.41x | 268.0 | 205.9 | 1.30x | 15.4 -> 15.4 |
| json_roundtrip | 110.4 | 64.0 | 1.73x | 107.7 | 62.1 | 1.73x | 15.1 -> 15.4 |
| map_set | 105.2 | 78.1 | 1.35x | 64.7 | 56.3 | 1.15x | 15.1 -> 15.4 |
| method_calls | 413.0 | 251.5 | 1.64x | 263.6 | 180.8 | 1.46x | 15.1 -> 15.4 |
| prop_access | 272.5 | 160.0 | 1.70x | 181.9 | 92.7 | 1.96x | 15.1 -> 15.4 |
| regexp | 209.2 | 121.8 | 1.72x | 164.1 | 95.8 | 1.71x | 15.4 -> 15.4 |
| scheduler | 281.9 | 173.0 | 1.63x | 279.1 | 173.5 | 1.61x | 15.4 -> 15.4 |
| string_ops | 220.8 | 136.1 | 1.62x | 88.7 | 64.7 | 1.37x | 15.4 -> 15.4 |
| typed_array | 451.0 | 315.2 | 1.43x | 146.1 | 122.7 | 1.19x | 15.4 -> 15.4 |

Per-mode (`vm`, `ast`) numbers are in [js-perf-hashing-evidence.json](js-perf-hashing-evidence.json).
No program got slower. The remaining profile is expected to be dominated by global/scope lookups and the call
path (items 3 and 4 of the baseline ranking).
