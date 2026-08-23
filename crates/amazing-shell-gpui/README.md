# Amazing GPUI shell

The end-user desktop shell for Amazing, built with Zed's GPUI and isolated
from the std-only engine cores. The pinned GPUI revision provides the native
Windows platform and DirectX renderer.

The first shell slice implements the user-directed visual reference:

- custom Windows titlebar controls;
- pinned and color-grouped tabs, with a collapsible Specs group;
- back/forward/reload controls and an address surface;
- sidebar-tab mode, bookmarks row, profile and utility controls;
- an editorial status page matching the reference's Segoe/Georgia typography;
- responsive 1080×760 startup bounds for the win-001 RDP desktop.

![Amazing GPUI shell on win-001](../../docs/screenshots/gpui-shell-win001.png)

![Amazing GPUI grouped sidebar on win-001](../../docs/screenshots/gpui-shell-sidebar-win001.png)

Build and launch on win-001:

```powershell
cargo build --manifest-path crates\amazing-shell-gpui\Cargo.toml --target-dir target --release --locked
target\release\amazing-shell.exe
# Start directly in the grouped sidebar layout
target\release\amazing-shell.exe --sidebar
```

The current slice owns shell state and visual interactions. Connecting the
content viewport to Amazing's DOM/layout/paint pipeline, editable address
input, history persistence, and production tab lifecycle are the next UI
integration steps.
