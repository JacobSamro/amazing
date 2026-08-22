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

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::process::ExitCode;

    use amazing_dom::{Dom, NodeIdx, NodeKind};
    use amazing_paint::Surface;

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
    const BN_CLICKED: u16 = 0;
    const GWLP_USERDATA: i32 = -21;
    const DIB_RGB_COLORS: u32 = 0;
    const ID_ADDRESS: usize = 101;
    const ID_GO: usize = 102;
    const ID_BACK: usize = 103;
    const ID_FORWARD: usize = 104;
    const CONTROL_HEIGHT: i32 = 28;

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
        fn GetWindowLongPtrW(hwnd: Hwnd, index: i32) -> isize;
        fn InvalidateRect(hwnd: Hwnd, rect: *const Rect, erase: i32) -> i32;
        fn MoveWindow(hwnd: Hwnd, x: i32, y: i32, width: i32, height: i32, repaint: i32) -> i32;
        fn PostQuitMessage(code: i32);
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

    struct GuiState {
        hwnd: Hwnd,
        address: Hwnd,
        go: Hwnd,
        back: Hwnd,
        forward: Hwnd,
        width: u32,
        surface: Option<Surface>,
        history: Vec<String>,
        history_pos: usize,
    }

    impl GuiState {
        fn new(hwnd: Hwnd) -> Self {
            Self {
                hwnd,
                address: null_mut(),
                go: null_mut(),
                back: null_mut(),
                forward: null_mut(),
                width: 1024,
                surface: None,
                history: Vec::new(),
                history_pos: 0,
            }
        }

        fn navigate(&mut self, input: String, record: bool) {
            let input = input.trim().to_owned();
            if input.is_empty() {
                return;
            }
            match render_page(&input, self.width) {
                Ok(surface) => {
                    self.surface = Some(surface);
                    if record {
                        self.history.truncate(self.history_pos.saturating_add(1));
                        self.history.push(input.clone());
                        self.history_pos = self.history.len().saturating_sub(1);
                    }
                    set_window_text(self.address, &input);
                    set_window_text(self.hwnd, &format!("Ferric — {input}"));
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

        fn resize(&mut self, width: u32) {
            self.width = width.max(1);
            let control_width = width as i32;
            // SAFETY: child handles were created by this state and remain
            // valid until the parent window is destroyed.
            unsafe {
                MoveWindow(self.address, 4, 4, control_width - 300, 22, 1);
                MoveWindow(self.go, control_width - 290, 4, 65, 22, 1);
                MoveWindow(self.back, control_width - 215, 4, 65, 22, 1);
                MoveWindow(self.forward, control_width - 140, 4, 65, 22, 1);
            }
            if let Some(current) = self.history.get(self.history_pos).cloned() {
                self.navigate(current, false);
            }
        }

        fn go_back(&mut self) {
            if self.history_pos == 0 || self.history.is_empty() {
                return;
            }
            self.history_pos -= 1;
            if let Some(url) = self.history.get(self.history_pos).cloned() {
                self.navigate(url, false);
            }
        }

        fn go_forward(&mut self) {
            if self.history_pos + 1 >= self.history.len() {
                return;
            }
            self.history_pos += 1;
            if let Some(url) = self.history.get(self.history_pos).cloned() {
                self.navigate(url, false);
            }
        }
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

    fn collect_style_text(dom: &Dom, node: NodeIdx, out: &mut String) {
        if dom.name(node) == Some("style") {
            for child in dom.children(node) {
                if let NodeKind::Text { data } = &dom.get(child).kind {
                    out.push_str(data);
                    out.push('\n');
                }
            }
            return;
        }
        for child in dom.children(node) {
            collect_style_text(dom, child, out);
        }
    }

    fn render_page(input: &str, width: u32) -> Result<Surface, String> {
        let response = amazing_net::fetch(input, &amazing_net::FetchOptions::default())
            .map_err(|error| error.to_string())?;
        let html = response
            .text()
            .ok_or_else(|| "response was not decodable as text".to_owned())?;
        let dom = amazing_html::parse(&html);
        let mut css = String::new();
        collect_style_text(&dom, NodeIdx(0), &mut css);
        let fonts = amazing_layout::font::FontSource::fallback();
        let root = amazing_layout::layout_document(&html, &css, width.max(1) as f32, &fonts);
        let height = (root.border_box.y + root.border_box.h + root.margin.bottom.unwrap_or(0.0))
            .ceil()
            .clamp(1.0, 8192.0) as u32;
        Ok(amazing_paint::paint_document(&root, width.max(1), height, &fonts))
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
        if let Some(surface) = &state.surface {
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
            // SAFETY: bgra and info remain alive for the synchronous GDI call;
            // dimensions and stride match the owned Surface pixel buffer.
            unsafe {
                SetDIBitsToDevice(
                    hdc,
                    0,
                    CONTROL_HEIGHT,
                    surface.width,
                    surface.height,
                    0,
                    0,
                    0,
                    surface.height,
                    bgra.as_ptr().cast(),
                    &info,
                    DIB_RGB_COLORS,
                );
            }
        }
        // SAFETY: paint was initialized by BeginPaint and belongs to hwnd.
        unsafe { EndPaint(hwnd, &paint) };
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
                match low_word(w_param) {
                    ID_GO => state.navigate(window_text(state.address), true),
                    ID_BACK => state.go_back(),
                    ID_FORWARD => state.go_forward(),
                    _ => {}
                }
                0
            }
            WM_SIZE if !state_ptr.is_null() => {
                let width = (l_param as usize & 0xffff) as u32;
                unsafe { &mut *state_ptr }.resize(width);
                0
            }
            WM_PAINT if !state_ptr.is_null() => {
                paint(hwnd, unsafe { &*state_ptr });
                0
            }
            WM_DESTROY => {
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

        let mut state = Box::new(GuiState::new(hwnd));
        let edit_class = wide("EDIT");
        let button_class = wide("BUTTON");
        let address_text = wide(initial.unwrap_or("examples/sample.html"));
        let go_text = wide("Go");
        let back_text = wide("Back");
        let forward_text = wide("Forward");
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
        }
        if state.address.is_null() || state.go.is_null() || state.back.is_null() || state.forward.is_null() {
            eprintln!("amazing gui: could not create navigation controls");
            return ExitCode::FAILURE;
        }
        let state_ptr = Box::into_raw(state);
        // SAFETY: state_ptr remains allocated until the message loop exits;
        // GWLP_USERDATA stores it only as an opaque per-window value.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize) };
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

}

#[cfg(windows)]
pub use windows::run;
