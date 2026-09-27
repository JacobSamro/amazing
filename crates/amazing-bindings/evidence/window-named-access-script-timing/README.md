# `innerhtml-on-ordinary-template.html` ReferenceError: root cause and partial fix

Base `ffdde5a3` (local `dev`, includes `5c9a6cf8`'s sparse-checkout fix +
`HTMLTemplateElement` reflection). Coordinator ticket: root-cause and fix
`shadow-dom/declarative/innerhtml-on-ordinary-template.html`'s
`ReferenceError: ordinarytemplate is not defined`.

## Two hypotheses from the prior ticket's escalation, both re-examined

1. **Declarative-shadow-attach-skip-when-host-already-has-a-shadow-root.**
   Already correct: `Dom::attach_declarative_shadow`
   (`crates/amazing-dom/src/shadow.rs`) returns `NotSupportedError` when the
   host already has a shadow root; `tree.rs::start_template` turns that
   `Err` into `None` via `.ok()` and falls back to inserting an ordinary
   `<template>` element. Confirmed empirically (see below) -- not the bug.
2. **Window named access specific to `<template>`.** Not the real shape of
   the bug either: it's not about `<template>` at all.

## Minimal repro (not committed -- ad hoc, run directly through the
`wpt-harness` binary)

Two standalone probe pages, run via `amazing wpt-harness <scratch-root>
<page> 800 600 5000` against a scratch directory with a `.pinned-<sha7>`
marker file (bypasses the pinned-checkout requirement that a real WPT root
needs, for a non-WPT ad hoc test root) and a copy of `resources/` (a
symlink fails a `starts_with(root)` escape check after `canonicalize()`
resolves it outside the scratch root):

- **probe1**: `<div id=x></div>` immediately followed by a sibling
  `<script>window.__probe1 = typeof x</script>`, with **zero** DOM API
  calls in between. Result: `typeof x` is `"undefined"`, not `"object"`.
- **probe2**: the exact `innerhtml-on-ordinary-template.html` shape
  (MutationObserver attaches an imperative shadow root, then a
  `<template shadowrootmode=open id=ordinarytemplate>` is left over,
  then a script reads the bare identifier). `host.shadowRoot` is
  correctly non-null (confirms hypothesis 1 is not the bug) but
  `typeof ordinarytemplate` is still `"undefined"`.

## Root cause

`RuntimeBindingHost::with_dom`'s dirty-flag check
(`crates/amazing/src/runtime_binding_host.rs`) is the *only* place window's
named properties (HTML SS7.2.2.3) get recomputed, and it only runs when
*some* DOM-mutating JS binding call happens. A parser-driven insertion
(`amazing-html`'s tree builder inserting an `id`/`name`-bearing element
directly, never through a JS binding call) sets `Dom`'s
`named_objects_dirty` flag without ever passing through `with_dom`. If the
very next thing that runs is a script whose first statement is a bare
global-identifier read with no DOM call before it, nothing has triggered
the refresh yet -- ordinary global-identifier lookup has no hook into this
crate at all (a deliberate design choice: no engine-level `[[Get]]` hook
into `amazing-js`, per `with_dom`'s own comment).

## What's fixed here vs. what's escalated

Added `RuntimeBindingHost::sync_window_named_access(&mut self) ->
Result<(), Throw>` (`pub(crate)`, `runtime_binding_host.rs`): a one-line
wrapper around `self.with_dom(|_| Ok(()))`, triggering the exact same
dirty-check-then-refresh `with_dom` already does. This is ready to call,
but **not wired to a call site** -- every place that runs a
parser-encountered `<script>` lives in `runtime_host_html.rs` (html lane's
file) and `amazing-html`'s tree builder is also outside this lane's
granted paths, so the actual fix needs a one-line call added at each
`run_script`/`i.run_script` site there (listed with line numbers in
`questions.md`) immediately before the script executes. Routed to
`questions.md` for html lane / the integrator.

## Gate

Workspace (`venus-001`): `cargo nextest run --workspace --locked
--no-fail-fast` -- **3482 passed / 17 skipped / 0 failed**. `cargo test
--workspace --locked --doc` -- all `ok`. Crate:
`cargo nextest run -p amazing -p amazing-bindings -p amazing-dom --locked`
-- **736/736 passed, 6 skipped**.

No WPT corpus number attached to this ticket: the fix is not complete
(the hook exists but has no call site yet), so `innerhtml-on-ordinary-
template.html` is still expected to fail identically until html lane wires
it in. Re-measuring now would show zero change and give false confidence
that nothing happened; the real signal will come once the call site lands.
