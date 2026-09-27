# Wire `parse_fragment_with_shadow_roots` into `setHTMLUnsafe`/`Document.parseHTMLUnsafe`

Base `1da899a5` (local `dev`; the Observable ordering/`takeUntil` fix and
`parse_fragment_with_shadow_roots` from html lane are both already in).
Coordinator-assigned ticket: wire html lane's new
`amazing_html::parse_fragment_with_shadow_roots` into
`Element`/`ShadowRoot.setHTMLUnsafe` and `Document.parseHTMLUnsafe`, then
remeasure `shadow-dom/declarative`.

## Spec research

Fetched the live HTML spec (`html.spec.whatwg.org/multipage/dynamic-markup-
insertion.html` and `.../parsing.html`) rather than relying on html lane's
informal (if careful) description, since getting "replace all" vs. "attach
declarative shadow root" exactly right matters:

- `Element`/`ShadowRoot.setHTMLUnsafe` both route through "set and filter
  HTML", which always parses with the HTML fragment parsing algorithm's
  `allowDeclarativeShadowRoots = true` (`innerHTML`/`outerHTML`/
  `insertAdjacentHTML` never pass `true`, unchanged here).
- The HTML fragment parsing algorithm's own `context` is the *host itself*
  when the fragment target is an Element, or the host's ShadowRoot's host
  when the target is a ShadowRoot -- and `adjusted current node` is defined
  to *be* that context element whenever the parser's stack of open elements
  has only the synthetic root in it (the fragment case). A top-level
  `<template shadowrootmode>` therefore should attach to that real host per
  spec -- but html lane's `parse_fragment_with_shadow_roots(context: &str,
  ...)` only ever receives `context` as a tag-name string, inside a
  standalone `Dom` with no real `NodeIdx` for that host to attach to. That
  gap is exactly why html lane scoped their function to the nested-ancestor
  shape only and left the top-level shape to this lane (`questions.md`).
- `Document.parseHTMLUnsafe` is a *full-document* parse (`document's allow
  declarative shadow roots` set to `true`, then "parse HTML from a string"),
  not a fragment parse. `amazing_html::tree::TreeBuilder::new()` (every
  full-document parse) already defaults `allow_declarative_shadow_roots:
  true` unconditionally, and a full-document parse always has real ancestor
  elements genuinely on the open-elements stack (`<html>`, `<body>`, ...) by
  the time ordinary body content starts, so `start_template`'s existing
  `self.open.len() > 1` nested-shape handling already covers a template
  anywhere in the document, including directly under `<body>`. This needed
  no new parser support at all -- just an entry point that creates a new
  inert `text/html` Document, which `create_inert_document` (used by
  `DOMParser.parseFromString`) already is.

## Fix

- `FragmentParser::parse_fragment_with_shadow_roots` (new trait method,
  `host.rs`, defaults to plain `parse_fragment` so no other host regresses)
  and its `RuntimeBindingHost` impl (`runtime_binding_host.rs`), calling
  `amazing_html::parse_fragment_with_shadow_roots`.
- `markup_bindings.rs`: `O::ElementSetHTMLUnsafe` alone now calls the new
  method (the other three markup ops are unchanged). After parsing,
  `top_level_declarative_shadow_template` detects the fragment's first
  top-level child being a `<template shadowrootmode="open"|"closed">`
  (mirroring `start_template`'s own attribute reading) and, if found,
  attempts `Dom::attach_declarative_shadow` on the real `context_node`
  already computed for the parser context; on success the template's own
  `template_contents` children move into the new shadow root (via a new
  `Dom::import_subtree_children` helper in `amazing-dom/src/markup.rs`,
  since `import_fragment` always creates a fresh top-level
  `DocumentFragment` from *all* of a source's top-level children rather
  than an arbitrary subtree into an arbitrary target) and the template is
  detached from the parsed fragment (`Dom::pre_remove`) before the
  unmodified `import_fragment` path imports whatever top-level siblings
  remain. On failure (already a shadow host, or an invalid host element
  name), nothing is detached, so the template falls through unchanged as
  ordinary light-DOM content -- matching the spec's own exception-catch
  step.
- `Document.parseHTMLUnsafe(html, options?)`: new static `O::
  DocumentParseHTMLUnsafe`, registered on `I::Document`
  (`interfaces.rs`), dispatched in `parser_bindings.rs` by calling the
  existing `create_inert_document(bytes, "text/html", url, "UTF-8")`
  primitive `DOMParser.parseFromString` already uses -- no new parsing
  logic needed, per the spec research above. `options` (a sanitizer
  config) is ignored, matching `ElementSetHTMLUnsafe`'s own existing scope
  note (this engine has no sanitizer).

## Tests added (`crates/amazing-bindings/src/tests_set_html_unsafe.rs`)

No test exercised the top-level shadow shape at all before this ticket --
only the WPT corpus did, which is exactly why the gap existed unnoticed.
Three tests via a new `Host::parse_fragment_with_shadow_roots` sentinel-
markup override (`tests.rs`; a real HTML parser isn't needed to test this
plumbing -- `amazing-html`'s own tests and the WPT gate below cover real
parsing):

- `set_html_unsafe_attaches_a_top_level_declarative_shadow_root`
- `set_html_unsafe_top_level_shadow_shape_keeps_later_siblings_as_ordinary_content`
- `set_html_unsafe_top_level_shadow_shape_falls_back_when_already_a_shadow_host`

Plus one for the new static method, next to the existing `DOMParser`
coverage in `tests.rs`: `document_parse_html_unsafe_creates_an_inert_html_document`.

**Verified red before green** (temporarily reverted the six production
files to `HEAD` via `git show HEAD:path > path`, kept the fix in `/tmp`, no
`git stash`): all three failed against the unfixed code with the expected
messages (`shadow_root_of_host(host).is_some()` false; template not found
as ordinary content when the fallback should have kept it), then restored
the fix and reran: all three pass.

## Measured

Workspace gate (`venus-001`), run twice for stable numbers: `cargo nextest
run --workspace --locked --no-fail-fast` -- **3500 passed / 17 skipped / 0
failed** both times. `cargo test --workspace --locked --doc` -- all `ok`
(checked every crate's line, not just the tail of the run).

`shadow-dom/declarative` (56 files, pinned WPT `c2a2fce6`, base `1da899a5`
vs. this candidate, freshly rebuilt release binary both sides):

| | before | after |
|---|---|---|
| pass (excl. `gethtml.html`) | 727 | **764** |
| fail (excl. `gethtml.html`) | 120 | **84** |
| timeout / error (whole-file), either side | 0 | 0 |

**+37 subtests gained, 0 lost**, restricted to this directory's own files
(no other file in the selection changed at all). Per-file:

- `declarative-shadow-dom-basic.html`: 17/5 -> **22/0 (now fully passing)**
- `declarative-shadow-dom-slot-assignment-serialization.html`: 0/3 ->
  **3/0 (now fully passing)**
- `declarative-shadow-dom-slot-assignment.html`: 0/8 -> 5/3
- `declarative-shadow-dom-attachment.html`: 586/68 -> 610/44
- `shadowrootadoptedstylesheets-same-document.html`: `error` (`TypeError:
  value is not callable`, 0 subtests ran) -> `fail` (1 subtest ran, still
  0 pass) -- no longer crashes the whole file outright, not counted as a
  regression (0 pass either way).

`gethtml.html` (900/2279 -> 1835/1698, `timeout` both times) is excluded
from the totals above for the same reason the prior `setHTMLUnsafe` ticket
excluded it: an auto-generated thousands-of-subtests page that hits a hard
wall-clock worker deadline, whose retained-pass-count is a function of how
many subtests happened to complete before the timer fired, not of
correctness -- not usable as a before/after signal under this harness's
timeout model. Every remaining outcome (44 files) is byte-for-byte
unchanged.

Evidence: `run-corpus.py`, `before.json`/`after.json` in this directory.
