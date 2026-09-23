# LAYOUT-11 focused baseline

The fixed pre-CSS-11 source is lane commit `82d5574e2f38f8ccddfd6a336340ef513ee14467`
on `origin/dev` `8f2695d92b31962982bb9c389f4d3b4a784ba13a`. The archive was
expanded into the layout Windows build tree. `cargo build -p amazing --bin amazing
--locked` ran on win-001; the immutable renderer SHA256 was
`3c9e967b14c58c3e7757d741afb4d7f930e26f4db2177ac0789580f2e55dbde9`.
The pinned WPT revision was `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`,
800×600 at device scale 1. The zoom fixtures were copied from the CSS lane's
checkout of the same revision into the layout lane's WPT root.
The 292 tracked crate source files and Cargo manifests/lockfile in
`baseline-source.json` matched their win-001 copies, 292/292.

The five-case `focused-manifest.json` ran twice under a build slot, producing
`baseline-1.json` and `baseline-2.json`. Both report **0 pass, 5 mismatch**, with
identical per-case classifications, differing pixel counts and
actual/reference/diff SHA256 hashes:

| Case | Differing pixels |
| --- | ---: |
| `css-viewport__zoom__basic.html` | 30,000 |
| `css-viewport__zoom__inherited.html` | 30,000 |
| `css-cascade__all-prop-002.html` | 10,000 |
| `css-values__calc-border-radius-1.html` | 84 |
| `css-fonts__font-variant-ligatures-11.optional.html` | 424 |

This baseline predates CSS-11 and CSS-12. The latter must expose computed font
features before layout can measure the ligature case. The two cascade cases
have existing layout geometry and require the paint changes routed in
`.agents/layout/questions.md`.
