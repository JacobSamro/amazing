# Lossless UTF-16 frontend

This complete qualified successor is based on recorded origin/dev `601d78162b8075b5f0b40682cf245c472e4a8bc8`. It preserves all eleven original native/original-AST frontend groups and the pure RegExp controls from qualified `a9e78821`. The genuine TemplateSite/cache prerequisite is published on this fixed base; the clone, worker and foreign-realm template controls remain intact. Fresh Linux qualification retained all passing files and variants in both modes.

`LexerSource` retains original UTF-16 units, decoded real code points and point-to-unit boundaries. Lone surrogates remain distinct from replacement characters and private-use characters. Literal and regular-expression scanning preserves exact units; local string/template line rules retain actual source positions. The compatible string APIs encode their input and use the same scanner and parse goals as `parse_script_utf16`, `parse_module_utf16`, and `parser::parse_eval_utf16`. Token regexp sidecars feed existing `Regex::new_utf16`; Unicode identity-escape validation rejects non-scalar escapes while retaining permitted unescaped surrogate atoms and legacy identity escapes.

Ported algorithms: boa-dev/boa `39cd11214ecb366200157dab96b3919e61e9a391`, `core/parser/src/source/utf16.rs` and `core/parser/src/lexer/{cursor,string,template}.rs`, MIT. Existing mandatory headers and the exact notice row are preserved. Primary SourceText and ParseText: https://tc39.es/ecma262/multipage/ecmascript-language-source-code.html#sec-source-text .

This ticket changes scanner/token/parser source flow only. Runtime eval/dynamic-function consumers, retained callable source, typed static module requests, AST/thread ownership, frames and jobs keep their existing contracts.

The paired baseline contained the identical authored controls with test-only adapters that invoke the unchanged published string parse APIs through `String::from_utf16_lossy`. This explicitly measures the existing pre-API lossy path; no production UTF-16 placeholder is added. The actual baseline had eight authored failures; all passed after the change. Both sides completed workspace nextest, four doctests, release host and ten whole Test262 selectors in default and VM modes: language, String, RegExp, Function, Object, Proxy, Promise, AsyncFunction, GeneratorFunction and AsyncGeneratorFunction. Test262 pin, 60-second deadlines and eight jobs remain unchanged.

Historical qualification is identified separately in [the compact provenance record](js-lang_utf16-frontend-historical.json). Its counts are not a fresh gate on this origin base.

Fresh gate: workspace **4564 passed / 8 authored failed / 17 skipped → 4572 passed / 0 failed / 17 skipped**, with all legacy PASS/SKIP identities retained; four doctests passed on both sides. Each mode retained **32,758 passing files / 61,664 passing variants** across 33,181 unique selected files, with zero gains or losses. All forty corpus reports used the same pinned input, runner, profile, eight jobs and 60-second deadlines. Native raw reports, journals and both shipping binaries remain on Boat with authenticated durable copies.

A candidate shipping check stopped before compiling because its prepared checksum was stale. The unchanged candidate completed the single permitted tooling rerun after correcting that checksum against every baseline controller and the actual native ELF. The failed command and log remain in scratch; no source or corpus settings changed.

[Fresh counts and provenance](js-lang_utf16-frontend-current-gate.json) and [exact compact pass sets](js-lang_utf16-frontend-current-passsets.json) record the qualified comparison.
