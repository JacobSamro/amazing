# HTML-21 pinned html5lib tree probe

This is a bulk port of html5lib-derived WPT tree-construction data, not unmodified WPT harness credit. The pinned WPT revision is `53bc812da2ed81179ebd1ba76b2d197cc4747d12`; `manifest.json` inventories all 62 `.dat` files in `html/syntax/parsing/resources/` with exact SHA-256 and `#data` counts (1,936 cases). The nine selected files were frozen before parser edits, and every `#data` case in them runs (501 cases). The selected data files are stored here unchanged.

Run on a remote build host from the repository root:

```sh
cargo run -q -p amazing-html --example html21_probe --locked -- crates/amazing-html/evidence/html21 > result.tsv
```

The probe treats each `#data` block as a case in file order. It drops exactly one final LF that separates each data section from its following marker, then lets `parse` or `parse_fragment` normalize CRLF/CR input as usual. It ignores `#errors` and `#new-errors`. Expected `#document` rows are parsed mechanically: `| ` indentation gives depth, separate attribute rows attach to the preceding element and sort by name, and element/text/comment/doctype/processing-instruction rows become typed entries. Actual DOM nodes become the same entries; exact strings and tree depths are compared. Doctype public/system IDs are retained. No expected tree is edited or adjusted by case. The `raw_fnv64` column hashes the actual normalized tree or unsupported reason; whole-file SHA-256s below hash the complete raw TSV output.

Scripting-disabled runs, scripted mutation modes, and namespaced fragment contexts are outside the current HTML API; these modes are absent from the nine-file selection and remain explicitly unsupported. The 2 `unsupported` selected cases are parser panics for legacy `keygen` inputs (tests19 case 66 and webkit01 case 45). The remaining 11 failures are preserved as failures, including select-fragment legacy inputs, incomplete tail/ruby/table handling, and two processing-instruction cases.

## Results

| Source | Context | Cases | Before pass/fail/unsupported | After pass/fail/unsupported |
|---|---|---:|---:|---:|
| `adoption01.dat` | `div` | 1 | 1/0/0 | 1/0/0 |
| `adoption01.dat` | `document` | 17 | 17/0/0 | 17/0/0 |
| `blocks.dat` | `document` | 48 | 48/0/0 | 48/0/0 |
| `comments01.dat` | `document` | 16 | 16/0/0 | 16/0/0 |
| `tables01.dat` | `document` | 19 | 19/0/0 | 19/0/0 |
| `tests1.dat` | `document` | 112 | 110/2/0 | 110/2/0 |
| `tests19.dat` | `document` | 103 | 100/2/1 | 100/2/1 |
| `tests6.dat` | `body` | 1 | 0/0/1 | 1/0/0 |
| `tests6.dat` | `caption` | 2 | 2/0/0 | 2/0/0 |
| `tests6.dat` | `colgroup` | 1 | 0/1/0 | 1/0/0 |
| `tests6.dat` | `div` | 1 | 1/0/0 | 1/0/0 |
| `tests6.dat` | `document` | 39 | 36/3/0 | 36/3/0 |
| `tests6.dat` | `frameset` | 1 | 0/0/1 | 1/0/0 |
| `tests6.dat` | `html` | 1 | 0/0/1 | 1/0/0 |
| `tests6.dat` | `table` | 1 | 0/1/0 | 1/0/0 |
| `tests6.dat` | `tbody` | 2 | 1/1/0 | 2/0/0 |
| `tests6.dat` | `td` | 1 | 1/0/0 | 1/0/0 |
| `tests6.dat` | `tr` | 2 | 0/2/0 | 2/0/0 |
| `tests_innerHTML_1.dat` | `body` | 4 | 0/0/4 | 4/0/0 |
| `tests_innerHTML_1.dat` | `caption` | 16 | 15/1/0 | 16/0/0 |
| `tests_innerHTML_1.dat` | `colgroup` | 2 | 0/2/0 | 2/0/0 |
| `tests_innerHTML_1.dat` | `div` | 2 | 2/0/0 | 2/0/0 |
| `tests_innerHTML_1.dat` | `frameset` | 1 | 0/0/1 | 1/0/0 |
| `tests_innerHTML_1.dat` | `html` | 4 | 0/0/4 | 4/0/0 |
| `tests_innerHTML_1.dat` | `select` | 4 | 2/2/0 | 2/2/0 |
| `tests_innerHTML_1.dat` | `table` | 11 | 9/2/0 | 11/0/0 |
| `tests_innerHTML_1.dat` | `tbody` | 10 | 4/6/0 | 10/0/0 |
| `tests_innerHTML_1.dat` | `td` | 16 | 14/2/0 | 16/0/0 |
| `tests_innerHTML_1.dat` | `tr` | 11 | 3/8/0 | 11/0/0 |
| `webkit01.dat` | `document` | 52 | 49/2/1 | 49/2/1 |

Two baseline rounds were byte-identical at **450 pass, 37 fail, 14 unsupported**. Two post-edit rounds were byte-identical at **488 pass, 11 fail, 2 unsupported**. `transitions.tsv` records 26 fail-to-pass and 12 unsupported-to-pass transitions, 450 retained passes, zero lost passes, 11 unchanged failures, and 2 unchanged unsupported cases.

- `before-1.tsv` SHA-256 `e7ef427f20af89b1353353bcef0e2c0f56103a39719a49935068dec6b2214962`
- `before-2.tsv` SHA-256 `e7ef427f20af89b1353353bcef0e2c0f56103a39719a49935068dec6b2214962`
- `after-1.tsv` SHA-256 `fb5dabf1ac0fbc34da7360672e8633bb2827797a54e0e5fe468c0ed464aa5708`
- `after-2.tsv` SHA-256 `fb5dabf1ac0fbc34da7360672e8633bb2827797a54e0e5fe468c0ed464aa5708`

The implementation makes fragment contexts virtual in the tree builder, retains the synthetic html root while clearing table-family modes, and projects only the root children (plus comments after an explicit html end in an `html` fragment). Focused regression tests cover table, row, section, column group, caption, html/body/frameset contexts, and malformed-token scalar chunk equivalence.

## Windows final gate

The fixed-base win-002 `win-run.sh` extraction passed package nextest **153/153**, package doctest **1/1**, workspace nextest **2,043/2,043 with 5 skipped**, and workspace doctests **2/2**. The 30-file source/data/lock SHA-256 audit in `source-hashes.json` matched before and after. Both Windows post-edit probe outputs (preserved byte-for-byte as `windows-after-{1,2}.tsv.gz`) have uncompressed SHA-256 `fb18d8f9d2bfec9fe2da331af83c498a5491b870e81c73593df590c62b2192c9`; their 501 parsed rows, including per-case raw hashes, match the Linux post-edit rows exactly. PowerShell `Out-File` adds a UTF-8 BOM and Windows newlines, so the raw Windows TSV SHA-256 differs from the Linux TSV SHA-256. The complete command output is in `windows-final-gate.log`.
