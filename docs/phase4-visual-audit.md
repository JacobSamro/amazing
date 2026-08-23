# Phase 4 visual audit

Audit date: 2026-08-23. Fixture: `examples/layout.html`. Viewport: 800×804 CSS
pixels.

The complete capture and comparison ran on `win-001`, per `AGENTS.md`.
Amazing rendered with:

```bat
target\release\amazing.exe render examples\layout.html target\audit\phase4.bmp 800
```

The resulting BMP and PPM were both 800×804 (1,929,654 and 1,929,615 bytes).

The server's installed Microsoft Edge rendered an independent reference from
the same local fixture:

```bat
"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" ^
  --headless --disable-gpu --hide-scrollbars --window-size=800,804 ^
  --screenshot=target\audit\phase4-edge-reference.png ^
  file:///C:/Users/server/src/amazing/examples/layout.html
```

A server-side PowerShell/System.Drawing script converted the BMP, generated
the absolute diff and side-by-side image, and computed normalized RGB RMSE over
the two identical 800×804 buffers:

```bat
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\phase4-visual-audit.ps1
```

Measured normalized RMSE: **0.460741**. Artifacts were copied to the editing
tree only after this authoritative Windows gate completed.

Artifacts:

- `docs/screenshots/phase4-amazing.png`
- `docs/screenshots/phase4-edge-reference.png`
- `docs/screenshots/phase4-absolute-diff.png`
- `docs/screenshots/phase4-side-by-side.png` (Amazing left, Edge right)

This is a visual comparison gate, not a pixel-parity claim. The centered card
and broad block flow align visibly. Large pixel differences are expected and
preserved as evidence: the audited Amazing render uses fallback bar glyphs when
no TTF is supplied, implements a bounded CSS/layout subset, and assumes lossy
UTF-8 while Edge applies HTML encoding rules.
