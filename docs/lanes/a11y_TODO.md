# a11y lane — decision map and TODO

Lane owns `crates/amazing-a11y/` and this file. Gate: crate tests + a final
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
- [ ] **A11Y-2 — relationships and collections.** `aria-owns` reparenting,
  `aria-activedescendant`/focus path, `aria-posinset`/`setsize`,
  `aria-rowindex`/`colindex`/`colspan`/`rowspan`, `aria-level` on tree items,
  `aria-relevant` set semantics; expose a resolved `activeDescendant` and
  focus chain on the snapshot.
- [ ] **A11Y-3 — text content and ranges.** per-node accumulated text with
  character offsets (AT reading order), `AxNode::text()`; static-text leaf
  segmentation consistent with inline layout.
- [ ] **A11Y-4 — live regions and events.** `aria-live`/`aria-atomic`/
  `aria-relevant` change classification, `aria-busy`; emit a small
  `AxEvent` stream (`node added/removed/property-changed`) from a tree diff.
- [ ] **A11Y-5 — driver adoption** (with `integrator-assist`), then a CDP
  `Accessibility.getFullAXTree` backing for `devtools`.
