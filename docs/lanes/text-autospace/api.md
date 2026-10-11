Native source-only typed contract on published09e.

`TextStyle.autospace: TextAutospace` is shaping/cache identity; default NORMAL. TextAutospace flags alpha/numeric/punctuation + AutospaceMode Insert/Replace; NORMAL/AUTO alpha+numeric insert, NONE zero flags. CSS keeps canonical author keyword serialization separately.

`plan_autospace(text, runs: &[ShapedRun], visual_clusters: &[Range<usize>], resolve: impl FnMut(&Range<usize>, &Range<usize>) -> Option<AutospaceContext>) -> Result<AutospacePlan, AutospaceError>` takes an authentic single line after bidi. Context must come from the actual innermost common boundary element: owner, computed policy, used ic/actual shaped NBSP/NNBSP advances, French content language and real adjoining-box state. None is a genuinely excluded boundary, not a guessed current style.

`apply_autospace_plan(text, line: &mut ShapedLine, plan: AutospacePlan) -> Result<Vec<AutospaceAdjustment>, AutospaceError>` consumes the move-only plan once, validates every real cluster before mutation, adjusts existing advances only and returns owner/nonbreaking/replacement receipts. Actual source/glyph/cluster identities remain intact. Caller uses receipt ownership for paint; no DOM spaces or replacement of source strings.

Layout must invoke both intrinsic/wrap and final post-bidi line paths, using actual LCA/IFC and nonzero inline edge guards. French nonbreaking receipts must affect candidate breaks before line selection. Source-only native controls do not prove that shipping consumer exists.

Provider cluster closure: visual_clusters are typographic grapheme ranges, while a glyph.cluster can cover multiple visual units. Planning maps units to genuine covering glyph ranges; ordinary non-spacing ligatures remain valid. Adjustment attachment ranges are actual glyph clusters, while the boundary resolver receives original unit ranges. Zero-glyph invisible formatting is skipped without inserting text. A qualifying boundary swallowed by one glyph yields AutospaceError::NeedsReshape(byte_offset); the caller must apply pre-shaping candidate boundaries and reshape, never silently omit spacing. Candidate boundaries include ASCII-space removal edges for replace.

Native TextStyle::shaping_equivalent(&self, other: &Self) retains all twenty-one legacy shaping equality fields and excludes only autospace. Full PartialEq includes policy for paragraph/layout cache identity. Actual provider grouping must call this helper instead of full style equality; explicit eligible spacing boundaries control font-run splits. No font/catalogue/cache fields changed.
