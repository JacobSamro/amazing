# Coordinator follow-up: sparse-checkout fix + HTMLTemplateElement reflection

Base `4c512a79` (local `dev`, includes `cd57f3bf`'s `sequence_shadow_roots` +
file-move). Fixes both gaps that ticket's evidence flagged as unaddressed:

1. **`run-corpus.py` sparse checkout**: added `html/resources` and `webrtc`
   to a new `SHARED_RESOURCE_DIRECTORIES` list (was inlined as
   `['resources', 'common', ...]` at both `sparse_checkout` call sites).
   `shadow-dom/declarative/gethtml.html` and its siblings
   (`declarative-shadow-dom-attachment.html`, `-opt-in.html`,
   `-repeats.html`, plus a long tail of `shadow-dom/untriaged/**` files that
   share the same `../../html/resources/common.js` helper) now run instead
   of erroring before any assertion. `html/dom/usvstring-reflection.https.html`
   (needs `webrtc/RTCPeerConnection-helper.js`) is the other file this was
   found to fix.
2. **`HTMLTemplateElement.shadowRootMode`/`shadowRootDelegatesFocus`/
   `shadowRootClonable`/`shadowRootSerializable`** (HTML SS4.12.4): added a new
   `LegacyReflectionKind::LimitedKnownValues(&'static [&'static str])`
   variant (getter-only value limiting, default `""`, since WebIDL's
   reflection algorithm only limits the getter -- the setter is a plain
   passthrough) for `shadowRootMode`; the other three reuse the existing
   `LegacyReflectionKind::Boolean` machinery unchanged. All four wired via
   the existing generic `O::LegacyReflectGet`/`LegacyReflectSet(Interface,
   attribute, kind)` pattern already used for `HTMLInputElement`'s `switch`
   and dozens of other legacy-reflected attributes -- zero new bespoke
   `Operation` variants needed.

## Measured (venus-001, `--directory shadow-dom --directory html/dom`, whole
directories, 509 files selected)

Both runs used a **freshly built `cargo build -p amazing --release --locked`**
binary (`run-corpus.py` only copies whatever's already in
`$CARGO_TARGET_DIR/release/amazing`; it does not build it -- an easy trap,
caught here by re-running after rebuilding and seeing the numbers move).

| | before (`4c512a79`, that commit's own `run-corpus.py`) | after (this candidate) |
|---|---|---|
| files selected | 509 | 509 |
| subtests gained | -- | **1621** |
| subtests lost | -- | **0** |
| status: error -> pass | -- | 34 files |
| status: error -> fail | -- | 29 files |
| status: error -> timeout | -- | 2 files |

`shadow-dom/declarative/declarative-shadow-dom-basic.html` (the reflection
fix's most direct target): `fail {pass: 7, fail: 15}` -> `fail {pass: 15,
fail: 7}` -- exactly the four newly-reflected attributes' getter/setter
subtests (mode, delegatesFocus, clonable) flipping to pass. Its remaining 7
failures are all `"value is not callable"` -- a distinct, unrelated missing
API, not chased here (out of this ticket's two assigned items).

`shadow-dom/declarative/gethtml.html`: `error` (missing resource) ->
`timeout` with **828 pass / 2062 fail** retained (this auto-generated page
has thousands of subtests and hits the harness's per-test deadline; it at
least now runs and produces real signal instead of erroring before any
assertion). The 34 `error -> pass` and 29 `error -> fail` files are the rest
of the `shadow-dom/untriaged/**` tree that shares the same
`../../html/resources/common.js`/`../../../html/resources/common.js`
dependency at various relative depths.

## Gate

Workspace (`venus-001`): `cargo nextest run --workspace --locked
--no-fail-fast` -- **3482 passed / 17 skipped / 0 failed**. `cargo test
--workspace --locked --doc` -- all `ok` (`amazing_js`/`amazing_html` 1/1
each). Crate tests: `cargo nextest run -p amazing-bindings -p amazing-dom
--locked` -- **372/372 passed**.

## Found in passing, not implemented this ticket

`shadow-dom/declarative/innerhtml-on-ordinary-template.html` errors with
`ReferenceError: ordinarytemplate is not defined` -- routed to
`questions.md`. Likely either a declarative-shadow-attach-skip-when-host-
already-has-a-shadow-root gap, or a window-named-access gap specific to
`<template>` elements; not triaged further, not one of this ticket's two
assigned items.

Evidence: `before.json`/`after.json` in this directory.
