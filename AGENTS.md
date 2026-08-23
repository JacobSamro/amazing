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
- Amazing itself is std-only; no extra deps required to build.
- Env vars set at user level (left over from other projects): `LIBCLANG_PATH`,
  `PYTHON3`.

## Project notes

- Amazing: from-scratch std-only browser engine. Zero external crates in engine
  cores (`amazing-html/css/dom/layout/js/net/paint`). Read ARCHITECTURE.md for
  locked decisions before writing code.
- Phased plan with hard exit criteria — never auto-proceed to the next phase;
  report per-phase results and wait for confirmation.
- Conformance suites: WPT tracked from Phase 3 onward, Test262 from Phase 6;
  always report measured numbers, never estimates.


Keep files under 4000 Lines per file. For code health
