# IA-HTML2 fixed-base driver evidence

This is a driver-facing synthetic raw-byte selection, not upstream WPT and not a
browser-wide pass rate. The lane gate is pinned to dev base `2021707`; the
co-integrator must gate the eventual merged head separately. The BEFORE driver
is revision `2021707`, the AFTER driver is implementation revision
`43c83d40bf07b2468cc5ec9c2aee5c722ded49ff`. The reports record both
executable SHA-256 values, per-case input SHA-256, exact HTTP Content-Type
strings, exit code, stdout/stderr SHA-256, and actual output.

Run on win-001 in `C:/Users/server/src/wt/integrator-assist` with
`tests/wpt-integrator-assist/collect-html2-driver.ps1`, frozen executables under
`target/diagnostics/ia-html2/`, and distinct `before-1`, `before-2`, `after-1`,
`after-2` output directories. Each repeated stage has exactly identical case
objects and projection hash. The original BEFORE fixture source accidentally
used non-ASCII PowerShell literals that Windows PowerShell read incorrectly;
those discarded initial reports are excluded. All committed BEFORE reports use
explicit code points, matching the AFTER collector source.

`comparison.json` records each of the 12 matching input hashes and both result
hashes. Seven HTML outputs change as expected. The five unchanged controls are
malformed UTF-8 replacement, local valid XML, local malformed XML, local XML
undecoded byte, and HTTP XML MIME dispatch. There are zero changed exit
classifications. No selection case exercises Unsupported; current-source
`xml_dispatch` tests separately cover the external-DTD Unsupported exit 3.
XML still enters `parse_xml` through caller-decoded Unicode, so this gate does
not prove UTF-16 or declared-encoding XML decoding.

`head-source-manifest.json` lists SHA-256 of all 944 tracked HEAD paths under
`crates/`, `scripts/`, `tests/` and the root Cargo files. The Windows lane tree
matched all 944 after the workspace gates; `source-verification.json` records
that result. The nine changed source/harness files and frozen AFTER executable
also matched before the proof. `gate-summary.json` names the exact Windows
commands, counts, log hashes, target directory, and primary shared build-slot
wrapper. The scoped binary package has no library target, so its separate
`--doc` invocation reports no doctests; workspace doctests passed 2/2.
