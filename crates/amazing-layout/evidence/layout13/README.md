# LAYOUT-13 evidence

Fixed base `9e736e2`, pinned WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, 127 generated-content, list, and flex pseudo-element cases. `generated-manifest.json` freezes the selection. `final-before.json` and `final-after.json` are the raw Windows render reports; `generated-summary.json` lists all 17 gains and 11 individually routed, primary-accepted exceptions. Passes moved 44/127 to 50/127. Fourteen page-script/DOM crashes occur on both sides. `source.json` records source, archive, manifest, and executable hashes.

The ten retained selections on the preceding CSS-13 base `f730427` moved 158/184 to 159/184, with one `display:contents` pseudo-element gain and zero losses; `retained-before/`, `retained-after/`, and `retained-summary.json` contain the raw reports. This retained sweep preceded the CSS counter/string fixes and the final marker display correction. The integrator rechecks retained zero losses at merge.

`win002-gate/gate-final.log` is the final Windows test gate on the source with the marker display correction: layout 251/251 nextest, workspace 1963/1963 nextest (5 skipped), layout 0/0 doctests, workspace 2/2 doctests. `pre-css-fixes/` retains the original CSS-13 fixed-base comparison that identified the upstream bugs.
