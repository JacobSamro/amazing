# HTML-22 pinned html5lib tree probe

This is a ported html5lib-derived parser corpus, not an unmodified WPT harness result. Revision `53bc812da2ed81179ebd1ba76b2d197cc4747d12` supplies all five complete source files. `manifest.json` records each exact SHA-256 and case count. The file list was frozen before parser edits: 389 cases in `foreign-fragment.dat`, `tests2.dat`, `tests4.dat`, `tests10.dat`, and `tests16.dat`. All `#data` blocks in those files run, including document and fragment contexts. The existing 501-case HTML-21 selection runs under the same probe with `--retained`.

Remote rerun commands from the repository root:

```sh
cargo run -q -p amazing-html --example html22_probe --locked -- crates/amazing-html/evidence/html22 > new.tsv
cargo run -q -p amazing-html --example html22_probe --locked -- crates/amazing-html/evidence/html21 --retained > retained.tsv
```

The probe removes exactly one LF separating `#data` from the next section, then uses the HTML API for CRLF/CR normalization. It ignores parse-error sections. Expected `#document` rows are converted mechanically to typed depth/value entries: the `| ` indentation sets depth; an attribute row one level below its element, including names beginning with `<`, attaches to that element; attributes sort before comparison. Quoted text rows remain text even if they contain `="`. Actual DOM nodes use the same representation. No case-specific expected edits are made. `raw_fnv64` hashes the normalized actual tree or reason string. Complete raw TSV SHA-256s are below.

Scripting-on mutation and scripting-off modes, namespaced foreign fragment contexts, and unsupported HTML fragment context names are classified `unsupported`; parser panics are classified `crash`. The new selection has 76 unsupported cases: 63 namespaced foreign fragments, 12 scripting-mode cases and 1 `head` fragment. It has no crashes. The retained HTML-21 selection has one legacy-keygen parser crash. These modes and failures receive no conformance pass credit.

## New tranche results

| Source | Context | Cases | Before pass/fail/unsupported/crash | After pass/fail/unsupported/crash |
|---|---|---:|---:|---:|
| `foreign-fragment.dat` | `div` | 3 | 1/2/0/0 | 1/2/0/0 |
| `foreign-fragment.dat` | `math annotation-xml` | 3 | 0/0/3/0 | 0/0/3/0 |
| `foreign-fragment.dat` | `math malignmark` | 1 | 0/0/1/0 | 0/0/1/0 |
| `foreign-fragment.dat` | `math math` | 3 | 0/0/3/0 | 0/0/3/0 |
| `foreign-fragment.dat` | `math mfenced` | 1 | 0/0/1/0 | 0/0/1/0 |
| `foreign-fragment.dat` | `math mi` | 5 | 0/0/5/0 | 0/0/5/0 |
| `foreign-fragment.dat` | `math mn` | 5 | 0/0/5/0 | 0/0/5/0 |
| `foreign-fragment.dat` | `math mo` | 5 | 0/0/5/0 | 0/0/5/0 |
| `foreign-fragment.dat` | `math ms` | 5 | 0/0/5/0 | 0/0/5/0 |
| `foreign-fragment.dat` | `math mtext` | 5 | 0/0/5/0 | 0/0/5/0 |
| `foreign-fragment.dat` | `svg desc` | 10 | 0/0/10/0 | 0/0/10/0 |
| `foreign-fragment.dat` | `svg foreignObject` | 3 | 0/0/3/0 | 0/0/3/0 |
| `foreign-fragment.dat` | `svg path` | 5 | 0/0/5/0 | 0/0/5/0 |
| `foreign-fragment.dat` | `svg svg` | 9 | 0/0/9/0 | 0/0/9/0 |
| `foreign-fragment.dat` | `svg title` | 3 | 0/0/3/0 | 0/0/3/0 |
| `tests10.dat` | `document` | 54 | 49/5/0/0 | 49/5/0/0 |
| `tests16.dat` | `document` | 197 | 171/14/12/0 | 185/0/12/0 |
| `tests2.dat` | `document` | 63 | 56/7/0/0 | 59/4/0/0 |
| `tests4.dat` | `div` | 1 | 1/0/0/0 | 1/0/0/0 |
| `tests4.dat` | `head` | 1 | 0/0/1/0 | 0/0/1/0 |
| `tests4.dat` | `html` | 1 | 1/0/0/0 | 1/0/0/0 |
| `tests4.dat` | `plaintext` | 1 | 1/0/0/0 | 1/0/0/0 |
| `tests4.dat` | `script` | 1 | 1/0/0/0 | 1/0/0/0 |
| `tests4.dat` | `style` | 1 | 1/0/0/0 | 1/0/0/0 |
| `tests4.dat` | `textarea` | 2 | 2/0/0/0 | 2/0/0/0 |
| `tests4.dat` | `title` | 1 | 1/0/0/0 | 1/0/0/0 |

Two byte-identical before runs: **285 pass / 28 fail / 76 unsupported / 0 crash**. Two byte-identical after runs: **302 pass / 11 fail / 76 unsupported / 0 crash**. Exact transitions: 17 fail-to-pass, 285 retained passes, 11 retained failures and 76 retained unsupported cases, with zero losses. `transitions.tsv` contains each case and its raw output hashes.

## HTML-21 retention

The 501-case selection has **489 pass / 11 fail / 0 unsupported / 1 crash** both before and after. All classifications and raw per-case hashes are identical; `retained-transitions.tsv` records every row. The HTML-22 probe recognizes an attribute whose name begins with `<` in one retained expected tree, which the earlier HTML-21 probe had marked unsupported; the parser passes that case. The legacy-keygen panic is now reported as a crash.

## Raw SHA-256s

- `before-1.tsv`: `5425307bc7b36da79e7e2c0e1db68cff0d5d48305a3bdefafd38f1fdc6383f83`
- `before-2.tsv`: `5425307bc7b36da79e7e2c0e1db68cff0d5d48305a3bdefafd38f1fdc6383f83`
- `after-1.tsv`: `0691789d6f191bff854f8a28570ad7776f57c858081e4d83feb63b729b5dc1b6`
- `after-2.tsv`: `0691789d6f191bff854f8a28570ad7776f57c858081e4d83feb63b729b5dc1b6`
- `retained-before-1.tsv`: `22ba9a5f7dafc0ccedd26f1badd67a42ecd64d8ef4ce29f23b9db9f87bf036ed`
- `retained-before-2.tsv`: `22ba9a5f7dafc0ccedd26f1badd67a42ecd64d8ef4ce29f23b9db9f87bf036ed`
- `retained-after-1.tsv`: `22ba9a5f7dafc0ccedd26f1badd67a42ecd64d8ef4ce29f23b9db9f87bf036ed`
- `retained-after-2.tsv`: `22ba9a5f7dafc0ccedd26f1badd67a42ecd64d8ef4ce29f23b9db9f87bf036ed`

The parser repair discards an appropriate script/RAWTEXT/RCDATA end-tag candidate at EOF after its name instead of replaying the partial tag as text. A numeric character reference with no digits leaves `&#`, `&#x`, or `&#X` literal. Owned regression tests exercise the expected DOM tree and every scalar chunk split. The 11 unchanged new-tranche failures are preserved in the transition table.

## Windows final gate

The fixed-base win-002 `win-run.sh` extraction refreshed and re-synced this worktree once. The 35-file source/fixture/lock audit in `source-hashes.json` matched before and after. Package nextest passed **155/155**, package doctest **1/1**, workspace nextest **2,079/2,079 with 5 skipped**, and workspace doctests **2/2**. Both Windows post-edit runs matched the Linux after rows exactly, including every per-case raw hash, for the 389-case tranche and 501-case retention selection. Raw Windows TSV outputs are preserved byte-for-byte in `windows-*.tsv.gz`; their uncompressed SHA-256s are `f74e3dfe8939189800c1ff9b0af1d0483d50fa1a19deff05a9e3511a42e1c92b` for the tranche and `d4208ea36b966fe0c1ee8b8cc6685f1c9a5332e477266a166516945a1bb9eacd` for retention. PowerShell adds a UTF-8 BOM and Windows newlines, so these raw-file hashes differ from Linux despite identical parsed rows. Full command output is in `windows-final-gate.log`.
