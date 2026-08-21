# AGENTS.md

## Work location policy

**All development, git history, builds, and tests live on the Windows build
server. Do not build or test on macOS.**

- Host: `server@win-001`
- SSH key: `~/.ssh/win001-build-key` (filename is missing an "f" — correct as-is)
- Working tree: `C:\Users\server\src\ferric`
- Default SSH shell is cmd.exe — avoid quote-wrapped paths; prefer PowerShell
  `-EncodedCommand` for anything non-trivial.
- File authoring workflow: stage files locally in
  `/var/folders/.../T/opencode/ferric`, then `scp -r` into the working tree,
  or write directly via ssh when trivial.

### Build server environment (already provisioned)

- cargo/rustc 1.97.1, git 2.55, VS 2022 Build Tools
- Python 3.12 + LLVM/libclang installed (not needed by Ferric itself; std-only)
- Env vars set at user level: `LIBCLANG_PATH`, `PYTHON3`

### Commands

```
ssh -i ~/.ssh/win001-build-key -o BatchMode=yes server@win-001 "cd /d C:\Users\server\src\ferric && cargo test --workspace"
sh scripts/ci.sh   # same gate, runnable anywhere cargo exists
```

## Project notes

- Ferric: from-scratch std-only browser engine. Zero external crates in
  engine cores (`ferric-html/css/dom/layout/js/net/paint`). See
  ARCHITECTURE.md for locked decisions before writing code.
- Phased plan with hard exit criteria — never auto-proceed to the next phase;
  report and wait for confirmation.
- Conformance suites (WPT/Test262) are tracked from Phase 3 / Phase 6 onward;
  always report measured numbers.
