# Resume notes — 2026-09-21

Where the fleet stopped, and what to do first. Read alongside
`docs/INTEGRATOR.md` (role, merge protocol, grant ledger, traps).

## Blocker to clear first

**win-001 refuses all new SSH connections.** Diagnosed, not fixed:

```
ping                           2/2 packets, 0% loss
TCP port 22                    OPEN
banner + SSH2_MSG_KEXINIT      exchanged (OpenSSH_for_Windows_9.5)
then                           reset, before authentication
```

Existing sessions stayed healthy while new ones failed, and the reset lands
exactly where sshd would spawn its per-connection child. That points at the
server being unable to create new processes — memory, handles, or desktop heap —
rather than a config limit. Ten lanes running concurrent `cargo`/`rustc` is the
plausible cause. `IPQoS=none`, a pinned KEX, and `IdentitiesOnly` all fail
identically, so it is not a client negotiation problem.

**Correction, 2026-09-22.** The original diagnosis below guessed at process-
creation or memory exhaustion. The evidence says otherwise: the box was never
rebooted (194.9 h uptime), had ~49 GB RAM free, and sshd recovered on its own
once lanes stopped retrying. That matches sshd's default `MaxStartups
10:30:100`, which randomly drops connections before authentication under
concurrent handshakes. Fixed by multiplexing all ssh/scp over one master and
raising the limits to `MaxSessions 50` / `MaxStartups 50:30:100` -- see AGENTS.md.

On the box, via RDP or console:

1. `Restart-Service sshd`
2. If that fails, check memory and kill orphaned `rustc`/`cargo` processes from
   killed builds — js-runtime flagged one unverified orphaned Test262 repeat
   child, and there may be more.
3. Worst case reboot. Nothing there is precious: git lives on the Mac and the
   lane trees rebuild from `scripts/refresh-lane-trees.sh --all`.

Then verify with `ssh … server@win-001 'echo ALIVE'` and broadcast restoration.

## Fleet

Ten agents in tmux, all told to checkpoint and stop. Six engine lanes are the
integrator's; `integrator-assist`, `html` and `replaced-content` belong to the
co-integrator working from `../amazing-integrator2`.

| Lane | Ticket | State at stop |
|------|--------|---------------|
| js-lang | JS-4 | briefed, not started — **critical path**, two lanes blocked |
| js-runtime | JSRT-1 | checkpointed `8a04aed`; VM design + `BindingHost` draft in report |
| dom-bindings | DOM-5 | triaging `dom/` clusters; blocked on `BindingHost` + driver adoption |
| layout | LAYOUT-4 | recovery gates; report collection pending access |
| css | CSS-5 | restarted fresh; presentational hints incl. `cellspacing` |
| net | NET-8 | WebSocket design; randomness seam approved |
| replaced-content | RC-4 | committed `6c5bda9`, clean tree, report written |
| html | HTML-1 | parser merged; waiting on IA-XML dispatch |
| integrator-assist | IA-XML | IA-1 staged `ia1-rebased-aa9cb9b` |

## First actions tomorrow

1. Fix win-001, verify, `scripts/refresh-lane-trees.sh --all`, broadcast.
2. Re-arm the monitor: `python3 scripts/fleet-monitor.py 45` under the Monitor
   tool. Add a **memory check** — CPU and disk were watched, RAM was not, and
   RAM is what this outage implicates.
3. Re-run any gate whose last measurement predates the outage. Lanes were told
   not to present pre-outage runs as current; hold them to it.
4. Unblock the DOM chain: js-lang JS-4 scheduling seam → js-runtime
   `BindingHost` → integrator-assist driver adoption → DOM-5 becomes
   WPT-visible. This is what moves `dom/` off 2/627.

## Open decisions for the user

None outstanding. The Public Suffix List and brotli exceptions were approved and
shipped (`ARCHITECTURE.md` 6c/6d, ADR 0002). Expect the next one to be Unicode
property tables or a JIT question from the js-runtime lane.

## Measured state

26 tickets merged. Workspace ~1261 tests passing on nextest plus doctests.

| Suite | Result |
|-------|--------|
| Test262 | **10,314 / 53,872** files (102,918 variants), conformant runner, twice, zero diffs |
| WPT `dom/` | 2 pass / 241 fail / 30 timeout / 221 error / 133 unsupported |
| WPT `url/` | 864 / 896 |
| Broad geometry | 34 / 61 |
| Broad flex/grid | 61 / 200 |
| Tables | 8 / 12 |
| Floats | 6 / 8 |
| Position | 12 / 12 |
| CSS Images | 12 / 12 |
| Cascade corpus | 35 pass / 24 mismatch / 13 unsupported |

Three published numbers were corrected today, two of them mine: the Test262
"196/196" claim (a 641-file sparse checkout against a real 53,872), the geometry
"38/23" baseline (does not reproduce; 32/29 measured), and a merge message that
claimed work sitting on an unmerged sibling branch. All three were caught by
lanes re-running evidence rather than trusting it. Protect that.
