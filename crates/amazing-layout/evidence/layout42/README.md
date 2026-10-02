# LAYOUT42 evidence

Fixed source base 745f8faada37f63c3447cac50bfd5606a0a7c207, including unchanged CSS35 synthesis/CSS36 typed metric API. Native Linux build target /tmp/amazing-layout36-target and source /tmp/amazing-layout37-source, pooled Boat bx_shgjh66q. No local build.

Run iterate.sh for the whole layout crate and twelve full pinned static WPT directories. gate.sh restores six consumer files from the base for before render, restores candidate, gates workspace/doctests/harness, then compares all twelve directories to that before. New font_policy module is not referenced by baseline lib.rs. Zero lost passes required; renderer/source manifests included. Summary and compact maps recorded after gate.

The abstract fake-font tests distinguish all five explicit axes and check actual shaped glyph size without altering CSS-visible size. Actual provider ratios, FromFont pre-inheritance resolution, contextual math callback and driver wiring remain CSS41/root-owned; no guessed font ratios here.
