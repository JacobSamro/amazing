# RC-5 fixed-base static SVG resource styling

WPT is pinned at `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
`../rc5-svg-manifest.json` freezes four existing CSS Images SVG resource
reftests: both intrinsic aspect ratios in `object-fit: fill` and `contain`.
Their upstream `support/colors-16x8.svg` and `colors-8x16.svg` resources put
literal `fill` declarations in inline SVG `style` attributes. The before
projection is the RC-4b after run on dev `253d9ec`, retained in
`prior-projections.json`: 4/4 raw matches, zero SVG credit, because both
rendered sides omitted the unsupported SVG resource. Actual, reference and
diff hashes are recorded per case. The frozen manifest SHA-256 is
`5f906f1cc7726b6f9398b36639435c63bdd8064bfbefddbf556ed20d97f7ed23`.

The owned 64×64 `../rc5-fixtures/` proof has a styled SVG, an equivalent
presentation-attribute SVG, and a blank SVG. The saved RC-4b renderer produced
the same hash for styled and blank (`d2f6d1ef...`) while the reference differed
(`d050278e...`); see `owned-before.json`. The new renderer produces the
reference hash for styled and reference in both repeated runs, and differs
from blank across all 4,096 pixels. This proves the resource contributes pixels.

On fixed dev base `0964385d49bb1a0da1822446a4e1572d91d7c925`, the primary
lane-only refresh was followed by re-sync of all 16 owned changes and a shared
REFRESH-HOLD. Ten source/manifest/fixture files matched Mac and Windows by
SHA-256 before and after all final gates. `validation.json` records the counts
and executable hash; `final-input-hashes-*.json` records every input hash.
All final builds and rendering ran on win-001. The workspace gate and exact
selection sweep acquired the shared build slot. Root nextest passed 1,396
tests with 3 skipped, and separate root doctests passed 2. The isolated image
crate passed 19 nextest tests; its separate doctest run found zero doctests.

`wpt-final-1.json` and `wpt-final-2.json` each report 4 pass, zero mismatch,
error, timeout, unsupported and renderer error. Classification, differing
pixels, maximum difference, and actual/reference/diff hashes are identical
across repeats. All four actual and reference hashes changed from the inert
prior packet. `owned-final-*.json` preserves the repeated fixture hashes.
Raw pass count did not rise. These are four non-inert selected headless
reftests, not a general SVG or unmodified-browser WPT conformance claim.

The decoder accepts only inline `fill`, `color`, `fill-opacity`, and `opacity`
declarations on the existing SVG subset. It uses the canonical CSS declaration
parser and color parser. Inline declarations override presentation attributes;
CSS `!important` determines winners among inline duplicates. Unsupported
properties or values reject the whole resource. The CSS crate does not expose
standalone SVG presentation metadata or specified-to-computed resolution, so
CSS-wide keywords, custom properties/`var()`, stylesheets, paint servers,
stroke, text, paths, filters, animation, external references, and group opacity
remain unsupported. A CSS API contract request is in lane `questions.md`.
