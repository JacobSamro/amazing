# Whole grid track component boundaries

Recorded origin/dev base `a5a56a90e1fa507fa9babb56e6f64636b496f93f`.
Historical initial source `654358e6`: authentic native exit 1. Workspace 5011 passed/1 failed; docs 36 passed. All 945 grid cases retained 413 passes, CSS17 stayed 4350/5126, but one containment case lost a pass as crash (storage startup os107). The string-template control revealed an incomplete shorthand consumer. All original assertions and initial failures are preserved; the genuine successor is documented separately.

The existing grid grammar split component lists only at whitespace. Valid
`[one]50px[two]50px[three]50px` was therefore dropped before layout could consume
its three tracks and named lines. The independent layout dense ticket retains
its original geometry/subgrid/name assertions; no layout files are changed here.

Grid now consumes atomic token components: primitives, complete bracket blocks
and complete functions, respecting nested delimiter kinds. All original tokens
remain borrowed, including string content and decoded identifier escapes;
there is no character-based re-tokenization. The existing contiguous `[a][b]`
line-name group remains accepted. Track dimensions/ranges, repeat arity and
fixed-size restrictions, name grammar and shorthand expansion still use their
existing validators. The global non-grid whitespace helper is unchanged.

The same component boundary rule covers explicit rows/columns, integer and
auto-repeat bodies, implicit tracks, fixed track lists, string template rows and
both auto-flow shorthand axes. Original CSS6 controls remain intact; six new
bulk groups cover borrowed token identity/nesting, the authentic dense producer
value, adjacent named/repeated/function tracks, implicit/template/shorthand
forms, variable resolution/serialization/cascade, and invalid grammar/arity.
No external algorithm was copied; the scanner adapts our existing balanced
delimiter handling to the grid producer seam.

References: [CSS Syntax component values](https://drafts.csswg.org/css-syntax-3/#consume-component-value)
and [CSS Grid track lists](https://drafts.csswg.org/css-grid-2/#track-sizing).

The admitted initial gate retained all 17 parser selections and the whole eight relevant render cohorts (2421 cases, including 945 grid), workspace nextest and doctests. It used authentic c220 BEFORE maps. Raw output/source/999 captured executables remain on bx_u5k4sfjy. The completed diagnostic mission extracted original context only; no test or corpus was rerun.
