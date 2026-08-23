//! Windows hardware-acceleration capability seam.
//!
//! The engine remains platform-independent and software-rasterized. This
//! module only proves that the Windows shell can acquire a real D3D11
//! hardware device, while keeping COM and raw ABI operations isolated.

pub struct GpuInfo {
    pub backend: &'static str,
    pub feature_level: &'static str,
}

pub struct PresentInfo {
    pub backend: &'static str,
    pub width: u32,
    pub height: u32,
}

#[cfg(not(windows))]
pub fn probe() -> Result<GpuInfo, String> {
    Err("this build has no native GPU adapter".into())
}

#[cfg(not(windows))]
pub fn presentation_smoke() -> Result<PresentInfo, String> {
    Err("this build has no native GPU presenter".into())
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::ptr::null_mut;

    use super::{GpuInfo, PresentInfo};

    const D3D_DRIVER_TYPE_HARDWARE: u32 = 1;
    const D3D11_CREATE_DEVICE_BGRA_SUPPORT: u32 = 0x20;
    const D3D11_SDK_VERSION: u32 = 7;
    const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
    const FEATURE_LEVELS: [u32; 4] = [0xb100, 0xb000, 0xa100, 0xa000];
    const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
    const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
    const DXGI_SWAP_EFFECT_DISCARD: u32 = 0;
    const WS_POPUP: u32 = 0x8000_0000;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const IID_ID3D11_TEXTURE_2D: Guid = Guid {
        data1: 0x6f15_aaf2,
        data2: 0xd208,
        data3: 0x4e89,
        data4: [0x9a, 0xb4, 0x48, 0x95, 0x35, 0xd3, 0x4f, 0x9c],
    };

    #[repr(C)]
    struct DxgiRational {
        numerator: u32,
        denominator: u32,
    }

    #[repr(C)]
    struct DxgiModeDesc {
        width: u32,
        height: u32,
        refresh_rate: DxgiRational,
        format: u32,
        scanline_ordering: u32,
        scaling: u32,
    }

    #[repr(C)]
    struct DxgiSampleDesc {
        count: u32,
        quality: u32,
    }

    #[repr(C)]
    struct DxgiSwapChainDesc {
        buffer_desc: DxgiModeDesc,
        sample_desc: DxgiSampleDesc,
        buffer_usage: u32,
        buffer_count: u32,
        output_window: *mut c_void,
        windowed: i32,
        swap_effect: u32,
        flags: u32,
    }

    #[repr(C)]
    struct IUnknownVTable {
        query_interface: *const c_void,
        add_ref: *const c_void,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct SwapChainVTable {
        before_present: [*const c_void; 8],
        present: unsafe extern "system" fn(*mut c_void, u32, u32) -> i32,
        get_buffer:
            unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32,
        before_resize_buffers: [*const c_void; 3],
        resize_buffers: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, u32) -> i32,
    }

    #[repr(C)]
    struct DeviceContextVTable {
        before_update_subresource: [*const c_void; 48],
        update_subresource: unsafe extern "system" fn(
            *mut c_void,
            *mut c_void,
            u32,
            *const c_void,
            *const c_void,
            u32,
            u32,
        ),
    }

    // Rust's standard library has no Direct3D device interface, so the
    // platform adapter must cross the documented D3D11 system ABI.
    #[link(name = "d3d11")]
    unsafe extern "system" {
        fn D3D11CreateDevice(
            adapter: *mut c_void,
            driver_type: u32,
            software: *mut c_void,
            flags: u32,
            feature_levels: *const u32,
            feature_level_count: u32,
            sdk_version: u32,
            device: *mut *mut c_void,
            selected_feature_level: *mut u32,
            immediate_context: *mut *mut c_void,
        ) -> i32;
        fn D3D11CreateDeviceAndSwapChain(
            adapter: *mut c_void,
            driver_type: u32,
            software: *mut c_void,
            flags: u32,
            feature_levels: *const u32,
            feature_level_count: u32,
            sdk_version: u32,
            swap_chain_desc: *const DxgiSwapChainDesc,
            swap_chain: *mut *mut c_void,
            device: *mut *mut c_void,
            selected_feature_level: *mut u32,
            immediate_context: *mut *mut c_void,
        ) -> i32;
    }

    // `std` cannot create or destroy HWND values, so the isolated Windows
    // presentation smoke test crosses user32 for a temporary hidden window.
    #[link(name = "user32")]
    unsafe extern "system" {
        fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: *mut c_void,
            menu: *mut c_void,
            instance: *mut c_void,
            parameter: *mut c_void,
        ) -> *mut c_void;
        fn DestroyWindow(window: *mut c_void) -> i32;
    }

    unsafe fn release_com(object: *mut c_void) {
        if object.is_null() {
            return;
        }
        // SAFETY: successful D3D11 creation returns a COM interface whose
        // first pointer is an IUnknown-compatible vtable; Release is its
        // third entry and is called exactly once for this owned reference.
        // `std` has no COM ownership primitive or Direct3D binding.
        unsafe {
            let vtable = *(object as *mut *const IUnknownVTable);
            ((*vtable).release)(object);
        }
    }

    fn feature_level_name(level: u32) -> &'static str {
        match level {
            0xb100 => "11.1",
            0xb000 => "11.0",
            0xa100 => "10.1",
            0xa000 => "10.0",
            _ => "unknown",
        }
    }

    fn create(levels: &[u32]) -> Result<u32, i32> {
        let mut device = null_mut();
        let mut context = null_mut();
        let mut selected = 0;
        // SAFETY: all output pointers reference writable local storage;
        // adapter/software are null for a hardware default adapter, the
        // feature-level slice remains alive for the synchronous call, and
        // successful COM references are released before returning. Rust's
        // standard library cannot create a Direct3D hardware device.
        let result = unsafe {
            D3D11CreateDevice(
                null_mut(),
                D3D_DRIVER_TYPE_HARDWARE,
                null_mut(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                levels.as_ptr(),
                levels.len() as u32,
                D3D11_SDK_VERSION,
                &mut device,
                &mut selected,
                &mut context,
            )
        };
        if result < 0 {
            return Err(result);
        }
        if device.is_null() || context.is_null() {
            // SAFETY: either pointer, when non-null, is an owned COM
            // reference returned by the successful D3D11 call above; `std`
            // has no COM reference-release operation.
            unsafe {
                release_com(context);
                release_com(device);
            }
            return Err(E_INVALIDARG);
        }
        // SAFETY: both pointers are distinct owned references returned by
        // D3D11CreateDevice and are released once after capability probing;
        // `std` has no COM reference-release operation.
        unsafe {
            release_com(context);
            release_com(device);
        }
        Ok(selected)
    }

    pub struct Presenter {
        swap_chain: *mut c_void,
        device: *mut c_void,
        context: *mut c_void,
        width: u32,
        height: u32,
    }

    impl Presenter {
        /// Creates a hardware swap-chain presenter for one native window.
        ///
        /// # Safety
        /// `window` must be a live HWND owned by the calling thread and must
        /// remain valid until this presenter is dropped.
        pub unsafe fn new(window: *mut c_void, width: u32, height: u32) -> Result<Self, String> {
            if window.is_null() || width == 0 || height == 0 {
                return Err("D3D11 presenter needs a live window and non-zero size".into());
            }
            let desc = DxgiSwapChainDesc {
                buffer_desc: DxgiModeDesc {
                    width,
                    height,
                    refresh_rate: DxgiRational {
                        numerator: 0,
                        denominator: 0,
                    },
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    scanline_ordering: 0,
                    scaling: 0,
                },
                sample_desc: DxgiSampleDesc {
                    count: 1,
                    quality: 0,
                },
                buffer_usage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                buffer_count: 1,
                output_window: window,
                windowed: 1,
                swap_effect: DXGI_SWAP_EFFECT_DISCARD,
                flags: 0,
            };

            unsafe fn attempt(
                levels: &[u32],
                desc: &DxgiSwapChainDesc,
            ) -> Result<(*mut c_void, *mut c_void, *mut c_void), i32> {
                let mut swap_chain = null_mut();
                let mut device = null_mut();
                let mut context = null_mut();
                let mut selected = 0;
                // SAFETY: the descriptor and feature-level slice remain live
                // for this synchronous call; output pointers target writable
                // locals; the HWND was required live by Presenter::new; and
                // returned COM references are either stored or released.
                // `std` cannot create Direct3D devices or DXGI swap chains.
                let result = unsafe {
                    D3D11CreateDeviceAndSwapChain(
                        null_mut(),
                        D3D_DRIVER_TYPE_HARDWARE,
                        null_mut(),
                        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                        levels.as_ptr(),
                        levels.len() as u32,
                        D3D11_SDK_VERSION,
                        desc,
                        &mut swap_chain,
                        &mut device,
                        &mut selected,
                        &mut context,
                    )
                };
                if result < 0 || swap_chain.is_null() || device.is_null() || context.is_null() {
                    // SAFETY: any non-null outputs are owned COM references
                    // returned by the call above; `std` has no COM release.
                    unsafe {
                        release_com(context);
                        release_com(swap_chain);
                        release_com(device);
                    }
                    return Err(if result < 0 { result } else { E_INVALIDARG });
                }
                Ok((swap_chain, device, context))
            }

            // SAFETY: `attempt` receives the validated live HWND descriptor
            // and returns owned COM interfaces. The 11.1 retry follows the
            // D3D11 contract for runtimes that reject that enum value.
            let created = unsafe { attempt(&FEATURE_LEVELS, &desc) }.or_else(|code| {
                if code == E_INVALIDARG {
                    // SAFETY: same validated descriptor and ownership rules
                    // as the first attempt; `std` has no DXGI interface.
                    unsafe { attempt(&FEATURE_LEVELS[1..], &desc) }
                } else {
                    Err(code)
                }
            });
            let (swap_chain, device, context) = created.map_err(|code| {
                format!("D3D11 swap-chain creation failed: 0x{:08x}", code as u32)
            })?;
            Ok(Self {
                swap_chain,
                device,
                context,
                width,
                height,
            })
        }

        pub fn present_bgra(
            &mut self,
            pixels: &[u8],
            width: u32,
            height: u32,
        ) -> Result<(), String> {
            let expected = (width as usize)
                .checked_mul(height as usize)
                .and_then(|count| count.checked_mul(4))
                .ok_or_else(|| "GPU frame dimensions overflow".to_string())?;
            if width == 0 || height == 0 || pixels.len() != expected {
                return Err("GPU frame must be tightly packed non-empty BGRA8".into());
            }
            let row_pitch = width
                .checked_mul(4)
                .ok_or_else(|| "GPU row pitch overflow".to_string())?;

            // SAFETY: all COM pointers are owned live interfaces created by
            // Presenter::new. Vtable layouts and method indices are taken
            // from the installed Windows SDK headers. Source bytes remain
            // live for UpdateSubresource, and the acquired back buffer is
            // released exactly once. `std` cannot update or present DXGI.
            unsafe {
                let swap_vtable = *(self.swap_chain as *mut *const SwapChainVTable);
                if self.width != width || self.height != height {
                    let resized = ((*swap_vtable).resize_buffers)(
                        self.swap_chain,
                        0,
                        width,
                        height,
                        DXGI_FORMAT_B8G8R8A8_UNORM,
                        0,
                    );
                    if resized < 0 {
                        return Err(format!(
                            "DXGI ResizeBuffers failed: 0x{:08x}",
                            resized as u32
                        ));
                    }
                    self.width = width;
                    self.height = height;
                }

                let mut back_buffer = null_mut();
                let get_buffer = ((*swap_vtable).get_buffer)(
                    self.swap_chain,
                    0,
                    &IID_ID3D11_TEXTURE_2D,
                    &mut back_buffer,
                );
                if get_buffer < 0 || back_buffer.is_null() {
                    return Err(format!(
                        "DXGI GetBuffer failed: 0x{:08x}",
                        get_buffer as u32
                    ));
                }
                let context_vtable = *(self.context as *mut *const DeviceContextVTable);
                ((*context_vtable).update_subresource)(
                    self.context,
                    back_buffer,
                    0,
                    std::ptr::null(),
                    pixels.as_ptr().cast(),
                    row_pitch,
                    0,
                );
                release_com(back_buffer);
                let presented = ((*swap_vtable).present)(self.swap_chain, 0, 0);
                if presented < 0 {
                    return Err(format!("DXGI Present failed: 0x{:08x}", presented as u32));
                }
            }
            Ok(())
        }
    }

    impl Drop for Presenter {
        fn drop(&mut self) {
            // SAFETY: these are the three distinct owned COM references
            // returned at construction and Drop runs once. `std` has no COM
            // release operation; swap chain/context go before their device.
            unsafe {
                release_com(self.context);
                release_com(self.swap_chain);
                release_com(self.device);
            }
        }
    }

    pub fn probe() -> Result<GpuInfo, String> {
        let selected = match create(&FEATURE_LEVELS) {
            Ok(level) => level,
            Err(E_INVALIDARG) => create(&FEATURE_LEVELS[1..]).map_err(|code| {
                format!(
                    "D3D11 hardware device creation failed: 0x{:08x}",
                    code as u32
                )
            })?,
            Err(code) => {
                return Err(format!(
                    "D3D11 hardware device creation failed: 0x{:08x}",
                    code as u32
                ));
            }
        };
        Ok(GpuInfo {
            backend: "D3D11 hardware device",
            feature_level: feature_level_name(selected),
        })
    }

    pub fn presentation_smoke() -> Result<PresentInfo, String> {
        let class: Vec<u16> = "STATIC".encode_utf16().chain(std::iter::once(0)).collect();
        let title: Vec<u16> = "Ferric D3D11 smoke"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: both strings are live null-terminated UTF-16 buffers;
        // STATIC is a process-available system class; null parent/menu/
        // instance/parameter are valid for this hidden top-level popup.
        // `std` has no HWND creation operation.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                64,
                64,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if window.is_null() {
            return Err("could not create hidden GPU smoke window".into());
        }
        let result = (|| {
            // SAFETY: `window` is a live HWND created above and remains alive
            // until after Presenter drops. `std` cannot create a DXGI chain.
            let mut presenter = unsafe { Presenter::new(window, 64, 64) }?;
            let mut pixels = Vec::with_capacity(64 * 64 * 4);
            for y in 0..64 {
                for x in 0..64 {
                    let bgra = if (x / 8 + y / 8) % 2 == 0 {
                        [0x20, 0x40, 0xe0, 0xff]
                    } else {
                        [0xe0, 0xc0, 0x20, 0xff]
                    };
                    pixels.extend_from_slice(&bgra);
                }
            }
            presenter.present_bgra(&pixels, 64, 64)?;
            Ok(PresentInfo {
                backend: "D3D11 hardware swap chain",
                width: 64,
                height: 64,
            })
        })();
        // SAFETY: `window` is the owned HWND returned above, all swap-chain
        // references have dropped, and DestroyWindow is called exactly once.
        // `std` has no HWND destruction operation.
        unsafe { DestroyWindow(window) };
        result
    }
}

#[cfg(windows)]
pub use windows::{Presenter, presentation_smoke, probe};
