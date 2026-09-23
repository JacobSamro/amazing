# amazing-regex

A std-only, bounded subset of ECMA-262 RegExp. This is a new implementation,
not a wrapper around another regex library. `Regex` parses and compiles an
immutable pattern; `Matcher` borrows UTF-16 input and owns the budget for one
operation. The JavaScript interpreter owns RegExp objects and `lastIndex`.

## Supported surface

- Flags `g`, `i`, `m`, `s`, `u`, `v`, `y` on both constructors; duplicate,
  unknown, and combined `u`/`v` flags are rejected.
- Literals, dot, character classes, negation, ranges, digit/word/space escapes,
  hexadecimal and Unicode escapes, control escapes and word boundaries.
- Alternation, capturing and noncapturing groups, decimal backreferences,
  positive and negative lookahead, start/end assertions.
- Greedy and lazy `*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}`. Repetition clears
  nested captures and rejects optional zero-progress iterations. Lookahead
  captures propagate on success and cannot be revisited by outer backtracking.
- UTF-16 code-unit indices; `u` and `v` modes consume code points. `new_utf16`
  parses exact pattern code units and `exec_utf16` accepts exact input code
  units, including lone surrogates. `pattern_utf16` retrieves the original
  pattern losslessly; the older `pattern` accessor is lossy for such patterns.
- Unicode `i` canonicalization uses the C/S mappings from Unicode 17.0.0.
  Legacy `i` uses Rust's single-character uppercase mapping with ECMAScript's
  non-ASCII-to-ASCII restriction.
- UnicodeSets `v` classes support nested classes, union, intersection (`&&`),
  subtraction (`--`), class strings (`\q{...}`), and code-point ranges. String
  alternatives match longest first and backtrack when later atoms fail.
  Unicode 17.0 emoji properties of strings and `ASCII_Hex_Digit` are supported.

`Regex::new` remains a convenience constructor for Rust strings; `new_utf16`
accepts JS strings with unpaired surrogates. `Regex::exec` searches
losslessly; `matcher_utf16` shares one work budget across repeated searches.
The `g` flag is metadata here; the host must implement global iteration and
advance empty matches using `advance_index`. Sticky matching is enforced by
this crate. An exhausted matcher remains exhausted.

The amazing-js binding adds parser-directed regex literal lexing, `RegExp`,
`exec`, `test`, `toString`, source/flag getters, `lastIndex`, and native-regex
String `match`, `search`, and `replace` (captures and callback replacements).
A literal is validated at parse time and produces a fresh object when evaluated.
The parser chooses regex versus division by its operand grammar position.

The pinned upstream testharness.js additionally required block-aware template
substitutions, String.fromCharCode, lookup of exports on the global object,
actual arguments.length and rest-parameter arrays, Function identity,
Array.prototype exposure, Object.prototype.hasOwnProperty and Array.some.
These prerequisites are covered by the JS integration tests. The
`amazing-js` example `probe_harness` accepts the unchanged harness on stdin,
supplies only `self = globalThis`, and checks real PASS/FAIL and completion
callbacks. It is a shell integration probe, not a DOM conformance run.

## Deliberate gaps

This is not full ECMAScript conformance. Unsupported pattern forms produce an
error rather than being silently interpreted as a different regex:

- Named captures/references, lookbehind, inline flag modifiers, most Unicode
  character property escapes, and `d` indices. Unsupported properties are
  rejected explicitly.
- Annex B octal/decimal escape fallbacks, incomplete hex/Unicode escapes,
  quantified lookahead and legacy class-range escape exceptions. Some otherwise
  valid legacy identity escapes (`\p`, `\P`, `\k`) are conservatively rejected.
- The JS binding does not implement the RegExp Symbol protocols, custom `exec`
  dispatch in String methods, species/subclass construction, regex-aware
  `String.split`, `matchAll`, `replaceAll`, `RegExp.escape`, legacy static capture
  properties or `compile`. `RegExp.prototype.test` does dispatch overridden exec.
- The JS binding must pass its lossless string units to `new_utf16` and
  `exec_utf16` for end-to-end surrogate preservation. This includes
  non-Unicode dot matching half an astral character.
- Broader interpreter gaps (strict mode, descriptors, built-in prototypes,
  Symbols, and Test262 helper loading) still affect integration conformance.

## Resource bounds

Patterns are capped at 16,384 UTF-8 bytes via `new` or 16,384 UTF-16 code
units via `new_utf16`, 64 nested groups and 128 captures.
Compilation has a 32,768-unit instruction/work budget, including expanded
quantifiers. Matching allows 1,048,576 UTF-16 units and 1,000,000 execution steps
shared across search positions, lookahead and global iteration. Backtracking
uses an explicit stack capped at 8,192 frames and 1,048,576 saved storage slots.
Nested lookahead is additionally bounded by parser nesting. Limits return
`Error::Limit`; JS execution reports RangeError. Parse-time compilation failures
are reported as positioned syntax errors by the JS parser.

JS match/replacement collection permits at most 100,000 matches; replacements
also cap collected capture slots at 1,048,576 and output at 16,777,216 bytes.
These implementation limits intentionally reject some valid expensive patterns
or inputs. Work accounting is a deterministic execution bound, not a wall-clock
or exact heap-byte guarantee. Matching is backtracking, not linear-time.

## Data and specification

`src/unicode_casefold.rs` contains 1,512 C/S pairs extracted from
[Unicode 17.0.0 CaseFolding.txt](https://www.unicode.org/Public/17.0.0/ucd/CaseFolding.txt).
To regenerate, strip `#` comments, split nonempty rows on `;`, retain statuses
`C` and `S`, parse source/target as hexadecimal integers, and sort by source.
The accompanying `UNICODE_LICENSE.txt` contains the Unicode data license.
The two `data/emoji-*-17.0.txt` files are pinned copies of Unicode's
[Emoji 17.0 data](https://www.unicode.org/Public/17.0.0/emoji/); the same
license applies. `RGI_Emoji` is the union of the six listed property sets.

Semantics follow the [ECMA-262 RegExp algorithms](https://tc39.es/ecma262/multipage/text-processing.html#sec-regexp-regular-expression-objects)
and [regex literal grammar](https://tc39.es/ecma262/multipage/ecmascript-language-lexical-grammar.html#sec-literals-regular-expression-literals).
Measured JS-2 Test262 results and remaining work are recorded in
`docs/lanes/js-lang_TODO.md`; test counts refer to the project's existing harness,
whose limitations are documented there, rather than full Test262 conformance.
