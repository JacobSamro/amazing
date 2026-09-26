# LAYOUT-25 — TEXT-13-routed font-kerning orientation and font-synthesis wiring

Two of the four layout-owned TEXT-13 follow-ups (`.agents/layout/questions.md`,
2026-09-26 entries), picked up per LAYOUT-23's report.md instruction to resume
authorized own-path work from that queue.

## font-kerning orientation

`font_features.rs::from_style` mapped `font-kerning: normal`/`none` to both
`kern` and `vkrn` unconditionally. CSS Fonts 4 §6.6.4 maps it to `kern` for
horizontal text and `vkrn` for vertical text only. `from_style` now takes a
`vertical: bool` (the box's `writing_mode.uses_vertical_metrics()`) and sets
only the orientation-appropriate tag; explicit `font-feature-settings` still
wins, unchanged.

Pinned `css-fonts/font-kerning-{02,03}.html`: **0/2 -> 2/2** (both exact,
0 differing pixels). Before this fix `font-kerning-02` (horizontal,
`font-kerning:normal` vs `font-feature-settings:"kern" on` equivalence)
regressed from an earlier accidental pass once `vkrn` also enabled kerning
for the reference's vertical run; `font-kerning-03` (vertical) failed the
same way. Both now match.

## font-synthesis wiring

`box_tree.rs::text_style` hard-coded `synthesis: FontSynthesis::default()`
(weight+style synthesis always on). CSS Fonts 4 §5.5 exposes `font-synthesis`
as a shorthand that `amazing-css` already expands into
`font-synthesis-{weight,style,small-caps,position}` longhands, each computed
to `auto` (default) or `none`; layout was not reading them. A new
`box_tree::font_synthesis` reads `font-synthesis-weight` and
`font-synthesis-style` (the two `amazing_text::FontSynthesis` bits the
shaper can act on today) and disables the corresponding bit on `none`. The
`small-caps`/`position` longhands are read for future use but do not change
behavior yet, since the text lane has no synthesis path for them.

Pinned `css-fonts/matching/{fixed-stretch-style-over-weight,
range-descriptor-reversed,stretch-distance-over-weight-distance,
style-ranges-over-weight-direction}.html`, which set
`font-synthesis: none`: differing-pixel counts drop from the TEXT-13-measured
13481/3584/12324/17281 to 9856/3584/8064/12096. Three of the four measurably
improve (synthesis is now correctly suppressed); none regress. All four
remain **mismatch**, not counted as gains: per CSS Fonts 4 §5.2 these fixtures
also need the font-matching algorithm to select the correct face by
weight/stretch/style descriptor before synthesis becomes moot, which
`amazing-text-provider`'s face selection does not implement yet. That is a
separate, larger provider feature outside this wiring fix; `layout25-font13-
probe.json` records the exact before/after pixel counts. `range-descriptor-
reversed` is unaffected (3584 unchanged both ways), consistent with its
mismatch being a font `src` range-descriptor issue rather than synthesis.

## Regression check

Re-ran LAYOUT-23's `gate.sh` (same committed `before` snapshots) with this
change included: position 226->255/645 (+30, the one accepted right-113
exception unchanged), writing 243->245/1112 (+2/-0), ruby 70/118 unchanged,
tables 309->310/386 (+1/-0), retained 303->304/449 (+1/-0) — identical to
LAYOUT-23's own numbers, zero additional losses. `amazing-layout` crate
nextest: 371 passed (1 new orientation test replacing the old combined one,
plus a kept explicit-override test), 1 skipped.
