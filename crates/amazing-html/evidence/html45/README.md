# HTML45 scripting-mode tree-construction measurement

Pinned WPT: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
Fixed parser baseline: `605a1b95` (HTML43/44 complete).

The owned `html5lib_host` example accepts enabled/disabled scripting and an
optional fragment context, reading decoded HTML from stdin. The baseline
probe accepted the mode argument but exercised the former fixed-enabled
parser; `before.json` preserves that initial measurement: 1905 passed,
31 failed out of 1936. Every original 1894 CLI case passed. Of the 42 newly
measured explicit-script cases, 25 failures require disabled noscript parsing;
six require executing JavaScript mutations/document.write in an embedding.
Those six scripted-file cases are explicitly unsupported by the final
parser-only probe. Eight enabled and three disabled variants passed already.

The new parser configuration covers full decoded/byte documents, fragments,
incremental text/bytes and resumable parsing. Default entry points keep
scripting enabled. Disabled parsing never prepares HTML or SVG script events.

```sh
cargo build -p amazing-html --example html5lib_host --release --locked
python3 crates/amazing-html/evidence/html45/run.py "$HOME/corpora/wpt" \
  --binary "$CARGO_TARGET_DIR/release/examples/html5lib_host" --output "$OUT/html45-after.json"
```

This remains ported html5lib coverage, not unmodified WPT testharness credit.

Final measurement: **1930 passed, 0 failed, 6 unsupported**. All 1905 previous
passes remain passes; the 25 gains are parser fixes, with unchanged corpus
hashes and comparator. The original CLI corpus remains **1894/1894**, with
its 42 explicit scripting variants skipped. Native Boat gate on the fixed
605a1b95 base: **189 owned tests, 3526 workspace tests, 4 doctests passed**;
17 workspace skips. The complete InHeadNoscript mode is adapted from MIT
html5ever rules.rs at `6db7bbf1c4dfaecd28731950d427e40c8b911bef`.
