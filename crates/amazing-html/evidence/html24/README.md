# HTML-24 pinned html5lib tree probe

This is a ported html5lib-derived parser corpus, not an unmodified WPT harness result. Fixed base `7c61b9fcc33078685d2e552e648f2e486f93492b` includes the locally merged HTML-23 commit. Pinned WPT revision `53bc812da2ed81179ebd1ba76b2d197cc4747d12` supplies all 15 previously unselected complete files and 347 `#data` cases. `manifest.json` records each exact SHA-256, byte count, file case count, 346 document/1 html fragment contexts, and 327 default/19 script-off/1 script-on modes. All source bytes were checked against the HTML-21 inventory before parser edits.

The probe removes one section-delimiter LF from `#data`; the HTML API then performs CRLF/CR normalization. It compares typed tree rows from `| ` indentation, sorted attributes, doctype, comment, text, PI and element nodes. Expanded foreign attributes are serialized as `prefix local-name` in expected and actual trees; ordinary attributes keep their qualified name. This corrected mechanical normalization **before** the frozen baseline, accounting for four `tests9.dat` cases previously misclassified by the older probe. Parse-error rows are ignored. Unsupported scripting modes and expected rows stay `unsupported`; panics are `crash`. No case-specific expectation was edited.

Remote rerun commands from the repository root:

```sh
cargo run -q -p amazing-html --example html24_probe --locked -- crates/amazing-html/evidence/html24 > new.tsv
cargo run -q -p amazing-html --example html23_probe --locked -- crates/amazing-html/evidence/html23 > retained23.tsv
cargo run -q -p amazing-html --example html22_probe --locked -- crates/amazing-html/evidence/html22 > retained22.tsv
cargo run -q -p amazing-html --example html22_probe --locked -- crates/amazing-html/evidence/html21 --retained > retained21.tsv
```

| Source | Cases | Before pass/fail/unsupported/crash | After pass/fail/unsupported/crash |
| --- | ---: | ---: | ---: |
| `tests20.dat` | 64 | 63/1/0/0 | 63/1/0/0 |
| `tests7.dat` | 34 | 31/2/0/1 | 31/2/0/1 |
| `tests9.dat` | 27 | 27/0/0/0 | 27/0/0/0 |
| `tests25.dat` | 26 | 24/0/0/2 | 24/0/0/2 |
| `tests3.dat` | 24 | 23/1/0/0 | 23/1/0/0 |
| `ruby.dat` | 21 | 19/2/0/0 | 19/2/0/0 |
| `noscript01.dat` | 18 | 0/0/18/0 | 0/0/18/0 |
| `tests26.dat` | 20 | 16/4/0/0 | 20/0/0/0 |
| `menuitem-element.dat` | 20 | 20/0/0/0 | 20/0/0/0 |
| `tests5.dat` | 17 | 15/0/2/0 | 15/0/2/0 |
| `tests15.dat` | 14 | 14/0/0/0 | 14/0/0/0 |
| `tests17.dat` | 13 | 12/1/0/0 | 12/1/0/0 |
| `tests11.dat` | 13 | 13/0/0/0 | 13/0/0/0 |
| `void-in-phrasing.dat` | 13 | 13/0/0/0 | 13/0/0/0 |
| `tests21.dat` | 23 | 23/0/0/0 | 23/0/0/0 |
| **Total** | **347** | **313/11/20/3** | **317/7/20/3** |

Two byte-identical before rounds have SHA-256 `8092995a86bb851f92a72c69e79e879b61dc5ca1e8e7a96019541c7eb890d18a`; two byte-identical after rounds have `6ba474ad95b3b3cd6bc9946f85dda53300e04b0f45a5e1bfd6b6e5098717168b`. Exact transitions: 4 fail-to-pass, 313 retained passes, 7 retained failures, 20 retained unsupported, 3 retained crashes, zero losses. `new-transitions.tsv` records every case and raw output hash. The three keygen panics remain classified as crashes.

The parser repair handles `</p>` and `</br>` while in SVG or MathML: it exits the foreign stack before in-body recovery creates an HTML sibling. Other unmatched foreign end tags leave that stack open. Focused tests cover all four `tests26.dat` gains and the two initially regressed foreign cases. Linux package iteration passed 162/162 nextest tests and 1/1 doctest.

HTML-21 retention remains **489/11/0/1** with identical classifications and per-case hashes (SHA-256 `22ba9a5f7dafc0ccedd26f1badd67a42ecd64d8ef4ce29f23b9db9f87bf036ed`). HTML-22 retention moves from **303/11/75/0** (SHA-256 `d028f5d18a6fa74976bd46b2acf2bde13424c86faa07654cb09cbefd4a6e7812`) to **305/9/75/0** (SHA-256 `4d2071b9254475b1a2cd8098081070b83f3a23ab28a960f9ca728c7c6edfaa64`): two foreign-fragment cases gain, all 303 prior passes and other hashes remain. HTML-23 retention remains **355/11/7/0** with identical classifications and per-case hashes (SHA-256 `1618170b90739bffca2e7bff42656816228a6657de60864abbc568f1ac379e00`). The three retention transition tables preserve every row. All selections ran twice before and twice after.

`source-hashes.json` audits the fixed-base source, selected fixtures, probes, and root lockfile before and after the Windows gate. `windows-gate.ps1` contains the commands. Windows raw TSVs are preserved as deterministic gzip files; PowerShell adds a UTF-8 BOM and Windows newlines, so raw-file hashes differ from Linux. Parsed classifications and each raw FNV64 value are compared across hosts.

The final fixed-base win-002 gate passed package nextest **162/162**, package doctest **1/1**, workspace nextest **2,149/2,149 with 5 skipped**, and workspace doctests **2/2**. All 64 source/fixture/lock hashes matched before and after; Cargo.lock SHA-256 stayed `bea8c2e2a566cfa165f0cd1ae02f06b59ce9894f44375e142304e9e356626e7c`. Both raw Windows rounds were byte-identical per selection, and their parsed rows matched both Linux after rounds. The uncompressed Windows TSV SHA-256s are `a9250ba2e21e1a843033d0b0d15daf92d880c57f0ce753dc1d51c1178a9e8ce4` (new), `d4208ea36b966fe0c1ee8b8cc6685f1c9a5332e477266a166516945a1bb9eacd` (HTML-21), `d9c31a6ada4f2b075691f66f0f25345a61661c7cbf973599780cb2ad139e9c63` (HTML-22), and `ac107a95b186884546b9eb2c79996cbe2a6c2d07bbd4c55c92da4bee4d36802d` (HTML-23). Full output is in `windows-final-gate.log`.
