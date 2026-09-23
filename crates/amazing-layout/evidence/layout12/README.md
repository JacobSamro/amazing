# LAYOUT-12 evidence

Fixed source base: `7c6824b8a12a50ea6ba99688678b98617bd27f26`.
WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
`source.json` records the base/source archive hashes, five edited source file
hashes verified on win-001, and immutable renderer hashes. The final renderer
is `C4BAD39E094B064BFDA3413430D3036AE30B51CCF0BDFF590D7627B0A1445F1B`.

`broad-before-{1,2}.json` and `broad-after-{1,2}.json` are the complete raw
Windows WPT render reports. Both repetitions classify and hash identically:
86/200 to 93/200, seven gains and zero losses. `gate-summary.json` names each
gain and maps it to the pinned Chrome/Safari oracle in
`../layout5/browser-oracles.json`: five consensus gains, zero consensus losses.
The two-case `diagnostic-*` files show the initial red/green root-cause loop;
`broad-candidate-diagnostic.json` exposed the safe-alignment loss corrected
before the final renderer was frozen.

`retained-before/` and `retained-after/` each contain two raw reports for ten
selections. Every selection retained its pass count and actual hashes. The
image selection uses the same pinned image-capable executable as LAYOUT-11.

CSS-12 then landed at `7a1e31e`. `font-before.json` uses CSS-12 without the
layout font mapper; `font-after.json` uses the rebased LAYOUT-12 source. The
pinned ligature reftest moves 0/1 to 1/1, or 424 differing pixels to zero.
`font-source.json` records both source archives and distinct executable hashes.
The first candidate build reused the baseline executable because extraction
preserved file times; it was discarded and rebuilt with fresh file times.
`win002-gate/` contains the raw Windows nextest and doctest logs for the
rebased source. Counts are in `gate-summary.json` and the lane report.
