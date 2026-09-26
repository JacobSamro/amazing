# RC-21 — responsive/lazy image loading and scroll-triggered fetch

This directory holds the reproduction for the RC-21 pinned selections and the
results of each sweep. Windows-only render/reftest work runs on win-001 (the
WPT checkout and fonts live there); crate/workspace gates run through
`scripts/win-run.sh` (win-002, falling back to win-001 when win-002 is down).

## Frozen selections

Both manifests are committed beside the paint fixtures and pin WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`:

- render reftests: `crates/amazing-paint/tests/fixtures/rc21-wpt-render.json`
  (226 entries; `rel=match`/`rel=mismatch` HTML reftests under
  `html/semantics/embedded-content/the-img-element` and `css/css-images`).
- testharness: `crates/amazing-paint/tests/fixtures/rc21-wpt-harness.json`
  (117 `img`/`srcset`/`sizes`/`picture`/lazy/`decode()` files).

## Sweeps

```
# from the Windows lane tree root with CARGO_TARGET_DIR set
powershell -NoProfile -ExecutionPolicy Bypass -File tests\wpt-replaced-content\rc21\run-render-sweep.ps1 `
  -WptRoot C:\Users\server\target\layout\wpt `
  -ExecutablePath C:\Users\server\target\replaced-content\debug\amazing.exe

powershell -NoProfile -ExecutionPolicy Bypass -File tests\wpt-replaced-content\rc21\run-harness-sweep.ps1 `
  -WptRoot C:\Users\server\src\wpt `
  -ExecutablePath C:\Users\server\target\replaced-content\debug\amazing.exe
```

`run-render-sweep.ps1` wraps the repository `scripts/run-render-wpt.ps1`; the
WPT root there is a detached marker checkout (no git metadata), which the render
runner reads directly. `run-harness-sweep.ps1` loops `amazing wpt-harness`,
whose byte-pinning verifies every executed resource against the revision with
`git show`, so its WPT root must be a real clone containing the pinned commit.

## Recorded RC-21 baseline

Measured before the latest root-scroll and replaced-element commits:

- render 226: **185 pass / 6 mismatch / 9 unsupported / 26 crash**, no timeout.
- harness 117: baseline **4 pass / 60 fail / 39 timeout / 12 error / 2
  unsupported**; candidate **4 pass / 58 fail / 45 timeout / 8 error / 2
  unsupported** — no lost passes.

The current sweep runs on the merged dev base (`2a508101`), so any movement is
base drift, not all of it RC-21. Counts are recorded as current-HEAD candidate
numbers, not conformance gains.
