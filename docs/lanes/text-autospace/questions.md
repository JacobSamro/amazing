Required real Layout consumer: actual boundary-owning ancestor policy/used metrics; adjacency excludes intervening nonzero margin/border/padding/atomic/IFC edges. Paragraph width and final line spacing after bidi, line-edge suppression and nonbreaking punctuation cannot be omitted. Native TextSpan currently carries leaf source only; never infer containing element.

Native shaping-boundary producer is complete: autospace_shaping_boundaries. Required shipping consumer applies those boundaries before glyph selection, including replacement-space edges, and handles typed NeedsReshape rather than silently dropping a gap. Ordinary glyph ligatures covering multiple visual units remain valid. Do not split glyph ink or fabricate DOM spaces.

CSS existing grammar is complete; owner adds typed projection only. Provider font catalogue/cache is separately owned and untouched. No builds/allocation yet.

Primary §8.4.1 currently defines all Han Script_Extensions characters as ideographs, including shared-script punctuation U+00B7/U+3001/U+30FB. The earlier kana-range bullet excludes P*, but the Han bullet does not. Native controls retain literal primary semantics; Unicode Z separator suppression remains independent and higher-priority per insert/replace. Issue9503 is spec editorial uncertainty, not invented merge blocker or oracle waiver.

Actual provider grouping hook: amazing-text-provider/src/lib.rs816 currently compares entire TextStyle. New autospace-only differences must not break ordinary font ligatures; native shaping_equivalent helper retains every old field except autospace. Root was asked to route only this one comparison to the provider owner or installed CSS consumer. This lane does not edit the protected provider catalogue/cache file without that assignment.
