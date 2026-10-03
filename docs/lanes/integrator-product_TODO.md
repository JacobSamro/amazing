# Product runtime adoption

## SHELL-RUNTIME: use the gated document runtime in GPUI

The GPUI shell currently creates `ShellDomHost` and a plain `Interpreter` in
`engine_viewport::PageRuntime::new`, executes collected script strings, and pumps
legacy timers. The CLI/WPT driver uses `DriverScheduler` and installs
`RuntimeBindingHost`, including WebIDL wrappers, iframe realms, ordered modules,
document writing, scheduling and the new CSSOM adapter. Passing the driver corpus
therefore does not prove that these features are installed in the product.

After the CSSOM ticket, expose a shared, document-level runtime interface and
adopt it in the shell. Keep the CLI/WPT path on the same implementation. The
interface should own the DOM, bindings, interpreter and event loop together,
provide page-script/module execution and event dispatch, and expose a bounded
DOM borrow for style/layout snapshots. Avoid leaking each internal adapter into
the shell. Choose an Amazing library target or a small driver crate after mapping
the existing module dependencies; do not copy the entire driver into GPUI.

Preserve the shell's real BrowserSession resource policy, cookies, history,
navigation, viewport, media/image states, click default prevention and tab
isolation. Execute scripts with actual source URLs and parser insertion context.
Install bindings in every genuine browsing-context realm. Preserve the retained
presenter and idle/property-only frame paths.

Gate on Boat: workspace nextest and doctests, shell's complete test set, pinned
DOM/modules/CSSOM areas through the shared runtime, and the whole 50-page shell
capture selection before/after. Compare exact assertions and investigate every
lost pass. Capture actual Amazing windows with executable hash, source commit,
fixture URL and viewport, and inspect the images. A desktop capture is not proof.

## Current visual defect

The exact pushed `f120de60` news-home window runs but renders page glyphs as boxes;
chrome text is readable. Its GPUI system-font installer only probes Windows font
paths. Root commits `938215f9` and `1548855d` share the engine font loader, install
Linux system faces, and share the page resource cache across font aliases.
All 93 GPUI tests pass on Boat. The actual before/after Amazing windows were
captured and inspected; the corrected window renders readable page text.
Durable images and executable/source/viewport metadata are under
`crates/amazing-shell-gpui/evidence/linux-font-discovery/`.

## Other routed product gaps

- HTML52 owns actual Window.frames/length/indexed child/parent/top exposure;
  captured creator/container fallback base URLs follow it.
- The font driver resolves actual first-font metrics before inheritance, with
  registered-font tests and zero-loss pinned font reftests. GPUI adoption of the
  same metric boundary belongs with its shared runtime/render integration.
- Layout44 consumes physical XY transform origins against the actual reference
  box. Projective 4x4 paint is absent and needs a separate whole area; parsed Z or
  perspective inputs are not evidence of physical 3D projection.
- CSS42 border-image grammar is ready; actual used geometry and painter adoption
  is queued after the origin consumer, with a precise ownership proposal first.
