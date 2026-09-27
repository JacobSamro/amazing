# `notify_terminate` ordering + `takeUntil()` notifier mapping

Base `680d7659` (local `dev`; the prior `observable-takeUntil.any.js`
investigation, reverted, no code). Coordinator-assigned ticket, in order:
(1) add Observable crate-level unit tests covering `takeUntil` and
`notify_terminate` ordering, (2) fix the ordering and the `takeUntil` mapping
against those tests, with a full `dom/observable` WPT before/after in one
batch.

## Tests added (`crates/amazing-bindings/src/tests_observable.rs`)

No Observable unit test existed anywhere in this crate before this ticket --
the WPT corpus was the only signal, which is exactly how the earlier
initializer-throw fix attempt regressed nine unrelated files without either
gate noticing. Five tests, driven through a new `V::Script` mock-host
callable (a Rust closure run against the live `Host`, so a test can express
"this callback calls `subscriber.next()`/`.error()`/`.complete()`/
`.addTeardown()` on the exact `Subscriber` it was invoked with" without a JS
engine):

- `complete_runs_teardowns_before_forwarding_to_observers`
- `error_runs_teardowns_before_forwarding_to_observers`
- `take_until_notifier_error_completes_the_outer_instead_of_erroring`
- `take_until_notifier_completing_itself_does_not_terminate_the_outer`
- `take_until_notifier_next_completes_the_outer_and_never_subscribes_source`
  (guard: this mapping was already correct; proves the other four aren't
  trivially satisfied by a broken harness)

**Verified red before green**, per the ticket's ordering: ran the new tests
alone against `680d7659`'s unmodified `observable_bindings.rs` (temporarily
swapped in via `git show HEAD:... > observable_bindings.rs`, fix kept safe in
a separate copy, restored after -- no `git stash` used) --
**4 of 5 failed** exactly as predicted:
```
take_until_notifier_error_completes_the_outer_instead_of_erroring: left: [2], right: [3]
take_until_notifier_completing_itself_does_not_terminate_the_outer: left: [2], right: [1, 2]
error_runs_teardowns_before_forwarding_to_observers: left: [2, 1], right: [1, 2]
complete_runs_teardowns_before_forwarding_to_observers: (same shape)
```
the guard test passed both before and after. Restored the fix and reran: all
5 pass.

## Root causes fixed (`crates/amazing-bindings/src/observable_bindings.rs`)

Fetched and read the live WICG Observable spec text (`wicg.github.io/observable`)
rather than relying on recollection, since the prior ticket's investigation
had already shown this area is easy to get backwards.

1. **`notify_terminate` ordering.** `Subscriber#error()`/`#complete()` step 3
   is "close a subscription", which itself is: set inactive, signal-abort the
   subscription controller with the reason, then run teardown callbacks in
   reverse insertion order -- *all before* steps 4-5 iterate the internal
   observers and forward the event. This code forwarded to observers first,
   then ran teardowns, then aborted the signal -- backwards. Reordered to
   match: abort signal -> run teardowns -> forward to observers.
2. **`takeUntil()`'s notifier-event mapping.** The notifier's internal
   observer has *next steps* and *error steps* that both run the outer
   Subscriber's `complete()` (not `next()`/`error()`), and *no specified
   complete steps at all* (a no-op -- the spec's own note: "we do not need to
   complete the subscriber... the observable will continue to mirror
   sourceObservable uninterrupted"). This code treated notifier `Error` as an
   outer *error* and notifier `Complete` as an outer *complete*, both
   backwards. Fixed `relay_forward`'s `RelayKind::TakeUntilNotifier` arm:
   `Next | Error` -> outer completes; `Complete` -> no-op.

## Measured

Workspace gate (`venus-001`): `cargo nextest run --workspace --locked
--no-fail-fast` -- **3487 passed / 17 skipped / 0 failed** (3482 + 5 new
tests). `cargo test --workspace --locked --doc` -- all `ok`. Crate:
`amazing` + `amazing-bindings` + `amazing-dom` **741/741 passed, 6 skipped**
(736 + 5 new).

Full `dom/observable/tentative` directory (29 files, base `680d7659` vs this
candidate, freshly rebuilt release binary both sides, **two independent
`after` runs agree exactly** -- no flakiness this time, unlike the reverted
initializer-throw attempt):

| | before | after |
|---|---|---|
| total pass | 68 | **74** |
| total fail | 161 | 155 |
| total timeout | 0 | 0 |
| total error (whole-file) | 0 | 0 |

Per-file diffs (every other file in the directory is byte-for-byte
unchanged):

- `observable-constructor.any.js`: 24/20 -> 26/18
- `observable-filter.any.js`: 3/3 -> 4/2
- `observable-map.any.js`: 3/3 -> 4/2
- `observable-take.any.js`: 2/4 -> 4/2

**+6 subtests gained, 0 lost, 0 new timeouts, 0 new errors.**
`observable-takeUntil.any.js` itself is unaffected either way (still the
pre-existing whole-page `"[object]"` crash documented in the prior ticket --
a separate bug, not attempted here).

Evidence: `before.json`/`after.json` in this directory.
