//! Minimal Windows GUI shell for the from-scratch engine.
//!
//! This module intentionally uses only the Win32 ABI and the standard
//! library. The FFI is isolated here; the engine crates remain safe and
//! dependency-free. The first shell has one top-level window, an address bar,
//! Go/Back/Forward controls, and a software-painted page surface.

#[cfg(not(windows))]
pub fn run(_: Option<&str>) -> std::process::ExitCode {
    eprintln!("amazing gui: this build has no native shell for the current OS");
    std::process::ExitCode::FAILURE
}

#[cfg(not(windows))]
pub fn play_wav_file(_: &str) -> std::process::ExitCode {
    eprintln!("amazing play-audio: this build has no native audio adapter");
    std::process::ExitCode::FAILURE
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::process::ExitCode;
    use std::cell::RefCell;
    use std::rc::Rc;

    use amazing_dom::{Dom, NodeIdx, NodeKind};
    use amazing_js::DomHost;
    use amazing_layout::{Flow, LayoutBox};
    use amazing_paint::Surface;

    use crate::{load_page_media, BrowserDomHost, PageMedia};

    type Handle = *mut c_void;
    type Hwnd = Handle;
    type Hinstance = Handle;
    type Hicon = Handle;
    type Hcursor = Handle;
    type Hbrush = Handle;
    type Hdc = Handle;
    type Hmenu = Handle;
    type Wparam = usize;
    type Lparam = isize;
    type Lresult = isize;
    type Uint = u32;

    const CS_HREDRAW: u32 = 0x0002;
    const CS_VREDRAW: u32 = 0x0001;
    const WS_OVERLAPPEDWINDOW: u32 = 0x00cf0000;
    const WS_CHILD: u32 = 0x40000000;
    const WS_VISIBLE: u32 = 0x10000000;
    const WS_BORDER: u32 = 0x00800000;
    const ES_AUTOHSCROLL: u32 = 0x0080;
    const ES_LEFT: u32 = 0x0000;
    const BS_PUSHBUTTON: u32 = 0x00000000;
    const SW_SHOW: i32 = 5;
    const WM_DESTROY: Uint = 0x0002;
    const WM_PAINT: Uint = 0x000f;
    const WM_SIZE: Uint = 0x0005;
    const WM_COMMAND: Uint = 0x0111;
    const WM_MOUSEWHEEL: Uint = 0x020a;
    const WM_KEYDOWN: Uint = 0x0100;
    const WM_LBUTTONUP: Uint = 0x0202;
    const WM_TIMER: Uint = 0x0113;
    const BN_CLICKED: u16 = 0;
    const GWLP_USERDATA: i32 = -21;
    const GWLP_WNDPROC: i32 = -4;
    const VK_RETURN: usize = 0x0d;
    const GUI_TIMER_ID: usize = 1;
    const DIB_RGB_COLORS: u32 = 0;
    const SND_NODEFAULT: u32 = 0x0002;
    const SND_FILENAME: u32 = 0x0002_0000;
    const ID_ADDRESS: usize = 101;
    const ID_GO: usize = 102;
    const ID_BACK: usize = 103;
    const ID_FORWARD: usize = 104;
    const ID_NEW_TAB: usize = 105;
    const ID_TAB_BASE: usize = 200;
    const CONTROL_HEIGHT: i32 = 58;

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: Uint,
        w_param: Wparam,
        l_param: Lparam,
        time: u32,
        point: Point,
    }

    #[repr(C)]
    struct PaintStruct {
        hdc: Hdc,
        erase: i32,
        paint: Rect,
        restore: i32,
        inc_update: i32,
        reserved: [u8; 32],
    }

    #[repr(C)]
    struct WndClassW {
        style: u32,
        wnd_proc: Option<unsafe extern "system" fn(Hwnd, Uint, Wparam, Lparam) -> Lresult>,
        cls_extra: i32,
        wnd_extra: i32,
        instance: Hinstance,
        icon: Hicon,
        cursor: Hcursor,
        background: Hbrush,
        menu_name: *const u16,
        class_name: *const u16,
    }

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        image_size: u32,
        x_pels_per_meter: i32,
        y_pels_per_meter: i32,
        clr_used: u32,
        clr_important: u32,
    }

    #[repr(C)]
    struct BitmapInfo {
        header: BitmapInfoHeader,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassW(class: *const WndClassW) -> u16;
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Hwnd,
            menu: Hmenu,
            instance: Hinstance,
            param: *mut c_void,
        ) -> Hwnd;
        fn DefWindowProcW(hwnd: Hwnd, message: Uint, w_param: Wparam, l_param: Lparam) -> Lresult;
        fn DispatchMessageW(message: *const Msg) -> Lresult;
        fn GetMessageW(message: *mut Msg, hwnd: Hwnd, min: Uint, max: Uint) -> i32;
        fn GetModuleHandleW(name: *const u16) -> Hinstance;
        fn GetConsoleWindow() -> Hwnd;
        fn GetParent(hwnd: Hwnd) -> Hwnd;
        fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
        fn InvalidateRect(hwnd: Hwnd, rect: *const Rect, erase: i32) -> i32;
        fn MoveWindow(hwnd: Hwnd, x: i32, y: i32, width: i32, height: i32, repaint: i32) -> i32;
        fn PostQuitMessage(code: i32);
        fn SetTimer(hwnd: Hwnd, id: usize, elapse: u32, callback: Handle) -> usize;
        fn KillTimer(hwnd: Hwnd, id: usize) -> i32;
        fn CallWindowProcW(previous: isize, hwnd: Hwnd, message: Uint, w_param: Wparam, l_param: Lparam) -> Lresult;
        fn SetWindowLongPtrW(hwnd: Hwnd, index: i32, value: isize) -> isize;
        fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
        fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
        fn TranslateMessage(message: *const Msg) -> i32;
        fn UpdateWindow(hwnd: Hwnd) -> i32;
        fn BeginPaint(hwnd: Hwnd, paint: *mut PaintStruct) -> Hdc;
        fn EndPaint(hwnd: Hwnd, paint: *const PaintStruct) -> i32;
        fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
        fn GetWindowTextW(hwnd: Hwnd, text: *mut u16, length: i32) -> i32;
    }

    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn SetDIBitsToDevice(
            hdc: Hdc,
            x_dest: i32,
            y_dest: i32,
            width: u32,
            height: u32,
            x_src: u32,
            y_src: u32,
            start_scan: u32,
            scan_lines: u32,
            bits: *const c_void,
            info: *const BitmapInfo,
            usage: u32,
        ) -> i32;
    }

    // Rust's standard library exposes no native audio-output interface, so
    // synchronous Windows playback must cross the WinMM ABI at this isolated
    // platform boundary.
    #[link(name = "winmm")]
    unsafe extern "system" {
        fn PlaySoundW(sound: *const u16, module: Hinstance, flags: u32) -> i32;
    }

    struct Tab {
        surface: Option<Surface>,
        runtime: Option<PageRuntime>,
        history: Vec<String>,
        history_pos: usize,
        scroll_y: i32,
    }

    impl Tab {
        fn blank() -> Self {
            Self {
                surface: None,
                runtime: None,
                history: Vec::new(),
                history_pos: 0,
                scroll_y: 0,
            }
        }

        fn current_url(&self) -> Option<&str> {
            self.history.get(self.history_pos).map(String::as_str)
        }
    }

    struct PageRuntime {
        host: Rc<RefCell<BrowserDomHost>>,
        js: amazing_js::Interpreter,
        layout: LayoutBox,
        media: PageMedia,
    }

    struct RenderedPage {
        surface: Surface,
        runtime: PageRuntime,
    }

    struct PageScript {
        src: Option<String>,
        body: String,
    }

    struct GuiState {
        hwnd: Hwnd,
        instance: Hinstance,
        address: Hwnd,
        go: Hwnd,
        back: Hwnd,
        forward: Hwnd,
        new_tab: Hwnd,
        tab_buttons: Vec<Hwnd>,
        tabs: Vec<Tab>,
        active_tab: usize,
        width: u32,
        viewport_height: i32,
        edit_previous_proc: isize,
    }

    impl GuiState {
        fn new(hwnd: Hwnd, instance: Hinstance) -> Self {
            Self {
                hwnd,
                instance,
                address: null_mut(),
                go: null_mut(),
                back: null_mut(),
                forward: null_mut(),
                new_tab: null_mut(),
                tab_buttons: Vec::new(),
                tabs: Vec::new(),
                active_tab: 0,
                width: 1024,
                viewport_height: 700,
                edit_previous_proc: 0,
            }
        }

        fn tab(&self) -> &Tab {
            &self.tabs[self.active_tab]
        }

        fn tab_mut(&mut self) -> &mut Tab {
            &mut self.tabs[self.active_tab]
        }

        fn active_button(&self) -> Hwnd {
            self.tab_buttons
                .get(self.active_tab)
                .copied()
                .unwrap_or(null_mut())
        }

        fn update_tab_button(&self) {
            let label = self
                .tab()
                .current_url()
                .map(|url| format!("{}: {}", self.active_tab + 1, url))
                .unwrap_or_else(|| format!("{}: New tab", self.active_tab + 1));
            set_window_text(self.active_button(), &label);
        }

        fn add_tab(&mut self) -> bool {
            let index = self.tabs.len();
            let class = wide("BUTTON");
            let label = wide(&format!("{}: New tab", index + 1));
            // SAFETY: the tab button is a child of the live top-level window;
            // its UTF-16 buffers stay alive through the synchronous call.
            let button = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    label.as_ptr(),
                    WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                    4 + (index as i32 * 154),
                    4,
                    150,
                    22,
                    self.hwnd,
                    (ID_TAB_BASE + index) as Hmenu,
                    self.instance,
                    null_mut(),
                )
            };
            if button.is_null() {
                return false;
            }
            self.tabs.push(Tab::blank());
            self.tab_buttons.push(button);
            true
        }

        fn activate_tab(&mut self, index: usize) {
            if index >= self.tabs.len() {
                return;
            }
            self.active_tab = index;
            if let Some(url) = self.tab().current_url() {
                set_window_text(self.address, url);
                set_window_text(self.hwnd, &format!("Ferric — {url}"));
            } else {
                set_window_text(self.address, "about:blank");
                set_window_text(self.hwnd, "Ferric — New tab");
            }
            self.update_tab_button();
            // SAFETY: hwnd is the live top-level window owned by the message
            // loop; invalidating it schedules a repaint only.
            unsafe { InvalidateRect(self.hwnd, null(), 0) };
        }

        fn navigate(&mut self, input: String, record: bool) {
            let input = self.resolve_navigation(input.trim());
            if input.is_empty() {
                return;
            }
            match render_page(&input, self.width) {
                Ok(rendered) => {
                    let tab = self.tab_mut();
                    tab.surface = Some(rendered.surface);
                    tab.runtime = Some(rendered.runtime);
                    tab.scroll_y = 0;
                    if record {
                        let tab = self.tab_mut();
                        tab.history.truncate(tab.history_pos.saturating_add(1));
                        tab.history.push(input.clone());
                        tab.history_pos = tab.history.len().saturating_sub(1);
                    }
                    let history_length = self.tab().history.len();
                    if let Some(runtime) = self.tab_mut().runtime.as_mut() {
                        runtime.js.set_history_length(history_length);
                    }
                    let script_navigation = self
                        .tab_mut()
                        .runtime
                        .as_mut()
                        .and_then(|runtime| runtime.js.take_navigation());
                    if let Some(url) = script_navigation {
                        if url != input {
                            self.navigate(url, true);
                            return;
                        }
                    }
                    set_window_text(self.address, &input);
                    set_window_text(self.hwnd, &format!("Ferric — {input}"));
                    self.update_tab_button();
                    // SAFETY: hwnd is the live top-level window owned by the
                    // message loop; invalidating it schedules a repaint only.
                    unsafe { InvalidateRect(self.hwnd, null(), 0) };
                }
                Err(error) => {
                    eprintln!("amazing gui: {error}");
                    set_window_text(self.hwnd, &format!("Ferric — error loading {input}"));
                }
            }
        }

        fn resolve_navigation(&self, input: &str) -> String {
            if input.contains("://") || input.starts_with("file:") || input.starts_with("about:") {
                return input.to_owned();
            }
            let Some(base) = self.tab().current_url() else {
                return input.to_owned();
            };
            if base == input {
                return input.to_owned();
            }
            if base.starts_with("http://") || base.starts_with("https://") || base.starts_with("file:") {
                if let Ok(base_url) = amazing_net::Url::parse(base) {
                    if let Ok(resolved) = amazing_net::resolve(input, &base_url) {
                        return resolved.to_string();
                    }
                }
            }
            let path = std::path::Path::new(base);
            if std::path::Path::new(input).is_absolute() {
                input.to_owned()
            } else {
                path.parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(input)
                    .to_string_lossy()
                    .into_owned()
            }
        }

        fn resize(&mut self, width: u32) {
            self.width = width.max(1);
            let control_width = width as i32;
            // SAFETY: child handles were created by this state and remain
            // valid until the parent window is destroyed.
            unsafe {
                MoveWindow(self.address, 4, 32, control_width - 300, 22, 1);
                MoveWindow(self.go, control_width - 290, 32, 65, 22, 1);
                MoveWindow(self.back, control_width - 215, 32, 65, 22, 1);
                MoveWindow(self.forward, control_width - 140, 32, 65, 22, 1);
                MoveWindow(self.new_tab, control_width - 65, 4, 55, 22, 1);
                for (index, button) in self.tab_buttons.iter().enumerate() {
                    MoveWindow(*button, 4 + index as i32 * 154, 4, 150, 22, 1);
                }
            }
            if let Some(current) = self.tab().current_url().map(str::to_owned) {
                self.navigate(current, false);
            }
        }

        fn set_viewport_height(&mut self, height: i32) {
            self.viewport_height = (height - CONTROL_HEIGHT).max(1);
            self.clamp_scroll();
        }

        fn clamp_scroll(&mut self) {
            let max_scroll = self
                .tab()
                .surface
                .as_ref()
                .map(|surface| surface.height as i32 - self.viewport_height)
                .unwrap_or(0)
                .max(0);
            let scroll = self.tab().scroll_y.clamp(0, max_scroll);
            self.tab_mut().scroll_y = scroll;
        }

        fn scroll(&mut self, delta: i32) {
            self.tab_mut().scroll_y = self.tab().scroll_y.saturating_add(delta);
            self.clamp_scroll();
            // SAFETY: hwnd is the live top-level window owned by the message
            // loop; invalidating it schedules a repaint only.
            unsafe { InvalidateRect(self.hwnd, null(), 0) };
        }

        fn repaint_runtime(&mut self) {
            let width = self.width;
            let page = self.tab().current_url().unwrap_or("").to_owned();
            let tab = self.tab_mut();
            let Some(runtime) = tab.runtime.as_mut() else { return };
            let host = runtime.host.clone();
            let host_ref = host.borrow();
            let mut css = String::new();
            if let Err(error) = collect_author_css(
                &host_ref.dom,
                NodeIdx(0),
                &page,
                &mut css,
            ) {
                eprintln!("amazing gui: stylesheet refresh failed: {error}");
                return;
            }
            let fonts = amazing_layout::font::FontSource::fallback();
            let media = load_page_media(&host_ref.dom, &page);
            for warning in &media.warnings {
                eprintln!("amazing gui: {warning}");
            }
            let layout = amazing_layout::layout_dom_document_with_replaced(
                &host_ref.dom,
                &css,
                width.max(1) as f32,
                &fonts,
                &media,
            );
            let height = (layout.border_box.y
                + layout.border_box.h
                + layout.margin.bottom.unwrap_or(0.0))
                .ceil()
                .clamp(1.0, 8192.0) as u32;
            let surface = amazing_paint::paint_document_with_resources(
                &layout,
                width.max(1),
                height,
                &fonts,
                &amazing_paint::RectInkProvider,
                &media,
            );
            runtime.layout = layout;
            runtime.media = media;
            tab.surface = Some(surface);
            tab.scroll_y = 0;
            self.clamp_scroll();
        }

        fn click(&mut self, x: i32, y: i32) {
            let page_y = y - CONTROL_HEIGHT + self.tab().scroll_y;
            if page_y < 0 {
                return;
            }
            let node = self
                .tab()
                .runtime
                .as_ref()
                .and_then(|runtime| hit_test(&runtime.layout, x as f32, page_y as f32));
            let Some(node) = node else { return };
            if let Some(runtime) = self.tab_mut().runtime.as_mut() {
                if let Err(error) = runtime.js.dispatch_dom_event(node.0, "click") {
                    eprintln!("amazing gui: click handler failed: {}", error.describe());
                    return;
                }
            }
            let navigation = self
                .tab_mut()
                .runtime
                .as_mut()
                .and_then(|runtime| runtime.js.take_navigation());
            if let Some(url) = navigation {
                self.navigate(url, true);
                return;
            }
            if let Some(delta) = self
                .tab_mut()
                .runtime
                .as_mut()
                .and_then(|runtime| runtime.js.take_history_delta())
            {
                self.apply_history_delta(delta);
                return;
            }
            if let Some((url, replace, state)) = self
                .tab_mut()
                .runtime
                .as_mut()
                .and_then(|runtime| runtime.js.take_history_update())
            {
                self.apply_history_update(url, replace, state);
                return;
            }
            let default_allowed = self
                .tab()
                .runtime
                .as_ref()
                .map(|runtime| !runtime.js.last_event_default_prevented())
                .unwrap_or(true);
            if default_allowed {
                let href = self
                    .tab()
                    .runtime
                    .as_ref()
                    .and_then(|runtime| runtime.host.borrow().get_attribute(node.0, "href"))
                    .flatten();
                if let Some(href) = href {
                    self.navigate(href, true);
                    return;
                }
            }
            self.repaint_runtime();
            // SAFETY: hwnd is the live top-level window owned by the message
            // loop; invalidating it schedules a repaint only.
            unsafe { InvalidateRect(self.hwnd, null(), 0) };
        }

        fn pump_timers(&mut self) {
            let fired = if let Some(runtime) = self.tab_mut().runtime.as_mut() {
                match runtime.js.pump_timers() {
                    Ok(fired) => fired,
                    Err(error) => {
                        eprintln!("amazing gui: timer callback failed: {}", error.describe());
                        false
                    }
                }
            } else {
                false
            };
            if let Some(delta) = self
                .tab_mut()
                .runtime
                .as_mut()
                .and_then(|runtime| runtime.js.take_history_delta())
            {
                self.apply_history_delta(delta);
                return;
            }
            if let Some((url, replace, state)) = self
                .tab_mut()
                .runtime
                .as_mut()
                .and_then(|runtime| runtime.js.take_history_update())
            {
                self.apply_history_update(url, replace, state);
                return;
            }
            let navigation = self
                .tab_mut()
                .runtime
                .as_mut()
                .and_then(|runtime| runtime.js.take_navigation());
            if let Some(url) = navigation {
                self.navigate(url, true);
                return;
            }
            if fired {
                self.repaint_runtime();
                // SAFETY: hwnd is the live top-level window owned by the
                // message loop; invalidating it schedules a repaint only.
                unsafe { InvalidateRect(self.hwnd, null(), 0) };
            }
        }

        fn apply_history_delta(&mut self, delta: i32) {
            if delta < 0 {
                for _ in 0..delta.unsigned_abs() { self.go_back(); }
            } else {
                for _ in 0..delta as u32 { self.go_forward(); }
            }
        }

        fn apply_history_update(&mut self, url: String, replace: bool, state: amazing_js::value::JsValue) {
            let url = self.resolve_navigation(&url);
            {
                let tab = self.tab_mut();
                if replace && !tab.history.is_empty() {
                    tab.history[tab.history_pos] = url.clone();
                } else {
                    tab.history.truncate(tab.history_pos.saturating_add(1));
                    tab.history.push(url.clone());
                    tab.history_pos = tab.history.len().saturating_sub(1);
                }
                let history_length = tab.history.len();
                if let Some(runtime) = tab.runtime.as_mut() {
                    runtime.js.set_location_url(&url);
                    runtime.js.set_history_length(history_length);
                    runtime.js.set_history_state(state);
                }
            }
            set_window_text(self.address, &url);
            set_window_text(self.hwnd, &format!("Ferric — {url}"));
            self.update_tab_button();
            // SAFETY: hwnd is the live top-level window owned by the message
            // loop; invalidating it schedules a repaint only.
            unsafe { InvalidateRect(self.hwnd, null(), 0) };
        }

        fn go_back(&mut self) {
            if self.tab().history_pos == 0 || self.tab().history.is_empty() {
                return;
            }
            self.tab_mut().history_pos -= 1;
            if let Some(url) = self.tab().current_url().map(str::to_owned) {
                self.navigate(url, false);
            }
        }

        fn go_forward(&mut self) {
            if self.tab().history_pos + 1 >= self.tab().history.len() {
                return;
            }
            self.tab_mut().history_pos += 1;
            if let Some(url) = self.tab().current_url().map(str::to_owned) {
                self.navigate(url, false);
            }
        }
    }

    fn hit_test(layout: &LayoutBox, x: f32, y: f32) -> Option<NodeIdx> {
        let rect = layout.border_box;
        if x < rect.x || y < rect.y || x >= rect.x + rect.w || y >= rect.y + rect.h {
            return None;
        }
        let children: &[LayoutBox] = match &layout.flow {
            Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children,
            Flow::Inlines(_) | Flow::Empty => &[],
        };
        for child in children.iter().rev() {
            if let Some(node) = hit_test(child, x, y) {
                return Some(node);
            }
        }
        layout.node
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn low_word(value: usize) -> usize {
        value & 0xffff
    }

    fn high_word(value: usize) -> u16 {
        ((value >> 16) & 0xffff) as u16
    }

    fn set_window_text(hwnd: Hwnd, text: &str) {
        if hwnd.is_null() {
            return;
        }
        let text = wide(text);
        // SAFETY: the UTF-16 buffer remains alive for the synchronous Win32
        // call and hwnd is a handle created by this process.
        unsafe { SetWindowTextW(hwnd, text.as_ptr()) };
    }

    fn window_text(hwnd: Hwnd) -> String {
        // SAFETY: the length query and subsequent buffer fill target a live
        // EDIT control owned by this process.
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        // SAFETY: buffer capacity is length + terminating NUL as required by
        // GetWindowTextW; the control handle is valid for the message loop.
        let written = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..written.max(0) as usize])
    }

    fn collect_author_css(
        dom: &Dom,
        node: NodeIdx,
        page: &str,
        out: &mut String,
    ) -> Result<(), String> {
        if dom.name(node) == Some("style") {
            for child in dom.children(node) {
                if let NodeKind::Text { data } = &dom.get(child).kind {
                    out.push_str(data);
                    out.push('\n');
                }
            }
            return Ok(());
        }
        if dom.name(node) == Some("link") {
            let attrs = dom.attrs_of(node).unwrap_or(&[]);
            let rel = attrs
                .iter()
                .find(|(name, _)| name == "rel")
                .map(|(_, value)| value.to_ascii_lowercase())
                .unwrap_or_default();
            if rel.split_ascii_whitespace().any(|token| token == "stylesheet") {
                if let Some(href) = attrs.iter().find(|(name, _)| name == "href").map(|(_, value)| value) {
                    let url = resolve_script_url(page, href);
                    let response = amazing_net::fetch(&url, &amazing_net::FetchOptions::default())
                        .map_err(|error| format!("stylesheet {href}: {error}"))?;
                    let css = response
                        .text()
                        .ok_or_else(|| format!("stylesheet {href} was not text"))?;
                    out.push_str(&css);
                    out.push('\n');
                }
            }
        }
        for child in dom.children(node) {
            collect_author_css(dom, child, page, out)?;
        }
        Ok(())
    }

    fn collect_scripts(dom: &Dom, node: NodeIdx, scripts: &mut Vec<PageScript>) {
        if dom.name(node) == Some("script") {
            let mut source = String::new();
            for child in dom.children(node) {
                if let NodeKind::Text { data } = &dom.get(child).kind {
                    source.push_str(data);
                }
            }
            let src = dom
                .attrs_of(node)
                .and_then(|attrs| attrs.iter().find(|(name, _)| name == "src"))
                .map(|(_, value)| value.clone());
            if src.is_some() || !source.trim().is_empty() {
                scripts.push(PageScript { src, body: source });
            }
            return;
        }
        for child in dom.children(node) {
            collect_scripts(dom, child, scripts);
        }
    }

    fn collect_inline_handlers(dom: &Dom, node: NodeIdx, handlers: &mut Vec<(u32, String, String)>) {
        if let Some(attrs) = dom.attrs_of(node) {
            for (name, body) in attrs {
                if let Some(event) = name.strip_prefix("on").filter(|event| !event.is_empty()) {
                    handlers.push((node.0, event.to_owned(), body.clone()));
                }
            }
        }
        for child in dom.children(node) {
            collect_inline_handlers(dom, child, handlers);
        }
    }

    fn resolve_script_url(page: &str, source: &str) -> String {
        if source.contains("://") || source.starts_with("file:") {
            return source.to_owned();
        }
        if page.starts_with("http://") || page.starts_with("https://") || page.starts_with("file:") {
            if let Ok(base) = amazing_net::Url::parse(page) {
                if let Ok(resolved) = amazing_net::resolve(source, &base) {
                    return resolved.to_string();
                }
            }
        }
        std::path::Path::new(page)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(source)
            .to_string_lossy()
            .into_owned()
    }

    fn render_page(input: &str, width: u32) -> Result<RenderedPage, String> {
        let response = amazing_net::fetch(input, &amazing_net::FetchOptions::default())
            .map_err(|error| error.to_string())?;
        let html = response
            .text()
            .ok_or_else(|| "response was not decodable as text".to_owned())?;
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(&html),
        }));
        let scripts = {
            let host_ref = host.borrow();
            let mut scripts = Vec::new();
            collect_scripts(&host_ref.dom, NodeIdx(0), &mut scripts);
            scripts
        };
        let mut js = amazing_js::Interpreter::new();
        js.set_dom_host(host.clone());
        js.set_location_url(input);
        js.set_viewport_size(width, 700);
        let handlers = {
            let host_ref = host.borrow();
            let mut handlers = Vec::new();
            collect_inline_handlers(&host_ref.dom, NodeIdx(0), &mut handlers);
            handlers
        };
        for (node, event, body) in handlers {
            js.install_inline_handler(node, &event, &body)
                .map_err(|error| format!("inline onclick failed: {}", error.describe()))?;
        }
        for script in scripts {
            let source = match script.src {
                Some(src) => {
                    let script_url = resolve_script_url(input, &src);
                    let response = amazing_net::fetch(
                        &script_url,
                        &amazing_net::FetchOptions::default(),
                    )
                    .map_err(|error| format!("external script {src}: {error}"))?;
                    response
                        .text()
                        .ok_or_else(|| format!("external script {src} was not text"))?
                }
                None => script.body,
            };
            js.run_script(&source)
                .map_err(|error| format!("page script failed: {}", error.describe()))?;
        }
        js.dispatch_dom_event(0, "DOMContentLoaded")
            .map_err(|error| format!("DOMContentLoaded listener failed: {}", error.describe()))?;
        let mut css = String::new();
        {
            let host_ref = host.borrow();
            collect_author_css(&host_ref.dom, NodeIdx(0), input, &mut css)?;
        }
        js.dispatch_dom_event(0, "load")
            .map_err(|error| format!("load listener failed: {}", error.describe()))?;
        let body = host.borrow_mut().query_selector("body");
        if let Some(body) = body {
            js.dispatch_dom_event(body, "load")
                .map_err(|error| format!("body load listener failed: {}", error.describe()))?;
        }
        css.clear();
        {
            let host_ref = host.borrow();
            collect_author_css(&host_ref.dom, NodeIdx(0), input, &mut css)?;
        }
        let host_ref = host.borrow();
        let fonts = amazing_layout::font::FontSource::fallback();
        let media = load_page_media(&host_ref.dom, input);
        for warning in &media.warnings {
            eprintln!("amazing gui: {warning}");
        }
        let root = amazing_layout::layout_dom_document_with_replaced(
            &host_ref.dom,
            &css,
            width.max(1) as f32,
            &fonts,
            &media,
        );
        let height = (root.border_box.y + root.border_box.h + root.margin.bottom.unwrap_or(0.0))
            .ceil()
            .clamp(1.0, 8192.0) as u32;
        let surface = amazing_paint::paint_document_with_resources(
            &root,
            width.max(1),
            height,
            &fonts,
            &amazing_paint::RectInkProvider,
            &media,
        );
        drop(host_ref);
        Ok(RenderedPage {
            surface,
            runtime: PageRuntime { host, js, layout: root, media },
        })
    }

    fn paint(hwnd: Hwnd, state: &GuiState) {
        let mut paint = std::mem::MaybeUninit::<PaintStruct>::zeroed();
        // SAFETY: BeginPaint/EndPaint are paired around this WM_PAINT handler;
        // the returned HDC is valid only within that pair.
        let hdc = unsafe { BeginPaint(hwnd, paint.as_mut_ptr()) };
        if hdc.is_null() {
            return;
        }
        let paint = unsafe { paint.assume_init() };
        if let Some(surface) = &state.tab().surface {
            let mut bgra = Vec::with_capacity(surface.pixels.len());
            for pixel in surface.pixels.chunks_exact(4) {
                bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
            let info = BitmapInfo {
                header: BitmapInfoHeader {
                    size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                    width: surface.width as i32,
                    height: -(surface.height as i32),
                    planes: 1,
                    bit_count: 32,
                    compression: 0,
                    image_size: 0,
                    x_pels_per_meter: 0,
                    y_pels_per_meter: 0,
                    clr_used: 0,
                    clr_important: 0,
                },
            };
            let scroll = state.tab().scroll_y.max(0) as usize;
            let stride = surface.width as usize * 4;
            let skip = scroll.saturating_mul(stride).min(bgra.len());
            let visible = surface
                .height
                .saturating_sub(scroll as u32)
                .min(state.viewport_height.max(1) as u32);
            // SAFETY: bgra and info remain alive for the synchronous GDI call;
            // the source pointer starts at the selected top row and dimensions
            // match the owned Surface pixel buffer.
            unsafe {
                SetDIBitsToDevice(
                    hdc,
                    0,
                    CONTROL_HEIGHT,
                    surface.width,
                    visible,
                    0,
                    0,
                    0,
                    visible,
                    bgra[skip..].as_ptr().cast(),
                    &info,
                    DIB_RGB_COLORS,
                );
            }
        }
        // SAFETY: paint was initialized by BeginPaint and belongs to hwnd.
        unsafe { EndPaint(hwnd, &paint) };
    }

    unsafe extern "system" fn edit_proc(
        hwnd: Hwnd,
        message: Uint,
        w_param: Wparam,
        l_param: Lparam,
    ) -> Lresult {
        if message == WM_KEYDOWN && w_param == VK_RETURN {
            // SAFETY: the EDIT control's parent is the top-level window that
            // owns the GUI state; both handles are live during the message.
            let parent = unsafe { GetParent(hwnd) };
            let state_ptr = unsafe { GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut GuiState };
            if !state_ptr.is_null() {
                let state = unsafe { &mut *state_ptr };
                state.navigate(window_text(hwnd), true);
                return 0;
            }
        }
        let parent = unsafe { GetParent(hwnd) };
        let state_ptr = unsafe { GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut GuiState };
        let previous = if state_ptr.is_null() {
            0
        } else {
            unsafe { (*state_ptr).edit_previous_proc }
        };
        if previous == 0 {
            return unsafe { DefWindowProcW(hwnd, message, w_param, l_param) };
        }
        // SAFETY: previous is the original EDIT window procedure returned by
        // SetWindowLongPtrW; forwarding preserves native edit behavior.
        unsafe { CallWindowProcW(previous, hwnd, message, w_param, l_param) }
    }

    unsafe extern "system" fn window_proc(
        hwnd: Hwnd,
        message: Uint,
        w_param: Wparam,
        l_param: Lparam,
    ) -> Lresult {
        let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GuiState };
        match message {
            WM_COMMAND if !state_ptr.is_null() && high_word(w_param) == BN_CLICKED => {
                let state = unsafe { &mut *state_ptr };
                let command = low_word(w_param);
                match command {
                    ID_GO => state.navigate(window_text(state.address), true),
                    ID_BACK => state.go_back(),
                    ID_FORWARD => state.go_forward(),
                    ID_NEW_TAB => {
                        if state.add_tab() {
                            let index = state.tabs.len() - 1;
                            state.active_tab = index;
                            state.resize(state.width);
                            state.activate_tab(index);
                        }
                    }
                    id if id >= ID_TAB_BASE && id < ID_TAB_BASE + state.tabs.len() => {
                        state.activate_tab(id - ID_TAB_BASE);
                    }
                    _ => {}
                }
                0
            }
            WM_SIZE if !state_ptr.is_null() => {
                let width = (l_param as usize & 0xffff) as u32;
                let height = ((l_param as usize >> 16) & 0xffff) as i32;
                let state = unsafe { &mut *state_ptr };
                state.set_viewport_height(height);
                state.resize(width);
                0
            }
            WM_MOUSEWHEEL if !state_ptr.is_null() => {
                let delta = ((w_param >> 16) & 0xffff) as u16 as i16;
                unsafe { &mut *state_ptr }.scroll(if delta > 0 { -96 } else { 96 });
                0
            }
            WM_LBUTTONUP if !state_ptr.is_null() => {
                let x = (l_param as usize & 0xffff) as u16 as i16 as i32;
                let y = ((l_param as usize >> 16) & 0xffff) as u16 as i16 as i32;
                unsafe { &mut *state_ptr }.click(x, y);
                0
            }
            WM_TIMER if !state_ptr.is_null() => {
                unsafe { &mut *state_ptr }.pump_timers();
                0
            }
            WM_PAINT if !state_ptr.is_null() => {
                paint(hwnd, unsafe { &*state_ptr });
                0
            }
            WM_DESTROY => {
                // SAFETY: the timer belongs to this top-level window and is
                // stopped before the message loop exits.
                unsafe { KillTimer(hwnd, GUI_TIMER_ID) };
                // SAFETY: posting WM_QUIT is the documented way to terminate
                // this thread's GetMessage loop after the top-level window.
                unsafe { PostQuitMessage(0) };
                0
            }
            _ => {
                // SAFETY: forwarding unhandled messages to DefWindowProcW is
                // required by the Win32 window-procedure contract.
                unsafe { DefWindowProcW(hwnd, message, w_param, l_param) }
            }
        }
    }

    pub fn run(initial: Option<&str>) -> ExitCode {
        // SAFETY: GetConsoleWindow returns this process's optional console;
        // hiding it leaves the native GUI visible without changing console
        // behavior for the other command-line subcommands.
        unsafe {
            let console = GetConsoleWindow();
            if !console.is_null() { ShowWindow(console, 0); }
        }
        let class_name = wide("FerricWindow");
        let title = wide("Ferric — from-scratch browser engine");
        // SAFETY: null asks Windows for this process's module handle.
        let instance = unsafe { GetModuleHandleW(null()) };
        let class = WndClassW {
            style: CS_HREDRAW | CS_VREDRAW,
            wnd_proc: Some(window_proc),
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: null_mut(),
            cursor: null_mut(),
            background: null_mut(),
            menu_name: null(),
            class_name: class_name.as_ptr(),
        };
        // SAFETY: class_name and class live through registration; all fields
        // use process-owned values or null handles accepted by Win32.
        if unsafe { RegisterClassW(&class) } == 0 {
            eprintln!("amazing gui: RegisterClassW failed");
            return ExitCode::FAILURE;
        }
        // SAFETY: all pointers reference NUL-terminated UTF-16 buffers that
        // remain alive for the synchronous window creation call.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0x80000000u32 as i32,
                0x80000000u32 as i32,
                1100,
                760,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            )
        };
        if hwnd.is_null() {
            eprintln!("amazing gui: CreateWindowExW failed");
            return ExitCode::FAILURE;
        }

        let mut state = Box::new(GuiState::new(hwnd, instance));
        let edit_class = wide("EDIT");
        let button_class = wide("BUTTON");
        let address_text = wide(initial.unwrap_or("examples/sample.html"));
        let go_text = wide("Go");
        let back_text = wide("Back");
        let forward_text = wide("Forward");
        let new_tab_text = wide("+");
        // SAFETY: child controls use the just-created parent and stable UTF-16
        // class/text buffers; handles are retained in GuiState for the loop.
        unsafe {
            state.address = CreateWindowExW(
                0,
                edit_class.as_ptr(),
                address_text.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL | ES_LEFT,
                4,
                4,
                800,
                22,
                hwnd,
                ID_ADDRESS as Hmenu,
                instance,
                null_mut(),
            );
            state.go = CreateWindowExW(
                0,
                button_class.as_ptr(),
                go_text.as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                810,
                4,
                65,
                22,
                hwnd,
                ID_GO as Hmenu,
                instance,
                null_mut(),
            );
            state.back = CreateWindowExW(
                0,
                button_class.as_ptr(),
                back_text.as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                885,
                4,
                65,
                22,
                hwnd,
                ID_BACK as Hmenu,
                instance,
                null_mut(),
            );
            state.forward = CreateWindowExW(
                0,
                button_class.as_ptr(),
                forward_text.as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                960,
                4,
                65,
                22,
                hwnd,
                ID_FORWARD as Hmenu,
                instance,
                null_mut(),
            );
            state.new_tab = CreateWindowExW(
                0,
                button_class.as_ptr(),
                new_tab_text.as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                1035,
                4,
                55,
                22,
                hwnd,
                ID_NEW_TAB as Hmenu,
                instance,
                null_mut(),
            );
        }
        if state.address.is_null() || state.go.is_null() || state.back.is_null() || state.forward.is_null() || state.new_tab.is_null() {
            eprintln!("amazing gui: could not create navigation controls");
            return ExitCode::FAILURE;
        }
        if !state.add_tab() {
            eprintln!("amazing gui: could not create the initial tab control");
            return ExitCode::FAILURE;
        }
        let state_ptr = Box::into_raw(state);
        // SAFETY: state_ptr remains allocated until the message loop exits;
        // GWLP_USERDATA stores it only as an opaque per-window value.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize) };
        // SAFETY: edit_proc is an ABI-compatible subclass procedure and the
        // returned value is retained so unhandled edit messages can be
        // forwarded to the stock EDIT procedure.
        unsafe {
            (*state_ptr).edit_previous_proc = SetWindowLongPtrW(
                (*state_ptr).address,
                GWLP_WNDPROC,
                edit_proc as *const () as usize as isize,
            );
            SetTimer(hwnd, GUI_TIMER_ID, 16, null_mut());
        }
        unsafe { &mut *state_ptr }.navigate(initial.unwrap_or("examples/sample.html").to_owned(), true);
        // SAFETY: hwnd is a valid top-level window and the message-loop calls
        // follow the Win32 lifetime contract.
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
        }
        let mut message = Msg {
            hwnd: null_mut(),
            message: 0,
            w_param: 0,
            l_param: 0,
            time: 0,
            point: Point { x: 0, y: 0 },
        };
        loop {
            // SAFETY: message points to writable storage owned by this loop;
            // null hwnd requests messages for the current GUI thread.
            let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
            if result <= 0 {
                break;
            }
            // SAFETY: message was filled by GetMessageW and is valid for both
            // translation and dispatch during this iteration.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        // SAFETY: the loop has ended, so no window callback can access the
        // state pointer; reclaim the Box exactly once.
        unsafe { drop(Box::from_raw(state_ptr)) };
        ExitCode::SUCCESS
    }

    pub fn play_wav_file(path: &str) -> ExitCode {
        let path = wide(path);
        // SAFETY: `path` is a live, null-terminated UTF-16 filename for the
        // duration of this synchronous WinMM call; a null module is required
        // with SND_FILENAME and no callback or retained pointer is involved.
        // Rust's standard library cannot express native audio playback, which
        // is why this operation uses the platform FFI seam.
        let played = unsafe {
            PlaySoundW(path.as_ptr(), null_mut(), SND_FILENAME | SND_NODEFAULT)
        };
        if played == 0 {
            eprintln!("amazing play-audio: WinMM could not play the WAV file");
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn find_layout_box(layout: &LayoutBox, node: NodeIdx) -> Option<&LayoutBox> {
            if layout.node == Some(node) {
                return Some(layout);
            }
            let children: &[LayoutBox] = match &layout.flow {
                Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children,
                Flow::Inlines(_) | Flow::Empty => &[],
            };
            children
                .iter()
                .find_map(|child| find_layout_box(child, node))
        }

        #[test]
        fn page_click_runtime_mutates_dom_and_repaints() {
            let rendered = render_page("../../examples/phase8.html", 640).expect("fixture renders");
            let RenderedPage { surface, runtime } = rendered;
            let mut state = GuiState::new(null_mut(), null_mut());
            state.width = 640;
            state.tabs.push(Tab {
                surface: Some(surface),
                runtime: Some(runtime),
                history: vec!["../../examples/phase8.html".into()],
                history_pos: 0,
                scroll_y: 0,
            });
            let node = {
                let host = state.tab().runtime.as_ref().unwrap().host.clone();
                host.borrow_mut()
                    .query_selector("#title")
                    .expect("fixture target")
            };
            state
                .tab_mut()
                .runtime
                .as_mut()
                .unwrap()
                .js
                .dispatch_dom_event(node, "click")
                .expect("click dispatch");
            state.repaint_runtime();
            state
                .tab_mut()
                .runtime
                .as_mut()
                .unwrap()
                .js
                .pump_timers()
                .expect("timer pump");
            state.repaint_runtime();
            let host = state.tab().runtime.as_ref().unwrap().host.clone();
            let host = host.borrow();
            assert_eq!(
                host.get_text_content(node).as_deref(),
                Some("click handled by Ferric!")
            );
            assert_eq!(host.get_attribute(node, "data-ready"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-load"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-body-load"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-window"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-dom-api"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-selector-list"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-collections"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-matches"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-dataset"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-attr-api"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-temp"), Some(None));
            assert_eq!(host.get_attribute(node, "data-classlist"), Some(Some("ready active".into())));
            assert_eq!(host.get_attribute(node, "data-viewport"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-inner"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-inner-text"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-text-node"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-fetch"), Some(Some("fetched\n".into())));
            assert_eq!(host.get_attribute(node, "data-header"), Some(Some("text/plain; charset=utf-8".into())));
            assert_eq!(host.get_attribute(node, "data-json"), Some(Some("json-ok".into())));
            assert_eq!(host.get_attribute(node, "data-clicked"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-bubbled"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-inline"), Some(Some("yes".into())));
            assert_eq!(host.get_attribute(node, "data-timer"), Some(Some("yes".into())));
            assert_eq!(host.get_style(node, "color").as_deref(), Some("white"));
            assert_eq!(
                host.get_style(node, "backgroundColor").as_deref(),
                Some("red")
            );
            let runtime = state.tab().runtime.as_ref().unwrap();
            let title_box = find_layout_box(&runtime.layout, NodeIdx(node)).unwrap();
            let x = (title_box.border_box.x + title_box.border_box.w * 0.75) as u32;
            let y = (title_box.border_box.y + title_box.border_box.h * 0.5) as u32;
            let surface = state.tab().surface.as_ref().unwrap();
            let offset = ((y * surface.width + x) * 4) as usize;
            assert_eq!(&surface.pixels[offset..offset + 4], &[255, 0, 0, 255]);
        }
    }

}

#[cfg(windows)]
pub use windows::{play_wav_file, run};
