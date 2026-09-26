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
- [x] **A11Y-7 — accname/role WPT conformance.** `scripts/ensure-wpt.sh` (added
  to `dev` after A11Y-5) provisions the pinned WPT checkout, so
  `crates/amazing-a11y/tests/wpt_conformance.rs` (new; `#[ignore]`d behind
  `AMAZING_WPT_ROOT`) reimplements `aria-utils.js`'s three declarative
  verifiers (`verifyLabelsBySelector`, `verifyRolesBySelector`,
  `verifyGenericRolesBySelector`) directly against `build_document`, since the
  real WPT files drive through `test_driver.get_computed_label`/`_role`
  (WebDriver + a platform AT this engine doesn't have) — see the module doc
  comment. Selection: `accname/name/*.html` (15 files) + `wai-aria/role/*.html`
  (23 files). Before: 179 subtest failures. Engine fixes, biggest clusters
  first:
  - `role.rs`: case-insensitive `role=""` token matching; `form`/`region`
    require an author name to be valid in the fallback chain (else skip to the
    next token); `directory` (deprecated) computes as `list`; `img`/`image`
    synonyms both compute `"image"`; new tentative `sectionheader`/
    `sectionfooter` roles.
  - `tree.rs`: `ignorable()` no longer flattens a node with an explicit (even
    invalid/unrecognized) `role` attribute — conflict resolution reassigns the
    role, it never removes the node.
  - `accname.rs` (the bulk of the work): `flatten()` was collapsing NBSP as
    whitespace (`char::is_whitespace` includes it; accname's own whitespace
    set does not) and text/generated-content fragments were flattened
    per-fragment then rejoined with a forced separator, which invented spaces
    with no source whitespace and, backwards, lost genuine boundary
    whitespace (a `content: " before "` pad, a text node's own trailing
    space). Fixed by pushing fragments raw and concatenating directly, with
    exactly one `flatten` at the end. Also added: `text-transform` applied to
    text nodes; `::marker` content; a `display:block`/`inline-block`/…
    word-boundary rule; embedded-control value substitution for a `<label>`'s
    own traversal (scoped away from the specific control whose name is being
    computed, so a text field doesn't fold its own value into its own name);
    `visibility:hidden` tunnels through to a `visibility:visible` descendant
    (mirrors `tree.rs`'s `Decision::Promote`) without leaking the excluded
    node's own direct text; the `aria-labelledby`/`describedby` note that a
    hidden *referenced* target exposes its *entire* subtree unconditionally;
    a self-referencing `aria-labelledby` id (`aria-labelledby="self other"`)
    resolves via steps 2D-2G instead of being dropped or infinite-looping.
    Also fixed a latent pre-existing bug in `own_name` (pre-pushing a node
    into `visited` before calling `compute_name` on it, which always tripped
    the immediate "already visited" guard and returned empty — dormant until
    A11Y-7's fixture coverage reached a nested element with its own
    `aria-label`).
  - Two residuals, documented as exceptions in the test file rather than
    fixed: `comp_name_from_content_alt_counter_invalidation.html` needs a
    running JS engine (a CSSOM stylesheet mutation after load) this crate has
    no way to execute; one `comp_name_from_content.html` subtest needs a
    second, non-cloned "referenced anywhere in this computation" node set
    (today's `visited` is a per-branch ancestor-chain guard, cloned at each
    recursive step, which cannot see a sibling branch's reference) — a real
    architectural gap, not a one-line fix, for one WPT subtest.
  After: 690 passed, 0 failed, 8 logged exceptions (4 spec-acknowledged
  ambiguous `.tentative.html` cases + the 2 residuals above, each hit twice
  in the log), 4 unsupported files (3 pre-existing WebDriver-driven role
  files + the 1 script-dependent accname file). Crate: 95 passed, 1 skipped
  (release-profile-gated), 1 doctest. Workspace gate (run twice, stable):
  2914 passed, 0 failed, 11 skipped; all doctests green. Commit TBD.

## WPT corpus status

`scripts/ensure-wpt.sh` (landed on `dev` after A11Y-5) provisions the pinned
WPT checkout at `~/corpora/wpt` on a Linux sandbox; A11Y-7 used it for the
`accname/name/` and `wai-aria/role/` selections above. No `html-aam/`
selection has been added yet.
