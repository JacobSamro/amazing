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
- **A11Y-14: name-from-content walks the flat tree, not light-DOM children.**
  `accname::flat_children` dispatches on `Dom::shadow_root_of_host`/
  `is_html_slot` so a shadow host's rendered content (its shadow root's
  children, or a `<slot>`'s assigned-or-fallback nodes via
  `assigned_nodes(.., flatten: true, ..)`) is what gets visited. A `<slot>`
  is always transparent to that content — its own `aria-label`/
  `aria-labelledby` never substitutes the way `own_name` would for an
  ordinary element (`accname/name/shadowdom/slot.html`'s cases 3/4 have the
  same expected label with or without `aria-label` on the slot). Full
  AX-tree building (`tree::Builder`) is not yet shadow-tree-aware — only
  name-from-content is — since no target file needed a shadow host's
  content to have its own AX *nodes*, only to feed another element's
  *name*; broadening `tree::Builder` the same way is future work if a WPT
  selection needs it.
- **A11Y-14: ARIA IDL reflection's *name mapping* is mechanical, not a
  lookup table.** `aria::aria_mixin_attribute` derives an `ARIAMixin` IDL
  name's content attribute by rule (hyphenate before each interior capital,
  lowercase) rather than a hand-maintained list, so it can't drift from the
  spec's still-growing member set. The actual WebIDL wiring
  (`Operation`/`InterfaceDefinition::members`) lives in `amazing-bindings`
  (dom-bindings-owned) — filed in `.agents/a11y/questions.md`, not
  implemented here.
- **A11Y-15: native state readers use `amazing-dom`'s live accessors, not
  content attributes, for anything with a spec "dirty" IDL state.**
  `checked`/`selected`/an input's `value` each have a live/current state
  distinct from their parse-time content attribute (a real click/selection/
  edit never touches the attribute) — `host::native_checked`/
  `native_selected`/`accname::control_value`/`selected_native_option_text`
  read `Dom::input_checked`/`option_selected`/`input_value`/`textarea_value`
  accordingly. `disabled`/`required`/`readonly`/`open` (details) have no such
  divergence — they're plain reflected attributes with no separate live
  state — so those stayed attribute reads after an explicit audit (see
  `host.rs`'s own doc comments on each). `<input type=range>`'s ARIA
  `valuenow` fallback needs the live `Dom::input_value_as_number` for the
  same reason; `<progress>`/`<meter>` don't (their `value`/`max` reflect
  straight back to the attribute per spec), so `host::native_range_value`
  treats the three differently on purpose, not inconsistently.

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
  2914 passed, 0 failed, 11 skipped; all doctests green. Commit `b7f42f07`,
  merged into `dev` as `4caf9b2b`.
- [x] **A11Y-8 — live-tree updates.** Incremental AX tree diffs on DOM
  mutation, shaped for devtools' CDP-15 `Accessibility.nodesUpdated` seam
  (`amazing-devtools::host::drain_ax_nodes_updated`) and a later AT-SPI
  bridge on Linux. `crates/amazing-a11y/src/event.rs`'s A11Y-4 `diff()`
  already rebuilt-then-diffed two whole trees by `dom_node` identity, but
  folded every difference on a persisting node into one opaque `Changed`
  event with a `text_changed` bool — not enough for a consumer that needs to
  know *which* aspect changed (CDP's own AXNode fields, or an AT-SPI signal,
  which is one D-Bus call per aspect: name, role, state, active-descendant,
  …). Replaced `AxEventKind::Changed` with one kind per aspect —
  `NameChanged`, `DescriptionChanged`, `RoleChanged`, `StateChanged`,
  `PropertyChanged`, `ActiveDescendantChanged` — and a persisting node now
  emits one `AxEvent` per aspect that actually differs (zero, one, or
  several), each carrying `previous_name`/`previous_role` where relevant.
  Found and fixed one bug this surfaced: `AxProperties::raw` mirrors every
  non-empty `aria-*` attribute verbatim regardless of whether it also has a
  typed field, so comparing the whole `properties` struct made almost any
  `aria-label`/`aria-pressed`/… edit spuriously co-fire `PropertyChanged`
  alongside the real `NameChanged`/`StateChanged` — `properties_changed()`
  now compares the typed fields with `raw` excluded (documented gap: an
  `aria-*` attribute with no dedicated typed field changing produces no
  event of its own; the well-known ones already have a typed field).
  Added `event::rebuild_and_diff(dom, styled, layout, previous: &AxTree) ->
  (AxTree, Vec<AxEvent>)`, the incremental-on-mutation entry point: rebuilds
  the whole tree (Stage 1 has no incremental style/layout invalidation yet)
  and diffs it against the previous checkpoint, which is also what makes
  "name invalidation" correct for free — if node A's `aria-labelledby`
  target's text changes, A's own name is recomputed fresh on the rebuild and
  reported as A's own `NameChanged` even though A's subtree never changed
  (`name_invalidation_propagates_through_a_labelledby_reference` in
  `event_tests.rs`). `crates/amazing/src/runtime_host_a11y.rs` gained
  `diff_ax_tree`, a thin wrapper mirroring `build_ax_tree`, for a future
  driver caller. Filed the CDP-15 hookup itself (deduplicating per-node,
  dropping `Removed` since CDP's push event has no removal concept) in
  `.agents/devtools/questions.md` — `runtime_host_devtools.rs` is the
  devtools lane's file. AT-SPI investigation deferred to whichever ticket
  first needs a real Linux AT-SPI bridge; the granular `AxEventKind` split is
  already shaped for its per-aspect signal model.
  Crate: 102 passed, 0 failed, 1 skipped (release-profile-gated), 1 doctest.
  `amazing` crate: `runtime_host_a11y` tests pass (2/2, including the new
  `diff_ax_tree` one). Workspace gate (run twice, stable): 3051 passed, 0
  failed, 13 skipped; all doctests green both runs. Commit `e016f2f7`, merged
  into `dev`.
- [x] **A11Y-9 — the Linux platform accessibility bridge (design note).**
  Read before touching code, per the ticket's own instruction. Evidence
  gathered by reading GPUI's vendored source (`crates/amazing-shell-gpui`
  pins `gpui`/`gpui_platform` to a fixed `zed-industries/zed` git rev; fetched
  `crates/gpui/src/_accessibility.rs`, `crates/gpui/src/window/a11y.rs` and
  `crates/gpui/src/elements/div.rs` at that exact rev via `gh api`) rather
  than guessed from general AccessKit/GPUI familiarity.

  **Does GPUI already speak AT-SPI?** Yes, extensively.
  `crates/amazing-shell-gpui/Cargo.lock` (the isolated GPUI dependency graph)
  pulls in `accesskit`, `accesskit_consumer`, `accesskit_unix` and
  `accesskit_atspi_common` — `accesskit_unix` *is* the AT-SPI2-over-D-Bus
  adapter, owning the whole `org.a11y.Bus` registration. GPUI's own
  `window/a11y.rs` builds a fresh `accesskit::TreeUpdate` every frame from its
  own element tree and hands it to this adapter; the adapter diffs it against
  the previous update and calls the platform APIs. So: no D-Bus code needed
  here at all — GPUI/AccessKit already own that layer end to end, on all
  three desktop platforms via the same source, not just Linux.

  **Feed through AccessKit's tree model, and specifically: which of GPUI's
  two mechanisms?** GPUI exposes two ways to contribute AccessKit nodes:
  1. An ordinary `div().id(..).role(..)`, walked every frame by GPUI itself;
     real parent/child edges come from real element nesting, so this
     supports an arbitrarily deep tree for free.
  2. `Element::a11y_synthetic_children`/`A11ySubtreeBuilder` — one element
     pushes *extra* flat AccessKit nodes as children of *itself*. Read the
     implementation (`A11yNodeBuilder::push_leaf` in `window/a11y.rs`) and
     the one real product usage (`settings_ui`'s `input_field.rs`, text-input
     text runs): every `push_child` call registers as a child of whichever
     *real* element is running the callback, with no way to nest a pushed
     node under another pushed node. Using this for our whole AX tree would
     give every non-immediate node **two parents** (the real element, and its
     true logical parent) — `accesskit_consumer` expects a proper tree, and
     the one real usage in GPUI's own codebase is deliberately flat (text
     runs), confirming this reading rather than a nested one.

  **Decision: the div-tree route** — `crates/amazing-shell-gpui/src/
  a11y_bridge.rs` builds one real, zero-size, off-screen `div` per exposed
  `amazing_a11y::AxNode`, nested exactly like the `AxTree` itself, using
  `div()`'s public ARIA-mirroring builder methods (`.role`, `.aria_label`,
  `.aria_description`, `.aria_selected`, `.aria_expanded`, `.aria_toggled`,
  `.aria_numeric_value`(+`_step`/`_min`/`_max`), `.aria_value`,
  `.aria_placeholder`, `.aria_level`, `.aria_position_in_set`/
  `.aria_size_of_set`, `.aria_row_index`/`_count`/`.aria_column_index`/
  `_count`, `.aria_orientation`, `.aria_keyshortcuts`,
  `.aria_active_descendant`). These read as a close mirror of ARIA itself,
  which is unsurprising: GPUI's own accessibility guide cites WAI-ARIA and
  the APG directly.

  **Documented, real gap (not worked around):** at this pinned rev, `div()`
  has no `.aria_live()`/`.aria_busy()`/`.aria_invalid()`/`.aria_modal()`/
  `.aria_multiselectable()` — `Interactivity`'s `AriaAttributes` struct
  (backing every `.aria_*` method) simply has no such fields yet. Live
  regions, `aria-busy`, `aria-modal`, `aria-invalid` and
  `aria-multiselectable` cannot be expressed through GPUI's current public
  API. This is an upstream gap, not something to route around with a second,
  parallel AccessKit adapter; it is recorded here for whoever next
  bumps the pinned GPUI rev to check whether it has since been added upstream.

  **Identity and "your A11Y-8 change events":** each shadow `div`'s id is
  derived from the `AxNode`'s `dom_node` (falling back to its `AxNodeId` for
  the synthetic root) — the same key A11Y-8's `event::diff` compares by.
  Keeping this stable across rebuilds means AccessKit's *own* per-frame
  tree-diffing (documented in `window/a11y.rs`: "we build a `TreeUpdate` ...
  diffs it against the previous update ... nodes may have been created,
  destroyed, or updated") produces the AT-visible equivalent of A11Y-8's
  `Added`/`Removed`/`NameChanged`/… events automatically, every time the
  caller rebuilds this element from a freshly built `AxTree` (e.g. via
  `runtime_host_a11y::diff_ax_tree` after a mutation checkpoint). A11Y-8's
  `Vec<AxEvent>` is not walked a second time inside the bridge: GPUI/
  AccessKit's own frame-to-frame diff is the delivery mechanism here, not an
  extra one layered on top — the two ends of the A11Y-8 work (a rich,
  per-aspect diff *and* stable identity keyed by `dom_node`) turned out to be
  exactly what this bridge needed, for a different reason than the CDP-15
  seam that motivated the per-aspect split in the first place.

  **Focus:** GPUI reports one real AccessKit-focused node per window, tied to
  a `FocusHandle` via `.track_focus()`. The shell needs exactly one such
  handle for the whole page viewport regardless of accessibility (ordinary
  keyboard routing). Every shadow node whose `AxNode::states.focused` is
  true, or that some ancestor's `AxNode::active_descendant` names (ARIA's
  composite-widget roving-focus pattern), gets `.aria_active_descendant()`:
  GPUI honors this only while a real focused ancestor is present, so it is
  safe to set unconditionally on the target and never touches real keyboard
  focus itself.

  **Scope of this ticket's footprint:** `a11y_bridge.rs` (new file, this
  lane's) plus a new, self-contained proof binary
  (`src/bin/a11y_proof.rs`, new `[[bin]]` entry) that loads a real-pages
  fixture through the same parse/style/layout/`amazing_a11y::build` pipeline
  `amazing a11y-tree` uses and mounts the bridge's tree in a bare GPUI
  window — this is what the AT-SPI sandbox evidence below is captured
  against. Wiring the bridge into the live, interactive browser tab
  (`crates/amazing-shell-gpui/src/main.rs`'s `BrowserShell`/`Tab`, which is
  ~3000 lines of chrome this lane does not own the architecture of — which
  struct holds the DOM alongside styles/layout, when a tab's tree should
  rebuild, which `FocusHandle` already exists for the viewport) is filed as
  a request in `.agents/shell/questions.md` rather than guessed at and
  edited directly, per the escalation rule for paths this lane does not own.
  `Cargo.toml` gained one dependency line (`amazing-a11y`) and one `[[bin]]`
  block (mirroring the existing `amazing-compositor-proof` entry) — additive,
  not a change to any existing target.

  **Implementation and tests.** `a11y_bridge.rs` adds `AxNodeAttrs` (a
  GPUI-independent translation of one `AxNode` plus its ARIA-relevant
  properties/states), `map_role` (exhaustive `amazing_a11y::Role` ->
  `accesskit::Role`, with `GenericContainer` as the fallback for abstract/
  unreachable roles and for anything the pinned accesskit lacks — e.g.
  `Subscript`/`Superscript` fold into `Label` since that accesskit rev has no
  such variants), `node_attrs`/`node_attrs_with` (walks the tree once,
  precomputing the focus/`aria-activedescendant` target set), and
  `build_element`/`ax_tree_element` (constructs the real GPUI `div` tree,
  special-casing `GenericContainer` as a bare, id-less, role-less wrapper div
  so GPUI's own `debug_assert!("GenericContainer is filtered out of the a11y
  tree and has no effect")` is never tripped). 7 unit tests, covering: every
  `Role` variant maps to something other than the default placeholder;
  heading name/level; checkbox state -> `Toggled`; slider value/range; a
  focused node (via `amazing_dom::Dom::set_active_element`, since static HTML
  `autofocus` does not auto-apply in this pipeline) getting
  `report_as_active_descendant`; a node's `stable_id` (keyed by `dom_node`)
  surviving a name change at the same DOM node, confirmed on the
  `Role::Label` text-content child since `Role::Status` does not take
  name-from-content per ARIA; and `ax_tree_element` not panicking end to end
  on a real-pages fixture. All 7 pass.

  **Sandbox proof (pool-desktop Boat sandbox, Ubuntu 24.04, Xvfb :97).**
  Built and ran `amazing-a11y-proof` against
  `tests/real-pages/pages/app-analytics.html` (86-node `AxTree`) under Xvfb,
  then used `python3-pyatspi` as an independent AT-SPI2 client against the
  live `org.a11y.Bus`:
  1. Coarse proof: `pyatspi.Registry.getDesktop(0)` lists `amazing-a11y-proof`
     as a real desktop AT-SPI application, alongside the sandbox's own
     `ibus-*` services — the process self-registered on the accessibility
     bus purely by opening a GPUI window with the bridge's tree mounted; no
     bridge-side D-Bus code exists or was needed.
  2. Fine-grained proof: a recursive `getChildAtIndex()` walk from that
     application node reproduces the fixture's real structure through the
     whole chain (`AxTree` -> `a11y_bridge` -> GPUI div tree -> AccessKit ->
     `accesskit_unix` -> AT-SPI2 D-Bus -> `pyatspi`): the nav landmark with
     its Home/Section/Search links and Menu button, the "Analytics" heading,
     and the data table's header row (Name/Value/Status) plus all 8 data
     rows/cells with matching text. The two nested `AxNode`s that map to
     `GenericContainer` came through as transparent `'document frame'`
     wrappers, not spurious or panicking nodes. Full dump saved as evidence:
     `docs/lanes/a11y-9-atspi-dump.txt`.
  3. A `dbind-WARNING ... Unknown object '/org/a11y/atspi/cache'` appeared in
     the proof binary's stderr during the walk. Investigated: it did not
     block or truncate the tree walk (every node above came back with
     correct role/name/states on direct child traversal), and is a known
     at-spi2-core cache-registration race for a just-spawned application —
     cosmetic here, not a defect in the bridge. Recorded in the evidence file
     rather than silently dropped.

  **Sandbox package gaps found and fixed** (filed in `.agents/shell/
  questions.md` for the template, per the ticket): `libxkbcommon-x11-dev`
  was missing and blocked linking of *every* GPUI binary on this sandbox
  (not a11y-specific — the pre-existing `amazing-compositor-proof`/
  `amazing-shell` targets hit the same linker error); `xvfb` and
  `python3-pyatspi` were entirely missing. `at-spi2-core`, `at-spi2-common`,
  `libatk-bridge2.0-0t64` and `libatk-bridge2.0-dev` were already present, as
  was a running `at-spi-bus-launcher` user service and
  `toolkit-accessibility=true`.

  **Gate.** `crates/amazing-shell-gpui` is its own isolated Cargo workspace
  (`[workspace] members = []`), so the main `cargo nextest run --workspace`
  gate does not build or test it; gated it separately with its own
  `cargo test`, run twice for stability: 8 passed (7 new `a11y_bridge` tests
  + 1 pre-existing `compositor_presenter` test), 0 relevant failures, both
  runs identical. One pre-existing, unrelated failure was observed on the
  same runs, `compositor_presenter::tests::
  single_tile_opacity_and_rect_clip_stay_on_gpui_tile_path`
  (`left: 0.5019608, right: 0.5`, an 8-bit-quantization rounding mismatch) —
  it touches zero files this ticket changed and is the shell lane's own
  compositor-proof suite, so it is noted here as out-of-scope rather than
  chased. Main workspace gate (unaffected by this ticket — no main-workspace
  crate was touched) re-run for hygiene: unchanged from A11Y-8's numbers.

## WPT corpus status

`scripts/ensure-wpt.sh` (landed on `dev` after A11Y-5) provisions the pinned
WPT checkout at `~/corpora/wpt` on a Linux sandbox; A11Y-7 used it for the
`accname/name/` and `wai-aria/role/` selections above. No `html-aam/`
selection has been added yet.
