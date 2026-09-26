# a11y lane — decision map and TODO

Lane owns `crates/amazing-a11y/`, its own driver-adapter file
`crates/amazing/src/runtime_host_a11y.rs` (granted for A11Y-6, per the
driver-adapter-per-lane rule — the shared `crates/amazing/src/main.rs` and
`Cargo.toml` only ever get a one-line `mod`/dependency edit and a one-line
dispatch/call from this lane), and this file. Gate: crate tests + a final
Linux workspace gate. Corpus for this lane is its own table-driven fixtures
(accname-1.2 and HTML-AAM selections), since no WPT `accname/` checkout is
available on the build hosts; add one when the WPT corpus lands.

## Decisions

- **One crate, three layers.** `role` (taxonomy + HTML-AAM), `accname`
  (name/description), `aria` (states + properties) are independent pure
  functions over `&Dom`; `tree` composes them with computed style and layout;
  `bounds` is layout-only. This keeps the seam testable without a driver.
- **Explicit roles are filtered, not trusted.** Only a non-abstract ARIA token
  is accepted; `none`/`presentation` are resolved through the presentational
  conflict rule (focusable or global-ARIA element keeps its implicit role).
- **Engine-only roles use lowercase tokens** (`statictext`, `inlinetextbox`,
  `linebreak`, `listmarker`, `iframe`, `mark`, `ruby`) so the snapshot is a
  uniform string vocabulary.
- **Pruning is a tree property, not a name property.** display:none / `hidden` /
  `aria-hidden` drop the subtree; `visibility:hidden` promotes the element and
  drops its text but lets a `visibility:visible` descendant re-expose itself.
  Generic containers with no name that only wrap text are flattened to a
  fixpoint; focusable or ARIA-bearing generics are kept.
- **Snapshot is detached plain data** so a driver or a test can hold it without
  borrowing the DOM; `dom_node` is the arena index back to the live document.
- **No unsafe, no external crates** (`#![forbid(unsafe_code)]`).

## TODO

- [x] **A11Y-1** (bulk) — role mapping, accname-1.2, states/properties, pruning,
  bounds, snapshot API, table-driven fixtures. Commit `40148f23`.
- [x] **A11Y-2 — relationships and collections.** `aria-owns` reparenting,
  `aria-activedescendant`/focus path, `aria-posinset`/`setsize`,
  `aria-rowindex`/`colindex`/`colspan`/`rowspan`, `aria-relevant` flags.
  Commit `1f0eb1f2`.
- [x] **A11Y-3 — text content and ranges.** reading-order text and character
  offsets. Commit `e7bccba5`.
- [x] **A11Y-4 — live regions and events.** live-region resolution, `aria-relevant`
  gating and an `AxEvent` diff stream. Commit `21316489`.
- [x] **A11Y-5 — resolved relationships.** `AxRelations` resolves
  `aria-labelledby`/`describedby`/`details`/`errormessage` (single)/`controls`/
  `flowto`/`owns` id references to tree node ids, via `AxNode::relations` and
  `AxTree::relations(id)`. Resolution runs after `Builder::finish`, against the
  final pruned tree, so a reference to a node presentational flattening
  removed drops out instead of pointing at a stale pre-prune id;
  `aria-activedescendant` was moved to the same pass (it had the identical
  latent staleness since A11Y-2, just never hit by that ticket's fixtures).
  Commit `9adfb13d`.
- [x] **A11Y-6 — driver adoption.** `crates/amazing/src/runtime_host_a11y.rs`:
  `build_ax_tree(dom, styled, layout)` wraps `amazing_a11y::build` as the seam
  a future CDP `Accessibility.getFullAXTree` handler in `devtools` calls once
  the driver holds a live document's DOM/style/layout. `ax_tree_for_page`
  drives the same parse/style/replaced-content/font/layout pipeline `amazing
  render`/`amazing layout` use, backing the new `amazing a11y-tree
  <page.html> [width]` CLI subcommand and a test against
  `tests/real-pages/pages/app-analytics.html`. Commit `20b7c5f8`.

## WPT corpus status

No `accname/` or `html-aam/` WPT checkout exists on the Linux sandbox as of
A11Y-5 (checked `find / -iname '*wpt*'`; only `wpt-html`, `wpt-render`,
`wpt-replaced-content`, `wpt-integrator-assist` are present). This lane
continues to gate on its own table-driven fixtures until one is added.
