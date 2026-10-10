# Font catalogue gate (resume-font, 2026-10-11)

Base `ee2786a463f8e5d5975b6c58f3c0bdc5cee44f14` (origin/dev). Candidate source commit
`8f75b9fad5ad50cfb2424fd8594af027ed62eb44` (generic-family selection + installed-face catalogue
reuse; the squashed ready commit adds only these compact evidence files on top of that tree).
Run: `scripts/linux-render-wpt.sh resume-font css/css-fonts --workspace --baseline-ref <base>` on Boat.

- Workspace nextest: 5519 passed / 0 failed (base) -> 5535 passed / 0 failed (candidate), 17 skipped each.
  The CDP tests that timed out in the excluded bcedcb953 gate pass.
- Doctests: 39 / 0 failed, both sides.
- css/css-fonts (32 cases): 26 pass / 6 mismatch -> 29 pass / 3 mismatch. Gained: standard-font-family-2, -5, -17. Lost: none.
- Raw output stays on the Boat box under `amazing/out/resume-font/render-gate/catalogue1-{before,after}`; sha256 values are in the gate.summary.json files.
