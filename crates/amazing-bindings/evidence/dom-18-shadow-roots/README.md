# DOM-18 follow-up: `sequence_shadow_roots`, plus the `runtime_binding_host.rs` file-move

Implementation on fixed base `5cd043b2` (local `dev`, DOM-B-NEXT + HTML-35
merged).

## 1. File-size follow-up (coordinator request)

`crates/amazing/src/runtime_binding_host.rs` had grown to 2058 lines. Per the
per-lane adapter convention in `AGENTS.md`, moved
`refresh_window_named_properties` (HTML-35's window-named-access mechanism)
out to this lane's own adapter file, `crates/amazing/src/runtime_host_dom_bindings.rs`,
renamed `dom_refresh_window_named_properties`, `pub(super)`. The shared file
keeps only the one-line call site (`self.dom_refresh_window_named_properties()?`
in `with_dom`) and the doc comment on `window_named_property_keys` now points
at the new location. No behavior change -- logic is untouched, only moved.
Line counts after the move: `runtime_binding_host.rs` 2013 lines,
`runtime_host_dom_bindings.rs` 1164 lines (both well under the 4000-line cap).

## 2. `BindingHost::sequence_shadow_roots` (DOM-18 remaining gap)

`Element.getHTML({shadowRoots: [...]})` (DOM-18 declarative shadow DOM
serialization) needs a WebIDL `sequence<ShadowRoot>` conversion for its
`shadowRoots` option. `crates/amazing-bindings/src/host.rs`'s
`BindingHost::sequence_shadow_roots` default returned
`DomError::NotSupportedError` unconditionally -- the one genuine remaining
gap after HTML-17 (parser integration/attachment, already landed via
`amazing-html/src/declarative_shadow_tests.rs`) and DOM-B-REFLECT's aria
element-reflection routing (a different, still-open a11y item, see
`questions.md`).

Implemented `dom_sequence_shadow_roots` in this lane's adapter file
(`runtime_host_dom_bindings.rs`), mirroring the existing array-like walk in
`sequence_values`/`sequence_dom_strings` (`runtime_binding_host.rs`): reads
`.length`, walks indexed properties, resolves each item to a `NodeIdx` via
`platform_object(&item, InterfaceBrand::NODE)` +
`node_from_platform_key`. Deliberately does not re-check
`Dom::is_shadow_root` per element here -- `markup_bindings::invoke`'s
`O::ElementGetHTML` arm already re-validates every returned `NodeIdx` that
way and throws `TypeError` for the whole list if any item isn't actually a
shadow root, so duplicating the check here would be redundant.

New trait method wired in `runtime_binding_host.rs`:
`fn sequence_shadow_roots(&mut self, value: &JsValue) -> Result<Vec<NodeIdx>, Throw> { self.dom_sequence_shadow_roots(value) }`

### Measured: no observable corpus movement, and why

Ran `crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py` with
`--directory shadow-dom/declarative` (the whole directory, including
`tentative/shadowrootadoptedstylesheets/`) at fixed base `5cd043b2`
(`before.json`, via `--ref`) and this candidate (`after.json`, working
tree). **Byte-for-byte identical status and counts on all 56 files** --
zero diffs. `Element-getHTML`'s `shadowRoots` option is only exercised in
this pinned corpus by `shadow-dom/declarative/gethtml.html`, which errors
in *both* runs before reaching any assertion:
`resource ../../html/resources/common.js: No such file or directory` -- a
missing-resource gap in this script's sparse WPT checkout (its
`sparse_checkout` call lists `['resources', 'common', *DIRECTORIES]`, not
`html/resources`), unrelated to this change. `gethtml-ordering.html`
(the other `getHTML`-related file, already passing 3/3 in both before and
after) does not exercise the `shadowRoots` option at all.

So this fix removes an incorrect `NotSupportedError` throw for any real
`getHTML({shadowRoots})` call (previously any such call would throw
regardless of arguments) and is spec-correct and consistent with the
existing sequence-conversion pattern, but has no measurable effect in this
particular pinned WPT snapshot until the sibling `html/resources` gap is
fixed -- see `questions.md`'s 2026-09-27T21:30:00Z entry for that plus a
found-in-passing gap (`HTMLTemplateElement.shadowRootMode`/
`shadowRootDelegatesFocus`/`shadowRootClonable` reflection is entirely
unimplemented, driving several `declarative-shadow-dom-basic.html`
failures) routed there rather than implemented this ticket, given no time
budget remained after triage.

## Gate

Workspace (`venus-001`, base `5cd043b2` + this ticket's uncommitted diff at
gate time): `cargo nextest run --workspace --locked --no-fail-fast` --
**3482 passed / 17 skipped / 0 failed**. `cargo test --workspace --locked
--doc` -- all `ok`, including `amazing_js` 1/1 and `amazing_html` 1/1.

Evidence: `before.json`/`after.json` in this directory (raw
`run-corpus.py` output for `shadow-dom/declarative`).
