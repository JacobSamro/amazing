# Live viewport density and one baked replay per frame

At density 2, the old baked fallback replayed the entire viewport for every tile. Native captures repeatedly reached frame construction without publishing a measured viewport before the unchanged capture deadline. The compositor now prepares the actual common command range once per dirty frame and copies the original pixels into each tile. The prepared surface ends with that frame; cached frames perform no replay, changed resources are observed on the next dirty frame, and mixed ranges keep the original dispatch.

This ticket includes the original viewport/DPR propagation through shell measurement, input, JS metrics, text, paint and compositor/process snapshots. Current selected-image natural-size handling is preserved. Actual Linux capture uses an owned D-Bus session with accessibility enabled; native PNG ownership is explicit and validated.

The recorded base is `c08149e33a4a49c9c9a4121396493729625dcd02`. Measured AFTER is `bd4c4e39495b0a40e06ecb6ddb6525dab61fccac`; the genuine unoptimized viewport/session BEFORE is `645f1b6fd08878e9a9f5534b2f0de817889b4a99` on that same base. The ready commit changes only documentation/evidence beyond measured AFTER.

Linux qualification:

- Workspace: 4,971 passed, 17 skipped; whole workspace doctests command exited 0. These ran on `3684b9c3`; its crates/Cargo are byte-identical to measured AFTER. Only three public capture scripts subsequently changed and received native qualification.
- GPUI: all original 109 tests, 17 proof controls and 12 session controls passed.
- Actual window actions: all 28 passed at scales 1 and 2. BEFORE had 14 passes, two genuine scale-2 measurement failures and 12 unstarted actions. All 14 original passing identities remain passing.
- Original 20 static WPT render directories: 2,214 selected cases, 1,327 passes before and after, zero lost passes and zero removed cases. Pin `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`; 800×600, density 1, jobs 4, timeout 20.0, limit 0.
- Seven new native groups assert exact pixel equality against existing tile replay and full painting, actual one-replay/zero-cached-replay counts, ranges, edge tiles, controls, resources and existing admission failures.

[Actual scale-2 dashboard window](product-smoke-scale2.png) and [actual scale-2 resize-suite window](viewport-resize-scale2.png) were fetched with hash/size/PID/source/window metadata and inspected. Both are 1280×800 target-window captures; measured content is CSS 614×256, density 2, raster 1228×512. All 28 original PNGs remain on the native host. The retained native executable and fixture paths are recorded in `compact.json`; the shell is runnable on Linux with its normal `--url` and `--size` arguments and `GPUI_X11_SCALE_FACTOR` density input. Use the supported Linux wrapper/session helper for new captures.

Compiler/test fixture failures and the two initial source-transfer timeouts remain explicit in `compact.json`. Each transfer's single clean attempt succeeded; no third unchanged attempt or raw recovery occurred. Raw results, logs and measured ELFs remain native; artifact/raw-inventory commitments are recorded. One completed BEFORE endpoint could not later supply individual hash metadata over SSH, so that absence is explicit rather than replaced with another binary.

The integrator's merged-current workspace and 16-shard Test262 gate remain required before publication. This is a viewport/per-frame replay qualification, not a complete-browser, Windows or weak-collector readiness claim.
