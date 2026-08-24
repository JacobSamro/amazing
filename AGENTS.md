# AGENTS.md

## Work location policy

**Never build or test on the local macOS machine.** Edit locally, sync to the
Windows build server, and run every build/test there.

### Local editing tree

- `/Users/jMac/Projects/OpenSource/amazing`

### Build server

- Host: `server@win-001`
- SSH key: `~/.ssh/win001-build-key` (filename is missing an "f" — correct as-is)
- Working tree: `C:\Users\server\src\amazing`
- Default SSH shell is cmd.exe — avoid quote-wrapped paths; prefer PowerShell
  `-EncodedCommand` for anything non-trivial.

### Sync workflow

1. Edit locally under `/Users/jMac/Projects/OpenSource/amazing`.
2. Sync changed files up:
   ```
   scp -q -r -i ~/.ssh/win001-build-key <paths> 'server@win-001:C:/Users/server/src/amazing/'
   ```
   (`rsync` is NOT available on the Windows side — use `scp`.)
3. Commit on win-001 (git lives on the server):
   ```
   git -C C:\Users\server\src\amazing add -A
   git -C C:\Users\server\src\amazing commit -m "<message>"
   ```
   (server-side git identity is configured via `-c user.name/user.email` on
   first commit; reuse it)
4. Build/test on win-001:
   ```
   ssh -i ~/.ssh/win001-build-key -o BatchMode=yes server@win-001 "cd /d C:\Users\server\src\amazing && cargo test --workspace --locked"
   ```

### Build server environment (already provisioned)

- cargo/rustc 1.97.1, git 2.55, VS 2022 Build Tools
- VS 2022 CMake at `Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe`;
  its absolute path is stored in the user-level `CMAKE` variable for bundled
  libopus builds.
- vcpkg `2026.07.29` at `C:\Users\server\src\vcpkg` (commit
  `9e593bb18ea69cc5095e012465dcd675a822ed0d`) with
  `libvpx:x64-windows-static-md` 1.16.0#2. User-level `VPX_LIB_DIR`,
  `VPX_INCLUDE_DIR`, `VPX_VERSION`, `VPX_STATIC`, `PKG_CONFIG`, and
  `PKG_CONFIG_PATH` point at that installation.
- Amazing itself is std-only; no extra deps required to build.
- Env vars set at user level (left over from other projects): `LIBCLANG_PATH`,
  `PYTHON3`.

### Screenshot-based UI proof

For GPUI/browser-shell visual checks, capture the actual Amazing window on
`win-001`, copy the PNG to the Mac, and inspect that file. Do not infer UI
state from build success or a full-desktop screenshot.

1. Keep the RDP user session active and identify the `amazing-shell.exe`
   process, executable path, PID, session id, and build timestamp.
2. Run `scripts\capture-gpui-shell.ps1` in that interactive RDP session. An
   ordinary SSH process cannot see the RDP window station, so invoke the script
   through a temporary interactive-only scheduled task running as `server`.
   Always unregister the temporary task in `finally`.
3. Write captures below `target\diagnostics\` (ignored), then copy them back:
   ```
   scp -q -i ~/.ssh/win001-build-key \
     server@win-001:C:/Users/server/src/amazing/target/diagnostics/<capture>.png \
     /private/tmp/<capture>.png
   ```
4. Inspect the copied PNG with the image-viewing tool. `PrintWindow` is the
   preferred capture path because it records the target window even when
   Terminal or another app obscures it.

Treat a screenshot as visual regression evidence only when its executable,
page/fixture, viewport, and capture path are recorded. It complements but does
not replace pinned WPT measurements. Remove temporary scheduled tasks and
throwaway capture helpers after the proof is collected.

## Project notes

- Amazing: from-scratch std-only browser engine. Zero external crates in engine
  cores (`amazing-html/css/dom/layout/js/net/paint`). Read ARCHITECTURE.md for
  locked decisions before writing code.
- Phased plan with hard exit criteria — never auto-proceed to the next phase;
  report per-phase results and wait for confirmation.
- Conformance suites: WPT tracked from Phase 3 onward, Test262 from Phase 6;
  always report measured numbers, never estimates.


Keep files under 4000 Lines per file. For code health

Generated lockfile exception: `crates/amazing-shell-gpui/Cargo.lock` may exceed
4,000 lines because the isolated GPUI platform graph is machine-generated and
must remain in one Cargo-readable lockfile for `--locked` reproducibility. This
exception does not apply to authored source, tests, scripts, or documentation.
