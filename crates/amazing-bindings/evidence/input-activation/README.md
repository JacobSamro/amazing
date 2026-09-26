# DOM-B checkbox/radio native activation and onclick

Implementation on fixed base `47bcf7f04265faf53702c4dcba37fa4535ad3c84`. Lands the
checkbox/radio native click activation behavior (pre-activation, cancellation
rollback, indeterminate handling, radio grouping) together with the `onclick`
IDL event handler attribute, replacing the legacy copy-on-wrap `onclick`
extension for elements whose native prototype now defines it.

## What changed this ticket

- `HTMLElement.click()`/`onclick`, `HTMLInputElement.indeterminate` installed as
  native WebIDL members (`crates/amazing-bindings/src/input_activation_bindings.rs`,
  `interfaces.rs`, `host.rs`, `invoke.rs`, `lib.rs`).
- `amazing-dom`'s `input_activation` module: `InputActivation` snapshot captures
  checked/indeterminate and, for radios, the previously-checked peer **by its
  own identity** (name/form owner/root at prepare time) rather than by
  re-deriving the clicked node's live radio-group membership. This matters
  because a `click`/`onclick` handler can morph the control's `type` (or the
  peer's `name`) before calling `preventDefault()`; the WHATWG "canceled
  activation steps" must still apply as if nothing had changed
  (`dom/events/Event-dispatch-click.tentative.html`).
- `EventTargets::{event_handler, set_event_handler}` in `amazing-dom/src/events.rs`:
  a `handler` flag on `Listener` distinguishes `onX` registrations from
  `addEventListener` ones. Replacing an event handler attribute preserves its
  registration position; clearing then re-adding moves it to the end.
  `removeEventListener` never touches a handler-flagged entry.
- Fixed bug found while finishing the previous agent's WIP: the `onclick`
  setter rejected non-callable objects with a `TypeError`, and dispatch fell
  back to reading `handleEvent` on a non-callable handler. Per WebIDL,
  `EventHandler` is `EventHandlerNonNull?` where `EventHandlerNonNull` carries
  `[LegacyTreatNonObjectAsNull]`: assigning any object (callable or not) never
  throws and is stored as-is; assigning a non-object primitive (or
  null/undefined) clears the handler. At invocation, a non-callable handler is
  silently skipped and its `handleEvent` property is **never read**
  (`html/webappapis/scripting/events/event-handler-handleEvent-ignored.html`).
  `callback_call` in `event_bindings.rs` now branches on the `handler` flag
  before doing the `EventListener`-only `handleEvent` fallback.
- Native tests: `crates/amazing-dom/src/input_activation.rs` gained a
  type-morph regression test; `crates/amazing/src/runtime_host_dom_bindings.rs`
  gained `onclick_ordering_replacement_removal_and_non_callable_interop_addeventlistener`
  covering ordering against `addEventListener`, in-place replacement, clear/re-add
  reordering, the same function used as both an `onclick` and an
  `addEventListener` listener being two independent registrations,
  `removeEventListener` never removing the handler slot, `return false`
  cancellation (and non-`false` non-cancellation), thrown-handler continuation,
  and the non-callable/handleEvent-not-read case.

## Measured

Linux Boat pool-dom, isolated lane target. Workspace **2883 passed / 10
skipped**, `cargo test --workspace --locked --doc` all green (2 runs).

WPT `dom/events`, `html/semantics/forms`, `uievents` (whole directories,
testharness sources only), pinned `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`,
before = fixed base `47bcf7f0` rebuilt via `--ref`, after = this candidate:

**2472→2597 passing subtests (+125), 2091→1983 failing subtests, zero lost
passes** (Counter((file, testname)), duplicate subtest names counted).
File-level status flips (14 total) are all `fail`/`timeout` → `pass` or
`fail`, e.g. `html/semantics/forms/the-label-element/clicking-interactive-content.html`
and `html/semantics/forms/the-input-element/checkbox-click-events.html` now
pass in full. `before.json`/`after.json` are the raw `run-corpus.py` reports;
`compare-corpus.py` is the before/after diff tool (Counter-based, not a set).

Remaining failures in this selection are dominated by unrelated `uievents`
gaps already tracked elsewhere: worker-deadline timeouts on
`mouse`/`focus-events` ordering fixtures, `textInput` (`.sub.js` resource
substitution unsupported), and non-input activation (label/button/submit
targets beyond checkbox/radio, PointerEvent instead of MouseEvent for clicks,
declarative form submission) which this ticket does not claim.

## Reproduction

```
bash scripts/linux-run.sh dom-bindings --ref 47bcf7f04265faf53702c4dcba37fa4535ad3c84 --no-slot -- \
  'cargo build -p amazing --locked && mkdir -p "$OUT/dom-b-audit-input-activation-before" && \
   cp "$CARGO_TARGET_DIR/debug/amazing" "$OUT/dom-b-audit-input-activation-before/amazing" && \
   python3 crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py input-activation-before HEAD-47bcf7f0 \
     --reuse-binary --directory dom/events --directory html/semantics/forms --directory uievents'

bash scripts/linux-run.sh dom-bindings --no-slot -- \
  'cargo build -p amazing --locked && mkdir -p "$OUT/dom-b-audit-input-activation-final" && \
   cp "$CARGO_TARGET_DIR/debug/amazing" "$OUT/dom-b-audit-input-activation-final/amazing" && \
   cargo nextest run --workspace --locked --no-fail-fast && cargo test --workspace --locked --doc && \
   python3 crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py input-activation-final WIP-onclick-fix-v3 \
     --reuse-binary --directory dom/events --directory html/semantics/forms --directory uievents'

python3 crates/amazing-bindings/evidence/input-activation/compare-corpus.py before.json after.json
```

## Scope not covered here (queued)

- PointerEvent (click currently constructs `MouseEvent`; modern HTML specifies
  `PointerEvent`).
- Non-input activation targets: label proxying beyond the click-forwarding
  already covered, submit/reset button form submission, `<a>`/`<area>`
  navigation, popovers, `<input type=file>` pickers.
- Event handler *content attributes* (`onclick="..."` HTML attribute compiling
  to a function) — deferred to the JS compiler/live-object-environment seam
  (see `questions.md`).
