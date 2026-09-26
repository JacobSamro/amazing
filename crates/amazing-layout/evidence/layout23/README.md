# LAYOUT-23 — positioned axis constraints

Baseline engine749555ca, identical to final LAYOUT-22 binary labeled
40c722aa-layout22-candidate4 (source audit in layout22/final-after/provenance.txt).
Pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293; Linux800×600.
Complete static css-position and CSS2/positioning directories:645 cases,
126 script-dependent cases explicitly excluded. Before:226 pass,410 mismatch,
9 unsupported;zero errors,timeouts,crashes. The frozen manifest retains its
original mistaken LAYOUT-22 selection label so its measured SHA stays valid.

Scope: containing-block direction, static edges, auto sizing, min/max sizes,
automatic margins and replaced intrinsic preservation. Inline fragmentation,
multicol and sticky scrolling remain visible limitations.
Retain full writing1112,ruby118,tables386 and retained449; the pre-existing
LAYOUT-22 CSS th loss remains held separately and is not erased by this baseline.

Rules: [CSS2.2 used dimensions](https://www.w3.org/TR/CSS22/visudet.html)
sections10.3.7–8 and10.6.4–5, and
[relative positioning](https://www.w3.org/TR/CSS22/visuren.html#relative-positioning).
