# HTML-23 pinned html5lib tree probe

This is a ported html5lib-derived corpus at WPT revision `53bc812da2ed81179ebd1ba76b2d197cc4747d12`, not an unmodified WPT harness result. `manifest.json` freezes seven complete, previously unselected source files and all 373 `#data` cases before parser edits. Their exact SHA-256 hashes, context counts (369 document, 3 div, 1 select), and input modes (369 default, 3 script-off, 1 script-on) are recorded there. Source bytes were fetched by pinned revision and checked against the HTML-21 inventory.

The HTML-22 probe mechanics are used unchanged except that this probe strips the closing `?>` marker from an expected processing-instruction row before comparing its target/data with the DOM. That corrects serialization normalization before baseline measurement; it does not edit a case-specific expectation. The probe removes one section-delimiter LF from `#data`, then the HTML API performs CRLF/CR normalization. It turns `| ` indentation into typed depth/value entries, attaches and sorts attribute rows, and compares exact trees. Parse-error sections are ignored. Scripting modes the parser cannot emulate, unsupported expected rows, and unsupported fragment contexts are classified `unsupported`; panics are `crash`. No unsupported case receives pass credit.

Remote rerun commands from the repository root:

```sh
cargo run -q -p amazing-html --example html23_probe --locked -- crates/amazing-html/evidence/html23 > new.tsv
cargo run -q -p amazing-html --example html22_probe --locked -- crates/amazing-html/evidence/html22 > retained22.tsv
cargo run -q -p amazing-html --example html22_probe --locked -- crates/amazing-html/evidence/html21 --retained > retained21.tsv
```

| Source | Cases | Before pass/fail/unsupported/crash | After pass/fail/unsupported/crash |
| --- | ---: | ---: | ---: |
| `entities01.dat` | 75 | 73/2/0/0 | 75/0/0/0 |
| `entities02.dat` | 26 | 20/6/0/0 | 26/0/0/0 |
| `processing-instructions.dat` | 124 | 121/0/3/0 | 121/0/3/0 |
| `doctype01.dat` | 37 | 37/0/0/0 | 37/0/0/0 |
| `scriptdata01.dat` | 26 | 26/0/0/0 | 26/0/0/0 |
| `tests18.dat` | 36 | 32/2/2/0 | 32/2/2/0 |
| `webkit02.dat` | 49 | 38/9/2/0 | 38/9/2/0 |
| **Total** | **373** | **347/19/7/0** | **355/11/7/0** |

Two byte-identical before rounds have SHA-256 `d4db92af2d38cb1caa0abb947936d3c9015515defe0da4eacd6a9199b17be9d6`; two byte-identical after rounds have `1618170b90739bffca2e7bff42656816228a6657de60864abbc568f1ac379e00`. `new-transitions.tsv` records every case and raw FNV64 output hash: 8 fail-to-pass, 347 retained passes, 11 retained failures, 7 retained unsupported, zero losses. The eight gains add named `not`, `notin`, `pound`, `prod`, and legacy `AElig` forms with longest-match and attribute-context rules. Focused tokenizer tests cover text and attributes.

The 501-case HTML-21 retention selection is unchanged at **489 pass / 11 fail / 0 unsupported / 1 crash** in all four runs (SHA-256 `22ba9a5f7dafc0ccedd26f1badd67a42ecd64d8ef4ce29f23b9db9f87bf036ed`). The legacy-keygen panic remains a crash. The 389-case HTML-22 retention selection moves from **302/11/76/0** (SHA-256 `0691789d6f191bff854f8a28570ad7776f57c858081e4d83feb63b729b5dc1b6`) to **303/11/75/0** (SHA-256 `d028f5d18a6fa74976bd46b2acf2bde13424c86faa07654cb09cbefd4a6e7812`): one `head` fragment becomes pass, with all 302 prior passes retained and all other raw hashes unchanged. `html21-transitions.tsv` and `html22-transitions.tsv` record every row.

The dom-bindings blocker is repaired in the same owned crate: `parse_fragment` accepts ordinary HTML contexts such as `span`, uses the context tokenizer state for RCDATA, RAWTEXT, script data and plaintext, and uses the context as the final node when resetting insertion mode. The API uses a name string, so it does not convey foreign namespace or ancestor/form context. Focused tests cover ordinary elements, the additional RAWTEXT elements, `head`'s in-body default, and a table section closing back to its virtual table context.

`source-hashes.json` audits the fixed-base source, corpus fixtures, probe and root lockfile before and after the Windows gate. `windows-gate.ps1` contains the exact commands. `windows/` preserves two raw Windows rounds as deterministic gzip files for the new and both retention selections. PowerShell emits a UTF-8 BOM and Windows newlines; parsed rows, classifications, and every raw FNV64 hash match the Linux after rounds.

The final fixed-base win-002 gate passed package nextest **160/160**, separate package doctest **1/1**, workspace nextest **2,109/2,109 with 5 skipped**, and separate workspace doctests **2/2**. All 47 source/fixture/lock hashes matched before and after; Cargo.lock SHA-256 stayed `859b976b8eb930ccb3496d2d2e7c5732126f99d17a2e0fd474719a3ae7c592a8`. Each Windows selection ran twice with byte-identical raw outputs, and all parsed rows match both Linux after rounds. An earlier Windows attempt before the final fragment tests stopped on `amazing-process::split::browser_crash_reaps_renderer_at_each_transfer_phase`; `windows-attempt1.log` preserves it. The final fixed-source run passed that workspace test and the entire gate. Full output is in `windows-final-gate.log`.

The uncompressed Windows TSV SHA-256s (both rounds per selection) are `ac107a95b186884546b9eb2c79996cbe2a6c2d07bbd4c55c92da4bee4d36802d` for the new tranche, `d4208ea36b966fe0c1ee8b8cc6685f1c9a5332e477266a166516945a1bb9eacd` for HTML-21, and `2db8b8f5b5f8a4506944695c209bfbccf418e4d695757383ed616bf43ba7e2ab` for HTML-22.
