# HTML43/44 Linux gate

Fixed base: `2eb1229a`. Final parser changes: `21a7f275`; comparator: `243381b0`.
Boat pool-dom sandbox `bx_na7t27mn`, large bare-metal 8 vCPU/16 GB.
Source and target copied to native `/tmp` to avoid lazy HOME artifact faults.

- Owned nextest: 549 passed, 6 skipped.
- Workspace nextest: 3521 passed, 17 skipped.
- Workspace doctests: 4 passed.
- Full CLI corpus: 1851/1894 -> 1894/1894, zero lost passes.
- Discovered cases: 1936 across 62 pinned WPT `.dat` sources; 42 explicit
  script-mode variants skipped by this CLI measurement.
- Gains: 32 parser changes (21 HTML43, 11 HTML44), 11 dump/comparator corrections.

`before.txt` and `after.txt` retain the complete failures and counts.
`manifest.json` records source hashes and case counts from the same pinned corpus.
This is ported html5lib tree-construction coverage, not unmodified WPT credit.

Reproduce remotely after building a fresh release Amazing CLI:

```sh
bash scripts/ensure-wpt.sh
mapfile -t files < <(find "$HOME/corpora/wpt/html/syntax/parsing/resources" -maxdepth 1 -name '*.dat' -printf '%f\n' | sort)
python3 scripts/html5lib.py "$HOME/corpora/wpt" --binary "$CARGO_TARGET_DIR/release/amazing" --files "${files[@]}" --show-failures 100
```

Full nextest/doctest logs are retained in the lane's orchestration scratch.
