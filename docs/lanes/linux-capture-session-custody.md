# Linux capture session custody

Source-only tooling repair on recorded `fd87ab4c5467af81758ea59cff179863ffdde6f4`.
The original viewport candidate `91e488affa6733971abce45bef3c58fe0160817e` is unchanged.
Its first scale-2 launch ended with capture exit 3: the actual window existed,
but no measured engine viewport record existed. The authenticated 696-byte
shell log (SHA256 `1f0ac62b62ddc983bb35d63fd930a41a19c8dd57d58f651fecec922afdbe8da6`)
contains six lines, including an AccessKit worker D-Bus NoReply panic. Native
existing-log diagnostic `b36c8ad1126544b4ba3859aa58a9230a` and compact fetch both
exited 0. The panic's causal sufficiency for the missing viewport is UNKNOWN.
Original phase 3 and the unused clean GPUI rerun remain preserved.

The former adapter created Xvfb and the shell without owning their session bus.
AccessKit 0.21.1 (registry checksum
`b016ca8db0ea0ea2ceff29a9d6240391492d960716aa471967c00e8cc8cb197c`,
upstream `f40dfc01a0c0e76de535969f82fb35e19513737d`) unwraps its worker event-loop
result, explicitly prefers inherited AT_SPI_BUS_ADDRESS, and ignores sends into
a closed worker channel. Therefore this isolated worker panic does not prove a
fatal main-thread rendering failure. No dependency code is copied or changed.

The public capture adapter now transports an exact stdlib session custodian
alongside its existing proof helper, so an older fixed engine ref does not need
to contain the new helper. `dbus-run-session` owns a fresh actual bus for the
complete capture. Inherited session/starter/AT-SPI addresses are cleared solely
to select this new session's real accessibility activation. Accessibility is
not disabled. GUID, actual bus owner PID, and actual org.a11y.Bus owner/absence
are recorded before and after the child. An absent service remains explicit;
these facts cannot replace a measured viewport or certify a frame.

The capture performs ordered shell TERM/wait/KILL cleanup, then Xvfb cleanup,
then lets the custodian and bus terminate. Custodian signals reach the inner
custodian first so its capture child can clean up while the bus remains live.
A fresh receipt protects old proof files. Actual child failures remain failures;
a successful child with a replaced/disconnected bus cannot certify success.
Original window actions and viewport assertions are unchanged.

Eight grouped native Linux controls exercise a live actual bus, stale inherited
addresses, child exit status, receipt collision, actual session disconnection,
no-AT-SPI-service session, real AT-SPI activation/owner disconnection (requires
actual owner in the owned process group before any signal), and signal cleanup
on a live bus. The original 109 GPUI controls, 14 proof controls, and complete
scale-1/scale-2 product and resize actions remain required for qualification.
No native control, engine build, or window launch has run for this source yet.

References:
- https://dbus.freedesktop.org/doc/dbus-run-session.1.html
- https://github.com/AccessKit/accesskit/blob/f40dfc01a0c0e76de535969f82fb35e19513737d/platforms/unix/src/context.rs
- https://github.com/AccessKit/accesskit/blob/f40dfc01a0c0e76de535969f82fb35e19513737d/platforms/unix/src/adapter.rs
- https://github.com/AccessKit/accesskit/blob/f40dfc01a0c0e76de535969f82fb35e19513737d/platforms/unix/src/atspi/bus.rs

Source correction: any post-spawn diagnostic/publication failure now terminates
and reaps the live owned child before bus probes and terminal publication.
Actual child exit remains recorded separately from diagnostic exit 3. A
termination cached before spawn cancels startup; one arriving during Popen is
forwarded immediately after assignment. Three additive native controls invoke
the real run_inside with real child processes and a real bus: rename the actual
receipt parent after spawn (ENOENT on RUNNING publication), send SIGTERM after
handler installation before spawn, and send SIGTERM during the actual spawn
return boundary. They require live-bus child cleanup, reaping, correct exits,
and no child startup after cached termination. All original eight test bodies
are unchanged; eleven lifecycle groups now require Linux qualification.

First coherent native attempt (73e45, job fc9e604) passed all 109 GPUI tests
but failed one of the original 14 proof controls: its private assembly fixture
omitted the new real session helper. The helper is now staged alongside the
original files; exit-37 and all original assertions remain unchanged. Eleven
lifecycle controls and every window action were unstarted, not passed.

The capture function now runs in the actual owned Bash child, instead of a
subshell whose parent could terminate without executing its traps. One additive
native control executes the exact public function prefix and cleanup functions
with a real native child and Xvfb, then terminates the custodian: the child must
clean up on a live bus, both child processes must be reaped, and the bus must
remain live through cleanup. It makes no UI/viewport proof claim; the original
28 genuine browser action shots remain mandatory. Twelve lifecycle groups now
require full native qualification. Original first failure and raw hashes remain
preserved; there has been no subsequent run.
