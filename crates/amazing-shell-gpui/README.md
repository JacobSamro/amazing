# Amazing GPUI shell

The end-user desktop shell for Amazing, built with Zed's GPUI and isolated
from the std-only engine cores. The pinned GPUI revision provides the native
Windows platform and DirectX renderer.

The current shell slice implements the user-directed visual reference and the
first real engine viewport:

- custom Windows titlebar controls;
- pinned and color-grouped tabs, with a collapsible Specs group;
- back/forward/reload controls and an address surface;
- focused address-bar editing with select-on-click, Backspace, Escape, and
  Enter-to-navigate behavior;
- Back, Forward, and Reload controls backed by branch-aware in-memory history;
- sidebar-tab mode, bookmarks row, profile and utility controls;
- a live `amazing-layout` → `amazing-paint` RGBA8 page surface, converted to
  GPUI's native BGRA image without crossing the engine-core boundary;
- response-driven HTML for `file://` and plain `http://` navigation, including
  inline `<style>` collection and explicit in-viewport fetch errors;
- content-height painting up to the engine safety cap and native GPUI wheel
  scrolling while browser chrome stays fixed;
- a native media tab that encodes H.264, decodes I420, converts to RGBA, and
  presents the resulting 320×180 frame through GPUI;
- six statefully encoded/decoded frames advanced by a detached 500 ms GPUI
  timer without changing the active tab or browser chrome;
- `--secure-video` launches the root DTLS-SRTP driver, validates framed RGBA on
  a background localhost reader, and replaces the synthetic loop with the six
  actual secure-session frames;
- real TrueType glyph rasterization using the Windows UI font;
- responsive 1080×760 startup bounds for the win-001 RDP desktop.

![Amazing GPUI shell on win-001](../../docs/screenshots/gpui-shell-win001.png)

![Amazing GPUI grouped sidebar on win-001](../../docs/screenshots/gpui-shell-sidebar-win001.png)

![Amazing GPUI fetched document on win-001](../../docs/screenshots/gpui-file-navigation-win001.png)

![Amazing GPUI scrolled engine surface on win-001](../../docs/screenshots/gpui-scroll-navigation-win001.png)

![Amazing GPUI history Back navigation on win-001](../../docs/screenshots/gpui-history-back-win001.png)

![Amazing GPUI decoded video presentation on win-001](../../docs/screenshots/gpui-video-presentation-win001.png)

Animated proof: [frame A](../../docs/screenshots/gpui-video-frame-a-win001.png)
and [frame B](../../docs/screenshots/gpui-video-frame-b-win001.png).

Direct secure proof: [frame A](../../docs/screenshots/gpui-secure-video-win001.png)
and [frame B](../../docs/screenshots/gpui-secure-video-b-win001.png).

Build and launch on win-001:

```powershell
cargo build --manifest-path crates\amazing-shell-gpui\Cargo.toml --target-dir target --release --locked
target\release\amazing-shell.exe
# Start directly in the grouped sidebar layout
target\release\amazing-shell.exe --sidebar
# Start directly on the decoded-video media tab
target\release\amazing-shell.exe --video
# Launch the real secure two-process session and display its decoded frames
target\release\amazing-shell.exe --secure-video
```

The shell now proves the full local presentation path from address input through
Amazing's file/HTTP response layer, DOM and inline CSS collection, layout, and
software paint into a GPUI image. HTTPS/TLS, external stylesheets and media,
full text editing/IME behavior, history persistence, and a production tab
lifecycle remain the next UI integration steps.
