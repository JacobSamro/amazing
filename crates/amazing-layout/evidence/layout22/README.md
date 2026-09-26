# LAYOUT-22 — vertical inline placement and alignment (active)

Pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293, Linux800×600.
The full static css-writing-modes directory contains1112 reftests;28
script-dependent cases are explicitly excluded. Final before sourceec384d80
(LAYOUT-21 complete): **230 pass,749 mismatch,133 unsupported**, no errors,
timeouts or crashes. `before-final/report.json` is authoritative. Preliminary
`before/` used the earlier ruby candidate5 and is retained only for diagnosis.

The unsupported cases report XML entities requiring external DTD declarations;
no external DTD is loaded. Raw classifications remain in the denominator.
Filename clustering is only a first triage aid, not a claim of root cause.

Current area: line-relative start/end/left/right/center/interword justification,
parent line struts, vertical atomic advances and placement, and the open
LAYOUT-14 direction-upright/text-combine compression cases. Absolute-position
and orthogonal block geometry are separate larger clusters in this directory.
Whole-directory + retained ruby/table/union sweeps accompany each bulk cycle.
No result is claimed until measured and zero losses are established.

Specifications:
- https://www.w3.org/TR/css-writing-modes-4/ sections2,4,5,9.1
- https://www.w3.org/TR/css-text-3/ section6
