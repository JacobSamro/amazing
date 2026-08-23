//! Windows hardware-acceleration capability seam.
//!
//! The engine remains platform-independent and software-rasterized. This
//! module only proves that the Windows shell can acquire a real D3D11
//! hardware device, while keeping COM and raw ABI operations isolated.

pub struct GpuInfo {
    pub backend: &'static str,
    pub feature_level: &'static str,
}

#[cfg(not(windows))]
pub fn probe() -> Result<GpuInfo, String> {
    Err("this build has no native GPU adapter".into())
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::ptr::null_mut;

    use super::GpuInfo;

    const D3D_DRIVER_TYPE_HARDWARE: u32 = 1;
    const D3D11_CREATE_DEVICE_BGRA_SUPPORT: u32 = 0x20;
    const D3D11_SDK_VERSION: u32 = 7;
    const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
    const FEATURE_LEVELS: [u32; 4] = [0xb100, 0xb000, 0xa100, 0xa000];

    #[repr(C)]
    struct IUnknownVTable {
        query_interface: *const c_void,
        add_ref: *const c_void,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
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

    pub fn probe() -> Result<GpuInfo, String> {
        let selected = match create(&FEATURE_LEVELS) {
            Ok(level) => level,
            Err(E_INVALIDARG) => create(&FEATURE_LEVELS[1..])
                .map_err(|code| format!("D3D11 hardware device creation failed: 0x{:08x}", code as u32))?,
            Err(code) => {
                return Err(format!(
                    "D3D11 hardware device creation failed: 0x{:08x}",
                    code as u32
                ));
            },
        };
        Ok(GpuInfo {
            backend: "D3D11 hardware device",
            feature_level: feature_level_name(selected),
        })
    }
}

#[cfg(windows)]
pub use windows::probe;
