# CSS lane decision map

## CSS-1 — typed computed values (prototype)

Typed values precede `@layer` (integrator decision, 2026-09-20). Locked decision
4a remains unchanged: `compute(dom, sheets) -> StyledDom` is the only style
computation seam. Custom-property dependency graphs, token substitution, and
CSS-wide defaulting remain private. No layout or paint source is changed.

### Approved interface and scope

`ComputedStyle::typed(name) -> Option<&ComputedValue>` adds a CSS-owned typed
view. Existing `get`, `iter`, and `set` signatures remain compatible. The
computed style stores a typed map alongside legacy serialization; existing
consumers can migrate separately without learning custom-property internals.
Unimplemented compound grammars are explicitly `ComputedValue::Unparsed`.

- Length/percentage values distinguish CSS pixels, percentages, and relative
  bases. All seven absolute units resolve at 96 CSS px/in, including Q.
  Computation resolves em/rem from computed element/root font sizes. ex/ch and
  viewport units retain explicit bases; the CSS seam does not invent metrics.
- Numbers and integers cover opacity, flex factors, order, z-index, orphans,
  widows, and unitless line-height. Property ranges are checked. Percent
  line-height computes against the element font; inherited computed lengths
  are copied rather than recalculated against a descendant's font.
- Angles retain radians without modulo reduction. The supported 2D transform
  list stores typed translate/scale/rotate/matrix operations.
- RGBA8 and currentColor reuse the existing paint color parser's supported
  grammar and precision. `color: currentcolor` inherits the parent's color;
  other color properties retain the currentColor keyword.
- Typed enums cover display, position, overflow, direction, box sizing,
  flex direction/wrap, border style, white space, visibility, text alignment,
  writing mode, float/clear, ruby roles, and the supported alignment keywords.
- Invalid ordinary longhands are rejected before ranking; invalid values
  after var substitution default as unset without reviving an earlier winner.
  Token boundaries are preserved: `var(--n)px` cannot create a dimension.
- Constructors and mutation update the typed map; mutations of one property
  preserve already computed inherited values of other properties.

This is a staged representation, not full CSS Values support: calc/min/max,
full color spaces/named-color coverage, all compound grammars, and layer/origin
rollback remain outside this ticket. Legacy consumers still parse serialized
values until their own migration. Typed ex/ch and viewport units require
used-value context. RGBA8 retains the existing parser's quantization and
permissive mixed legacy RGB channels; no new color-conformance claim is made.
Valid color keywords outside the RGBA8 table and unsupported color functions retain their serialized winning declaration and an
Unparsed marker. Treating those as invalid incorrectly revives earlier fallback
colors; the pinned corpus exposed this during development (30/29/13), and public
compute regression tests now cover the distinction. The same compatibility
rule preserves layout's existing calc(infinity), calc(-infinity), and calc(nan)
flex-factor forms as Unparsed until a typed math grammar lands.

References: [CSS Values 4](https://www.w3.org/TR/css-values-4/),
[CSS Variables 1](https://www.w3.org/TR/css-variables-1/).

### Evidence

Baseline source: lane/css c50ee3f before CSS-1. Lane rebased to origin/dev e9c1e21.
Windows-only execution at `C:\Users\server\src\wt\css`, target directory
`C:\Users\server\target\css`, two build jobs. No Mac build or test.
WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
Selection: unchanged `tests/wpt-render/cascade-manifest.json`, 72 cases,
800x600 viewport; XHTML/XML tests and references remain unsupported.
An isolated WPT checkout avoids other lanes changing the sparse selection.
The earlier all-unsupported attempt against the shared sparse checkout is
invalid evidence and excluded.

- CSS unit tests before: 62 passed, 0 failed.
- CSS unit tests after: 87 passed, 0 failed.
- Color parity integration target: 17 passed, 0 failed (16 original paint
  parser tests plus parity over edge cases and deterministic generated input).
- Public compute regression target: 5 tests cover unsupported colors, invalid
  substituted colors, pre-existing flex calc constants, and all ruby display roles.
- Final locked workspace: **772 passed / 0 failed / 3 ignored**, across 48
  test/doc-test targets. Earlier failed attempts found missing named-color,
  special flex calc, and ruby-display compatibility cases; all were fixed
  within CSS and covered by tests before this successful run.
- Two baseline cascade runs: each 33 pass / 26 mismatch / 0 timeout /
  13 unsupported / 0 crash. Two corrected candidate runs reproduce these counts.
  All four normalized evidence hashes are identical. Variables is 12 pass /
  0 mismatch / 0 timeout / 0 unsupported / 0 crash in each of those runs.


### Final rendering evidence (2026-09-20)

Counts below are **pass / mismatch / timeout / unsupported / crash**. Each
selection uses its unchanged `tests/wpt-render/<selection>-manifest.json` at
the WPT revision above. Every before/after case has identical classification,
differing-pixel count, maximum difference, and actual/reference/diff SHA256.

| Selection | Before | After |
|---|---|---|
| cascade (two runs each) | 33/26/0/13/0 | 33/26/0/13/0 |
| Variables subset (two runs each) | 12/0/0/0/0 | 12/0/0/0/0 |
| block-collapse | 9/0/0/0/0 | 9/0/0/0/0 |
| position | 9/3/0/0/0 | 9/3/0/0/0 |
| text-flow | 14/2/0/0/0 | 14/2/0/0/0 |
| flex-grid | 24/0/0/0/0 | 24/0/0/0/0 |
| paint-compositing | 24/0/0/0/0 | 24/0/0/0/0 |
| text-shaping | 12/0/0/0/0 | 12/0/0/0/0 |
| vertical-ruby | 11/9/0/0/0 | 11/9/0/0/0 |

Normalized SHA256 (both baseline runs and both final runs):
`3cea34605e5ca14ffeb0255179366d0ead786d744df8ba5d461045c4368d534d`.
Normalization sorts tests by id, retains id/classification/relation,
differing_pixels/max_difference and all three image SHA256 fields when present,
then encodes JSON with sorted keys and compact separators before hashing.

Artifacts on win-001:

- Runnable baseline: `C:\Users\server\target\css\css1-before.exe`, SHA256
  `c77c5dd302b8e8d0d1ec1caff58ce76c2b80128d353fb45382cb2bf9a252fad1`.
- Runnable final: `C:\Users\server\target\css\css1-verified.exe`, SHA256
  `2387536cbadddb791a451e571c9fa191fa385f7aeefba907b703615499f5388e`.
- Per-case reports/images: `C:\Users\server\target\css\css1-baseline-{1,2}`,
  `css1-before-<selection>`, and `css1-verified-{1,2,<selection>}`.
- Successful workspace log: `C:\Users\server\target\css\css1-workspace-complete.log`.
- Local orchestration evidence: `.agents/css/evidence.json`, `summary.json`,
  `source-hashes.json`, and `workspace.log` in the integrator's main tree.
  All six changed Rust files match the tested Windows files byte-for-byte.

Reproduce from the lane's Windows checkout, using PowerShell:

```powershell
$env:CARGO_TARGET_DIR = 'C:\Users\server\target\css'
$env:CARGO_BUILD_JOBS = '2'
cargo test -p amazing-css --locked
cargo test --workspace --locked
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath tests\wpt-render\cascade-manifest.json `
  -WptRoot C:\Users\server\target\css\wpt `
  -ExecutablePath C:\Users\server\target\css\css1-verified.exe `
  -OutputDirectory C:\Users\server\target\css\css1-review `
  -SkipFetch -SkipBuild
```

## Cross-lane follow-up — remove transitional paint color parser

Originally labeled CSS-2 in the CSS-1 handoff; this is a replaced-content
follow-up, separate from the CSS-2 cascade ticket.

**Owner:** replaced-content, coordinated with css. **State:** relayed and acknowledged by
replaced-content via the integrator; do not implement as part of CSS-1.

CSS's `amazing_css::color::parse_css_color` is canonical. The existing paint
parser is deprecated-in-place and transitional, not an independent fork.
`crates/amazing-css/tests/color_parity.rs` compiles the actual paint parser
source as a test-only module and compares its behavior with CSS's parser.
This avoids a reversed runtime dependency or a new workspace lockfile entry.

Replace paint's duplicate implementation with a compatibility re-export of
the CSS parser (preserving paint's public API). Move any remaining tests to CSS
and retire the source-linked transitional parity module. Before removal, keep
parity green for malformed syntax, Unicode, numeric extremes, alpha, named
colors, and hex forms. Rerun paint's focused 24-case gate and workspace tests.

## Later tickets

- Layout/paint adopt `ComputedStyle::typed` for their supported properties.
  Keep percentages and missing metrics explicit at the used-value boundary.

## CSS-2 — cascade layers, origin/layer rollback, and scope

Implementation stays inside `amazing-css`; `compute(dom, sheets)` and typed
computed values remain the consumer seam. No caller/layout/paint files changed.

### Decisions and behavior

- Parse layer statements and blocks, dotted/nested names, and anonymous layers.
  Keep empty ordering declarations, including layers without matching rules.
  Names are case-sensitive and shared across author sheets; anonymous segments
  are private to each occurrence and stylesheet. Preserve token adjacency in
  dotted names; `default` is a valid identifier, CSS-wide keywords are not.
- Compile hierarchical first-appearance order across all sheets of an origin.
  A parent's direct declarations occupy its last implicit sublayer; unlayered
  rules occupy the outermost last layer. Important declarations reverse that
  entire order; inline style keeps its separate precedence.
- Compare importance, origin (reversed for important), element-attached style,
  layer (reversed for important), matched-selector specificity, scope proximity,
  then declaration order. Scope-start specificity never enters the rule's key.
- Retain the property candidate lists until rollback finishes. `revert` removes
  the author origin at both importance levels; UA revert defaults as unset.
  `revert-layer` removes the current layer and the important-to-normal interval,
  with the element-attached important exception. Rollback can chain through
  several layers. Logical margins join physical candidates before final ranking.
- Custom properties roll back before their dependency graph is resolved.
  Longhand and supported deferred-shorthand rollback keywords produced by
  variable substitution use the same retained candidates. Invalid substituted
  values still default; they do not revive a lower candidate. `all` also accepts
  variable substitution while retaining its CSS-wide-only grammar and excluding
  direction, unicode-bidi, and custom properties.
- Parse explicit scope roots/limits and nested scopes. Consider every applicable
  ancestor root, exclude limits and descendants, and use innermost-root distance
  only after specificity. `:scope` resolves through compound selector functions
  and combinators; outside an explicit scope it matches the document element.
  Implicit relative scope anchors contribute zero specificity.
- Group nesting and combined layer-name depth are capped at 128. Oversized
  constructs are dropped, not partially flattened into unlayered declarations.

References: [Cascade 5 sorting/layers/rollback](https://www.w3.org/TR/css-cascade-5/),
[Cascade 6 scoped rules and proximity](https://drafts.csswg.org/css-cascade-6/).

### Approved integration boundary and limits

`Stylesheet::scope_root: Option<NodeIdx>` stores the stylesheet owner's parent
for implicit scope; `None` is the ownerless document default. Explicit
`@scope (<selector>)` does not require it. The user approved this seam and owns
integrator work to preserve **separate stylesheets and owner nodes** through the
driver and shell, which currently concatenate CSS. This is also prerequisite
provenance for CSS-4 URL resolution (originally queued as CSS-3).

**Implicit @scope cannot be measured end to end until that driver/shell wiring
lands.** Public-seam tests assigning `scope_root` prove only CSS behavior, not
end-to-end browser support. No caller code is changed by this ticket.

Existing selector limitations remain (pseudo-elements, complex functional
selector arguments, CSS nesting `&`, and shadow-tree behavior). Layer-qualified
imports and media evaluation remain outside the parser's supported rule set.
User-origin sheets, animation origins, and transitions have no input seam;
origin rollback here covers the existing author and UA origins. This ticket
makes no full Cascade 5/6 conformance claim.

### Validation

Clean-artifact Windows workspace: **943 passed / 0 failed / 3 ignored**, across
52 test/doc-test targets. CSS: **88 unit + 27 cascade-context + 17 color-parity
+ 5 typed-compute tests**, all passing. The 27 public-seam tests plus one added
UA-origin unit test are regression tests, not added WPT coverage. Rendering
measurements are recorded below. Evidence uses base 80c6636
(133292f plus the refresh-script commit). The first refresh invalidated earlier
CSS-2 runs. A subsequent full-workspace attempt found stale Cargo library
artifacts even though source hashes matched: NET-3 tests could not find methods
present in `ParsedUrl`. Tar extraction restored older mtimes, so source-hash
verification alone was insufficient. All 18 first-party workspace packages were
cleaned from the isolated CSS target, and both measurements restarted as `r3`.
Only `css2-r3-*` artifacts qualify as final evidence. Build inputs are checked
before and after compilation, and immutable executable hashes before/after
rendering; the final source tree is also checked after all gates.


WPT revision: **c2a2fce69e5c2509d9dc28440ec8b42532e5c293**. Unchanged
`tests/wpt-render/<selection>-manifest.json` selections; lane-local WPT checkout,
800x600 viewport, scale 1, same fonts/resources. Counts are
**pass / mismatch / timeout / unsupported / crash**.

| Selection | Before | After |
|---|---|---|
| cascade (two runs each) | 32/27/0/13/0 | 35/24/0/13/0 |
| Variables subset (two runs each) | 12/0/0/0/0 | 12/0/0/0/0 |
| block-collapse | 9/0/0/0/0 | 9/0/0/0/0 |
| position | 9/3/0/0/0 | 9/3/0/0/0 |
| text-flow | 14/2/0/0/0 | 14/2/0/0/0 |
| flex-grid | 24/0/0/0/0 | 24/0/0/0/0 |
| paint-compositing | 24/0/0/0/0 | 24/0/0/0/0 |
| text-shaping | 12/0/0/0/0 | 12/0/0/0/0 |
| vertical-ruby | 11/9/0/0/0 | 11/9/0/0/0 |

Both baseline normalized hashes:
`f722c4dda0d3324bfa245734c6dc4d7688b2b819a0a46a9f8ca003f099ed8183`.
Both final normalized hashes:
`a259e9d395326e4c39b890dd494b159dcacb28a834ccc497e6ee94a7ec608026`.
Normalization is the same sorted per-case classification/difference/image-SHA256
projection used for CSS-1, excluding timestamps and filesystem paths. Every
one of the seven downstream selections is identical before/after, including
actual/reference/diff image hashes, not just counts.

Exactly three cascade cases change mismatch → pass:
`css-cascade__scope-featureless.html`,
`css-cascade__all-prop-revert-color.html`, and
`css-cascade__scope-pseudo-element.html`. The last remains **partially inert**:
pseudo-element/generated-content selectors are dropped in both test and reference;
this is a measured pixel pass after the root background starts matching, not a
claim of pseudo-element conformance. The three selected `revert-layer` cases
remain unsupported because their references require XHTML/XML parsing. Layer
semantics are covered by focused Rust tests, not added WPT passes.

The refreshed baseline is 32/27/13, not CSS-1's historical 33/26/13:
`css-values__calc-max-height-block-1.html` already mismatches on the refreshed
siblings before CSS-2. That drift is not attributed to this change.

Artifacts on win-001, all under `C:/Users/server/target/css/`:

- Baseline executable `css2-r3-before.exe`, SHA256
  `b16ce505d6334bf8ea33051d1bea99c5843a631764713b3553f06eeba340e230`.
- Final executable `css2-r3-final.exe`, SHA256
  `60442ba140ed95eb21bfdc240387469ed610b3956c87ccffdd4446d8f0648430`.
- `css2-r3-{before,final}-<index>-<selection>/run-*/report.json` and PNGs.
  Indices 1/2 are cascade; 3–9 follow the downstream order in the table.
- `css2-r3-workspace.log`, source/executable build proofs, and per-run logs.

Local orchestration evidence in the main checkout's `.agents/css/`:
`css2-r3-evidence.json`, `css2-r3-summary.json`, `css2-r3-workspace.log`,
`css2-baseline-source.json` (196 inputs), `css2-final-source.json` (199 inputs),
`css2-r3-{before,final}-build-proof.json`, and source-proof text files. The final
199 source/input hashes still matched after all gates. No Mac build or test ran.

Reproduce on Windows after syncing the complete chosen base, then the lane CSS
sources. Invalidate first-party Cargo artifacts after archive-based refreshes;
otherwise restored old mtimes can leave the wrong libraries cached. Use two jobs:

```powershell
Set-Location C:\Users\server\src\wt\css
$env:CARGO_TARGET_DIR = 'C:\Users\server\target\css'
$env:CARGO_BUILD_JOBS = '2'
cargo test -p amazing-css --locked
cargo test --workspace --locked
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath tests\wpt-render\cascade-manifest.json `
  -WptRoot C:\Users\server\target\css\wpt `
  -ExecutablePath C:\Users\server\target\css\css2-r3-final.exe `
  -OutputDirectory C:\Users\server\target\css\css2-review `
  -SkipFetch -SkipBuild
```

## CSS-3 — stylesheet ownership and implicit scope through the driver

The integrator reassigned CSS-3 to close the CSS-2 caller boundary. Background
shorthand and URL-base propagation are now CSS-4. Final base: origin/dev `0e298c1`.

- Driver `<style>` and fetched `<link rel=stylesheet>` sources are parsed as
  separate sheets in document order, with `scope_root` set to the owner node's
  parent. `style` and `layout` preserve their existing inline-then-extra-file
  policy; render preserves interleaved style/link document order after scripts.
- `compute` receives the ordered sheet slice. The layout command uses existing
  public box-tree and fallback layout functions with that computed result.
- A narrowly granted additive layout entry point,
  `layout_dom_document_with_stylesheets_and_text_and_replaced_in_viewport`,
  performs the existing fallback layout, float-aware shaping, replaced content,
  and optional fixed viewport pipeline. The existing string entry point is
  unchanged, per the integrator's explicit restriction during the tables ticket.
- Raw source text remains available for legacy font discovery. This does not
  yet resolve font URLs relative to their external stylesheet.
- Driver CLI regressions exercise split versus concatenated cascade order,
  shared named layers and important reversal, variables, extra-file order,
  interleaved fetched sheets, implicit scope for style/link owners, nested scope
  proximity, layout output, and parser recovery at a stylesheet boundary.
- GPUI is not changed. The integrator must collect separate sheets after DOM
  changes, set owner-parent scope roots, and use the new sheet-aware layout API
  (or pass the slice to `compute` at its existing direct call sites).

### Final CSS-3 evidence

All builds and tests ran on win-001, source `C:/Users/server/src/wt/css`,
target `C:/Users/server/target/css`, two Cargo jobs. Final base `0e298c1`
received an integrator artifact-invalidating refresh, followed by rebase and
re-sync of the three granted source/test files. Only `css3-r3-*` evidence is
final; earlier attempts were superseded by refreshes. No new dependencies.

- Full locked workspace: **1,023 passed / 0 failed / 3 ignored**, 53 targets.
- New actual-driver CLI regressions: **6 passed / 0 failed** (included above).
- Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`;
  unchanged `tests/wpt-render/cascade-manifest.json`, 800x600, scale 1,
  lane-local WPT checkout and the same fonts/resources.
- Two baseline and two candidate runs each: **35 pass / 24 mismatch /
  13 unsupported / 0 timeout / 0 crash**. Variables: **12/12** in all four.
  Every per-case classification, differing-pixel count, maximum difference,
  and actual/reference/diff image SHA256 is unchanged across all four runs.
- All four normalized evidence hashes:
  `a259e9d395326e4c39b890dd494b159dcacb28a834ccc497e6ee94a7ec608026`.
  Normalization is the sorted per-case projection documented for CSS-2 above.
  This preserves the pinned corpus; it does not add WPT conformance passes.

The dedicated runnable **80x60 implicit-scope fixture** has three 20px bands.
A style owned by the first section scopes it green, an external link owned by
its sibling scopes that section blue, and the unscoped third section stays red.
Compared with the explicit-selector reference, the baseline differs at
**3,200 pixels**, while the candidate differs at **0 pixels**. Candidate and
reference PPM SHA256 both equal
`e38c7b3cde94c88dbc92af0c36dcfb0e197bbce0a040ea303bfee19683897ec7`.
The final PPM was copied to the Mac, converted to PNG, and visually inspected.
This is driver software-raster evidence, not a GPUI window-capture claim.
The durable equivalent fixture and assertions live in
`crates/amazing/tests/css3_stylesheets.rs`.

Source manifests cover first-party Rust/manifests, root lock/manifest, the
render runner and selected cascade manifest. **163 baseline / 164 candidate**
inputs match before/after compilation; all 164 candidate inputs match again
after the gates and match the Mac source tree. Immutable executable hashes
were checked throughout both gate runs and at final collection.

Artifacts under `C:/Users/server/target/css/`:

- Baseline: `css3-r3-before.exe`, SHA256
  `d0ef0834ed903f409ff5423bc5a30de990bc775019290a8389a2f7df9ada818d`.
- Candidate: `css3-r3-final.exe`, SHA256
  `a7ba29b18321020c61b0b2f2c8214970939b130cb3ba6aa4f83ca8f19a30c11e`.
- Cascade reports/images: `css3-r3-{before,final}-{1,2}/run-*/`.
- Scope fixture: `css3-scope.html`, `css3-scoped.css`,
  `css3-scope-reference.html`; rendered `css3-r3-{before,final}-scope.ppm`,
  `css3-r3-reference.ppm`, and `css3-r3-scope-proof.json`.
- Workspace log: `css3-r3-workspace.log`.

Local summaries, build/source proofs, fixture/PPM/PNG copies, and reproduction
helpers are in the main checkout's `.agents/css/`: `css3-r3-evidence.json`,
`css3-r3-summary.json`, `css3-r3-*-source.json`, `css3-r3-*-build-proof.json`,
`css3-r3-final-source-proof.txt`, and `summarize_css3.py`.

Reproduce after the approved tree refresh and re-sync on Windows:

```powershell
Set-Location C:\Users\server\src\wt\css
$env:CARGO_TARGET_DIR = 'C:\Users\server\target\css'
$env:CARGO_BUILD_JOBS = '2'
cargo test -p amazing --test css3_stylesheets --locked
cargo test --workspace --locked
& C:\Users\server\target\css\css3-r3-final.exe render-wpt `
  C:\Users\server\target\css\css3-scope.html `
  C:\Users\server\target\css\css3-scope-review.ppm 80 60
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath tests\wpt-render\cascade-manifest.json `
  -WptRoot C:\Users\server\target\css\wpt `
  -ExecutablePath C:\Users\server\target\css\css3-r3-final.exe `
  -OutputDirectory C:\Users\server\target\css\css3-review `
  -SkipFetch -SkipBuild
```

The existing `&str` layout API is unchanged. Exactly five driver functions
changed: `cmd_style`, `collect_style_text`, `collect_author_css`, `cmd_layout`,
and `render_surface`. All authored files remain below 4,000 lines (driver:
3,519; layout lib: 1,549; CLI regressions: 197). CSS-3 is complete; stop here.

## CSS-4 — background shorthand and stylesheet URL provenance (queued)

User originally queued this during CSS-2; **do not start without a new brief**. Replaced-content has implemented background longhands in paint.
CSS owns expansion of the full multi-layer `background` shorthand grammar;
the current single-colour arm makes shorthand values containing `url()` unusable.
CSS must also preserve the winning declaration's stylesheet-relative URL
provenance through computed values, resolving against the originating stylesheet.
Keep this work separate from CSS-2 cascade layers, revert, and scope.


CSS-4 still needs an explicit stylesheet base-URL seam: preserve the fetched
stylesheet's final URL, inline/ownerless sheet document base, and the winning
declaration's provenance through cascade/variable/shorthand computation. CSS-3
preserves sheet identity and scope roots, not URL bases. Font discovery still
uses joined source text and the page URL; the render command's extra CSS string
also retains its existing combined ownerless input. Coordinate those caller
seams with the integrator before claiming relative-resource conformance.
