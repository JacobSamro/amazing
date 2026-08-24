# GPUI owns production presentation

Amazing uses GPUI as its sole production desktop presentation adapter because
one high-performance Rust UI layer must compose shell chrome, interaction and
accessibility overlays, document pixels, and in-document media consistently.
The standards engine stays provider-neutral and exports owned presentation
data; Chromium/WebView embedding and a parallel production Win32 UI are
rejected because either would split behavior, proof, and ownership across two
rendering stacks.

## Consequences

The Win32 shell remains diagnostic only. Native media tabs and codec probes do
not satisfy end-user playback: `<audio>` and `<video>` must be integrated into
the document lifecycle and browsing frame, then presented and captured through
GPUI like every other visible surface.
