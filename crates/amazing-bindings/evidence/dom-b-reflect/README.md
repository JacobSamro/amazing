# DOM-B-REFLECT: ARIAMixin reflection, HTMLInputElement.switch, Element.animate(), navigator.clipboard

Implementation on fixed base `0654b5d3`. Lands four routed items in one ticket:

1. **ARIAMixin reflection** (`role` plus the full `aria-*` `DOMString?` set) on
   `Element`, for every namespace and tag.
2. **`HTMLInputElement.switch`**, a boolean reflection reusing the existing
   generic `LegacyReflectGet`/`LegacyReflectSet` mechanism.
3. **`Element.prototype.animate()`** returning a minimal `Animation` object
   (`finished`/`ready` promises, `cancel`/`finish`/`play`/`pause`, `onfinish`).
4. **`navigator.clipboard`** with `readText`/`writeText`, backed by an
   in-memory, per-document clipboard.

## What changed this ticket

- `crates/amazing-bindings/src/aria_reflection.rs` (new): `Operation::AriaGet`/
  `AriaSet(&'static str)` bake the content-attribute name directly into the
  operation. Unlike `element_reflection.rs`'s `LegacyReflectGet`/`Set`, which
  gate on a specific tag or `Interface`, this path only checks "is an
  `Element`" -- ARIAMixin applies regardless of namespace or tag. Getter
  returns `null` when the attribute is absent (not `""`, unlike the existing
  `Kind::String`/`LegacyString` reflections); setter removes the attribute on
  `null`/`undefined`. Confirmed against `html/dom/aria-attribute-reflection.html`
  read in full.
  - **Scope cut**: the element-reference attributes (`ariaLabelledByElements`
    and friends, `FrozenArray<Element>?`/`Element?`) are not implemented. Per
    spec they need an explicitly-set-element override that the accessible
    name/description computation (`amazing-a11y`, the a11y lane) would have
    to consult ahead of any ID-ref fallback; an ID-ref-only getter would
    silently diverge from that algorithm. Routed to a11y coordination rather
    than half-implemented -- see `questions.md`.
- `crates/amazing-bindings/src/interfaces.rs`: `ELEMENT` gained `animate()`
  plus `role` and all 41 `aria*` getter/setter pairs; `INPUT` gained
  `switch` (`O::LegacyReflectGet/Set(I::HTMLInputElement, "switch", Boolean)`,
  the same mechanism already used for `align`/`useMap`). New `ANIMATION` and
  `CLIPBOARD` member tables and `INTERFACES` entries (`constructor: None` --
  neither is directly constructible; only reachable via `animate()` /
  `navigator.clipboard`).
- `crates/amazing-dom/src/animation.rs` (new): a `Dom`-owned, id-keyed
  `Animations` registry (mirrors `abort.rs`/`observable.rs`'s pattern), storing
  play state and the `finished`/`ready`/`onfinish` `CallbackId` roots. An
  animation is created already in the `Finished` play state -- this ticket's
  scope is explicitly *not* the full Web Animations timing model (no
  timeline, no keyframe/effect application, no pending-then-later-resolve
  scheduling), just enough that real pages calling `.animate(...)` and then
  using the returned object's promises/methods stop throwing "value is not
  callable" (`app-kanban.html` in IA's real-pages corpus).
- `crates/amazing-bindings/src/animation_bindings.rs` (new): `Element.animate()`
  and the `Animation` method/getter/setter set. `finished`/`ready` resolve
  synchronously (at creation) to the `Animation` itself. `cancel()` replaces
  the promise the `finished` getter returns with a freshly-rejected
  (`AbortError`) one and is a no-op once already idle. `finish()` sets the
  play state and invokes `onfinish` (synchronously, via `host.call`) every
  time it is called, matching a real re-dispatched `finish` event.
- `crates/amazing-bindings/src/clipboard_bindings.rs` (new) +
  `crates/amazing-dom/src/lib.rs` (`Dom::clipboard_text`/`set_clipboard_text`):
  a `Clipboard` singleton per document arena (`PlatformKey { slot: 0, .. }`),
  `readText`/`writeText` round-tripping through `Dom`'s new `clipboard_text`
  field. Not real OS clipboard integration -- same-page/same-session round
  trips only, which is what real pages exercise.
- `crates/amazing/src/runtime_host_dom_bindings.rs`: `install_navigator_clipboard`,
  mirroring the storage lane's lazy `navigator.<singleton>` getter pattern in
  `runtime_host_storage.rs` (get-or-create the ad-hoc `navigator` object, then
  a `CallOnly` accessor) -- `navigator` is not a WebIDL interface installed
  through `interfaces.rs`, so this can't go through the normal table. Wired
  into `crates/amazing/src/runtime_binding_host.rs`'s shared `install()` with
  one new call line, per the driver-adapter-per-lane convention.
- `crates/amazing-bindings/src/invoke.rs`: three new `handles`/`invoke`
  dispatch checks (`aria_reflection`, `animation_bindings`,
  `clipboard_bindings`), and `crates/amazing-bindings/src/lib.rs` gained the
  three `mod` lines.
- Native tests: `crates/amazing-bindings/src/tests_dom_b_reflect.rs` (new, 5
  tests) exercises all four features directly at the binding-op level using
  this crate's mock `Host`, including reading back `Promise` resolution
  values (`finished`/`ready` resolve to the `Animation`; `readText` resolves
  to the written text) that the JS-level test harness in `crates/amazing`
  cannot observe without a real microtask-pumping scheduler (those tests use
  `Interpreter::new_with_external_scheduler`, which does not auto-drain
  jobs). Covers the receiver-type-check rejection path for every new
  operation, `cancel()`'s idle-is-a-no-op rule, `onfinish` firing
  synchronously on `finish()`, and the clipboard singleton being stable
  across repeated `navigator.clipboard` reads.

## Measured

Linux k-test (root@159.69.124.56), isolated lane tree, fixed base `0654b5d3`.

Workspace **3378 passed / 17 skipped / 0 failed** (`cargo nextest run
--workspace --locked --no-fail-fast`); `cargo test --workspace --locked --doc`
all green, `amazing_js` and `amazing_html` each 1/1 as their AGENTS.md-flagged
doctest suites. `amazing-bindings` crate: 122/122 including the 5 new tests.

WPT `html/dom` (whole directory), `wai-aria`, `web-animations/interfaces/Animatable`
(whole directories, testharness sources only), pinned
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, before = fixed base `0654b5d3`
rebuilt via `--ref`, after = this candidate:

**10902->10946 passing subtests (+44 net), 3561->3380 failing subtests.**
Raw Counter diff: 178 gained, 134 lost. **Every one of the 134 "lost"
subtests is page-timing noise, not a regression**: all 134 are in five
pre-existing, untouched `html/dom/reflection-*.html` pages (auto-generated
`reflection.js` harnesses with hundreds to thousands of subtests per page,
run under the WPT test runner's 20s per-page wall-clock budget). Each such
page reports identical `fail` counts before and after -- only its `pass`
count drops, because the run got less machine time before the deadline fired
and completed fewer subtests, both reporting `status: "timeout"` /
`"worker deadline exceeded; completed subtests retained"`:

| file | before pass/fail | after pass/fail |
|---|---|---|
| `html/dom/reflection-embedded.html` | 721 / 388 | 701 / 388 |
| `html/dom/reflection-forms-weekmonth.html` | 1203 / 87 | 1188 / 87 |
| `html/dom/reflection-forms.html` | 1145 / 116 | 1127 / 116 |
| `html/dom/reflection-grouping.html` | 1180 / 114 | 1142 / 114 |
| `html/dom/reflection-misc.html` | 968 / 256 | 940 / 256 |

None of the attributes in the lost subtests (`img.decoding`, `input.size`,
`legend.lang`, `blockquote.title`, `pre.width`, `noscript.className`, ...)
are touched by this ticket. The 178 gained subtests show the identical
pattern in the other direction on different pages
(`html/dom/reflection-obsolete.html`'s `frameset.accessKey`,
`html/dom/reflection-tabular.html`'s `colgroup.hidden`, also untouched).
**Zero of the 134 lost subtests touch anything this ticket changed; 158 of
the 178 gained do** (`aria*`/`role`/`animate`). Two files flip status
outright: `html/dom/aria-attribute-reflection.html` and
`html/dom/aria-attribute-reflection.tentative.html`, both `fail -> pass`.

`input.switch` and `navigator.clipboard` have no automated coverage in this
pinned WPT checkout (`wai-aria/manual/switch_*-manual.html` are ARIA-role
manual tests, unrelated to the IDL attribute) -- covered instead by the new
native tests. `web-animations/interfaces/Animatable` mostly fails/errors
identically before and after: `animate.html` and
`getAnimations-pseudoElement-option-on-view-transition.html` error on
resources this sparse WPT checkout doesn't vendor
(`easing-tests.js`, `css-animations/support/testcommon.js`), and
`getAnimations`/`getAnimations-iframe` exercise the full timing/query model
this ticket does not implement -- not a regression, unchanged before/after.

`before.json`/`after.json` are the raw `run-corpus.py` reports;
`compare-corpus.py` is the before/after diff tool (Counter-based, not a set,
copied unmodified from `evidence/input-activation/`).

## Reproduction

```
bash scripts/linux-run.sh dom-bindings --ref 0654b5d3 --no-slot -- \
  'cargo build -p amazing --locked && mkdir -p "$OUT/dom-b-audit-reflect-before" && \
   cp "$CARGO_TARGET_DIR/debug/amazing" "$OUT/dom-b-audit-reflect-before/amazing" && \
   python3 crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py reflect-before HEAD-0654b5d3 \
     --reuse-binary --directory html/dom --directory wai-aria --directory web-animations/interfaces/Animatable'

bash scripts/linux-run.sh dom-bindings --no-slot -- \
  'cargo build -p amazing --locked && mkdir -p "$OUT/dom-b-audit-reflect-after" && \
   cp "$CARGO_TARGET_DIR/debug/amazing" "$OUT/dom-b-audit-reflect-after/amazing" && \
   python3 crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py reflect-after WIP-dom-b-reflect \
     --reuse-binary --directory html/dom --directory wai-aria --directory web-animations/interfaces/Animatable'

python3 crates/amazing-bindings/evidence/dom-b-reflect/compare-corpus.py before.json after.json
```

## Scope not covered here (queued)

- ARIAMixin element-reference attributes (`ariaLabelledByElements`,
  `ariaControlsElements`, etc.) -- needs an a11y-consulted explicitly-set
  override; see `questions.md`.
- `Element.animate()`'s full Web Animations model: timeline, keyframe/effect
  application, real timing-driven `finished`/`ready` settlement,
  `getAnimations()`.
- Real OS clipboard integration (paste from outside the page, clipboard
  permissions, non-text clipboard items).
