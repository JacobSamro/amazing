# `Element`/`ShadowRoot.setHTMLUnsafe` -- implement the callable method

Base `fa514dc9` (local `dev`, includes the `innerhtml-on-ordinary-template`
root-cause + `sync_window_named_access` hook). Next item taken from this
lane's queue (`declarative-shadow-dom-basic.html`'s remaining "value is not
callable" failures).

## Root cause

`setHTMLUnsafe(html, options?)` (HTML sanitizer API) was entirely absent --
no `Operation` variant, no interface member -- so every WPT test calling it
crashed with `TypeError: value is not callable`, aborting the whole test
file before any of its assertions ran.

## Fix

Added `O::ElementSetHTMLUnsafe` (`host.rs`), routed in
`markup_bindings.rs` into the *exact same* fragment-replacement body as
`O::ElementSetInnerHTML` (this engine has no sanitizer either way, so
"skip sanitizing" and the ordinary setter currently behave identically),
and declared `method("setHTMLUnsafe", O::ElementSetHTMLUnsafe, 1)` on both
`ELEMENT` and `SHADOW_ROOT` in `interfaces.rs` (the spec mixin applies to
both). Options are ignored (no sanitizer to apply).

## What's still missing (escalated, not fixed here)

`amazing_html::parse_fragment` (`crates/amazing-html/src/lib.rs`) has no
way to enable declarative shadow roots -- `build_fragment` in `tree.rs`
hardcodes `allow_declarative_shadow_roots = false` for every fragment
parse. Per spec, `setHTMLUnsafe`/`parseHTMLUnsafe` should have this
enabled. Routed to `questions.md` with a proposed non-breaking API
(`parse_fragment_with_shadow_roots`, a sibling function, so no other
caller needs to change) -- `amazing-html` is outside this lane's granted
paths.

## Measured

`declarative-shadow-dom-basic.html` (venus-001, freshly rebuilt release
binary both times, fixed base `fa514dc9`): `fail{pass:15,fail:7}` ->
`fail{pass:17,fail:5}` -- the 7 crashes are gone; 2 now genuinely pass, 5
fail for the exact, expected, already-diagnosed reason above (e.g. "No
shadow root found expected true got false", "expected null but got
Element node \<template shadowrootmode=...\>").

Whole-`shadow-dom/declarative` directory before/after (56 files):
**`declarative-shadow-dom-attachment.html`: 0/654 pass -> 586/68 pass** --
this file makes heavy use of `setHTMLUnsafe` across many subtests, and the
overwhelming majority now pass for real once the method exists.

**A note on `gethtml.html`'s numbers, to avoid a misleading "net loss"
reading**: a naive gained/lost subtest count across the whole directory
shows 588 gained, 608 lost. All of the "lost" comes from `gethtml.html`
alone, an auto-generated page with thousands of subtests that hits a hard
wall-clock worker deadline (`"worker deadline exceeded; completed subtests
retained"`) -- its retained-pass-count is a function of how many subtests
happened to complete before the timer fired, not of correctness. Measured
directly, back to back on the same binary: 1216, 608, and 530 pass on
three consecutive runs of the identical file with no code change between
them. This file is not usable as a before/after signal under this
harness's current timeout model; the 608 "lost" here is that noise, not a
regression. `declarative-shadow-dom-basic.html` and
`declarative-shadow-dom-attachment.html` (both far below the timeout, not
timing-sensitive) are the real signal for this change, and both improved
with zero contrary evidence anywhere else in the 56-file selection.

## Gate

Workspace (`venus-001`): `cargo nextest run --workspace --locked
--no-fail-fast` -- **3482 passed / 17 skipped / 0 failed**. `cargo test
--workspace --locked --doc` -- all `ok`. Crate:
`cargo nextest run -p amazing -p amazing-bindings -p amazing-dom --locked`
-- **736/736 passed, 6 skipped**.

Evidence: `before.json`/`after.json` in this directory.
