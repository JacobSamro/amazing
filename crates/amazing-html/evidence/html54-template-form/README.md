# Actual template fragment form pointer correction

Fixed base `8f2bbaa05b2c8ceced0c915dab8eff4cb4197c21`; final source `f08a4af61e5a343db285064159d2dc765a98a8ac`.

The current HTML Standard (verified 2026-10-03), [element pointers](https://html.spec.whatwg.org/multipage/parsing.html#the-element-pointers), defines parsing template contents to include an actual HTML template fragment context. Apply that predicate to form start/end and table form handling while preserving the ancestor form pointer. Foreign namespaces and uppercase non-HTML-template contexts retain their actual behavior.

Boat native `/tmp` source/target gate: 629 owned tests, 3,873 workspace tests, 17 workspace skips, four doctests. Both full html5lib selections retained all 1,894 and 1,930 passes (six expanded unsupported). Whole pinned WPT html/syntax: 220 pages, 3,095→3,096 passing assertions, zero lost assertion multiplicities or passing files. The exact gained assertion restores the accepted template form-pointer regression. Raw artifacts remain in MAIN `.agents/html/html-template-form-before/` and `html-template-form-final-after/`; compact name hashes and source pins are committed here.

Source series: `6480feb9` algorithm/native group; mechanical iterator correction `97b34818` (root equivalent `c49fc3e4`); `f08a4af6` stale native table-form expectation correction. The latter asserts actual inner form insertion and preserves the later outer form-pointer assertion. No WPT expectations changed.
