# JS performance: interned keys, environment slots, call path

Base `d813a2177` (includes the hashing ticket). Behaviour neutral. Compact evidence:
[js-perf-interned-keys-call-path-evidence.json](js-perf-interned-keys-call-path-evidence.json); raw results stay on the
Boat box under `~/amazing/out/r2-jsperf/perf1` (sha256 in the evidence file).

## What changed (crates/amazing-js/src)

- `env_vars.rs`: small linear `VarTable` for environment frames (hash map only past a size threshold);
  `Env::fast_cell`/`fast_read` and `Reference::Cell` for identifier read, call and assign, including global
  script lexicals and plain global-object vars.
- `JsString::interned` and `Ident.key`: member-site and VM-compile names are interned once instead of converted
  per evaluation; numeric literal parse cache; `JsString::from_index` cache and a `to_property_key` fast path.
- `JsObject::plain_get` / `plain_set` / `plain_own_data`: fast paths for `get_prop_receiver` and
  `set_with_receiver` on plain objects whose chain holds only data properties. The prototype walk is capped at
  32 hops; longer chains take the general path, which admits each step (needed by
  `call_resource_tests::callable_proxy_trap_and_native_metadata_lookup_are_admitted_at_each_property_entry`).
- Call path: `JsFunction::clone_for_call` (no params/name clone for native/bytecode callees), a simple-parameter
  fast path in `bind_params`, the implicit `arguments` object is skipped when the function source never mentions
  `arguments`, `eval` or a unicode escape, `function_names` only built for expression parameters, debug-hook
  clone only when a hook is set.
- Not done: compile-time (depth,slot) resolution and per-site inline caches (next ticket).

## Results (same box, median of 7, in-process ms, before = d813a2177)

Geometric mean speedup: default 1.486x, wrapped 1.972x. No program slower. All checksums agree.

| Program | default before | after | speedup | wrapped before | after | speedup |
|---|---|---|---|---|---|---|
| array_loop | 256.47 | 140.57 | 1.82x | 135.97 | 49.47 | 2.75x |
| class_inherit | 232.06 | 103.56 | 2.24x | 189.88 | 58.99 | 3.22x |
| closures | 293.48 | 153.9 | 1.91x | 149.6 | 73.29 | 2.04x |
| fib | 286.14 | 171.02 | 1.67x | 244.61 | 111.34 | 2.20x |
| json_roundtrip | 77.62 | 53.28 | 1.46x | 74.79 | 51.65 | 1.45x |
| map_set | 91.03 | 83.92 | 1.08x | 66.97 | 52.87 | 1.27x |
| method_calls | 298.47 | 207.33 | 1.44x | 207.52 | 95.11 | 2.18x |
| prop_access | 193.77 | 151.83 | 1.28x | 113.8 | 49.69 | 2.29x |
| regexp | 152.57 | 138.22 | 1.10x | 120.84 | 99.64 | 1.21x |
| scheduler | 212.0 | 87.49 | 2.42x | 212.91 | 83.53 | 2.55x |
| string_ops | 160.93 | 147.34 | 1.09x | 79.39 | 43.45 | 1.83x |
| typed_array | 350.43 | 333.9 | 1.05x | 146.11 | 86.65 | 1.69x |

## Gate

Rebased on origin/dev 65d26cc94. Workspace nextest 5768 passed, doctests ok. Test262 (single box, timeout 60) vs the
d813a2177 run: default 53,366 files / 102,507 variants, lost 0 / 0, gained 100 / 200 (dev's own gain); `AMAZING_JS_VM=1`
53,360 / 102,495, lost 0 / 0, gained 100 / 200. An earlier run lost `staging/sm/regress/regress-1507322-deep-weakmap.js`
to the 60 s deadline: it takes 70-80 s standalone with both the old and new binary (linear scan of the WeakMap
entry vector, js-stdlib lane), so it is load-dependent, not a regression.
