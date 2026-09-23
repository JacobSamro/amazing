# IPC-6 Windows AppContainer and network child evidence

Base: `c330766` (the network-contract readiness fix). Final gate: win-002,
`scripts/win-run.sh ipc`, with this lane's uncommitted IPC-6 changes synced into
its isolated tree. The process crate had 21/21 tests passing and 0 doctests.
The locked workspace nextest run completed successfully (1,961 tests selected);
the locked workspace doctest run completed successfully (2 doctests).

The renderer launch creates a per-launch AppContainer profile, copies the child
executable into its profile folder, starts the child suspended with only its
stdio handles inherited, assigns a one-process kill-on-close job before resume,
and retains the child process handle for browser-side shared-frame duplication.
The child verifies its AppContainer token before acknowledging bootstrap.

The Windows deny fixture verifies that the renderer cannot open a private user
file, connect to loopback, create another child, or open the browser process.
The shared-frame fixture compares sandboxed renderer pixels against a direct
render. Its initial failure was in the test: the browser event receiver was
dropped before `navigate_shared`, then the test called `acquire_frame` before
`receive_frame`, which already acquires and acknowledges. Keeping the receiver
and using the normal receive path made the isolated Windows test pass.

The network child demo dispatches `--type=network` through the same entry point
intended for the installed driver. The six-resource fixture checks document,
CSS, image, font, script, and JSON request status, URL, headers, and body against
direct Fetch. Driver wiring remains with integrator-assist under ADR 0009; the
ordered adoption handoff is in `.agents/ipc/questions.md` for routing.

Linux sandbox deny and frame parity remain ignored on Venus because host
AppArmor rejects unprivileged user namespace UID/GID mapping. The launcher
fails closed there; a narrow host profile is still awaiting the host decision.
