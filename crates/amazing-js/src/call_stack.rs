// Stack-bound discovery and current-frame measurement adapted from rust-lang/stacker
// (src/lib.rs, src/backends/{unix,macos,windows}.rs)
// @ d3ac2df79a409c681329006e9b31c96ab9c5fcae, MIT, Copyright 2014 Alex Crichton.
// GNU x86_64 pthread_attr_t layout adapted from rust-lang/libc
// (src/unix/linux_like/linux/gnu/b64/x86_64/mod.rs)
// @ b739c733e06a5f184bae7e296919c15d6ccd8ec8, MIT, Copyright The Rust Project Developers.
// No stack switching, additional thread, crate dependency or Send implementation.

#[derive(Clone, Copy)]
struct Bounds { low: usize, high: usize }
impl Bounds {
    fn new(low: usize, high: usize) -> Option<Self> {
        (low != 0 && low < high).then_some(Self { low, high })
    }
    fn remaining(self, marker: usize) -> Option<usize> {
        (self.low < marker && marker < self.high).then(|| marker - self.low)
    }
}

std::thread_local! {
    // An interpreter can execute on a serialized coroutine worker. Bounds
    // belong to that actual OS thread, not to the interpreter or its realm.
    static BOUNDS: Option<Bounds> = platform::bounds();
}

#[inline(never)]
pub(crate) fn remaining() -> Option<usize> {
    let mut marker = std::mem::MaybeUninit::<u8>::uninit();
    // SAFETY: this writes exactly one byte to valid local storage. The
    // volatile write prevents the marker becoming a static constant.
    unsafe { marker.as_mut_ptr().write_volatile(0); }
    let address = marker.as_ptr() as usize;
    BOUNDS.with(|bounds| bounds.and_then(|bounds| bounds.remaining(address)))
}

pub(crate) fn initialize() { let _ = remaining(); }

#[cfg(test)]
pub(crate) fn usable_bytes() -> Option<usize> {
    BOUNDS.with(|bounds| bounds.map(|bounds| bounds.high - bounds.low))
}

#[cfg(all(target_os = "linux", target_env = "gnu", target_arch = "x86_64", target_pointer_width = "64"))]
mod platform {
    use super::Bounds;
    use std::ffi::{c_int, c_void};
    #[repr(C)]
    struct Attr { words: [u64; 7] }
    unsafe extern "C" {
        fn pthread_self() -> usize;
        fn pthread_attr_init(attr: *mut Attr) -> c_int;
        fn pthread_getattr_np(thread: usize, attr: *mut Attr) -> c_int;
        fn pthread_attr_getstack(attr: *const Attr, address: *mut *mut c_void, size: *mut usize) -> c_int;
        fn pthread_attr_getguardsize(attr: *const Attr, size: *mut usize) -> c_int;
        fn pthread_attr_destroy(attr: *mut Attr) -> c_int;
    }
    pub(super) fn bounds() -> Option<Bounds> {
        let mut storage = std::mem::MaybeUninit::<Attr>::uninit();
        // SAFETY: this binding is restricted to the verified GNU x86_64 ABI.
        // Every successful initialization is destroyed, including query errors.
        unsafe {
            if pthread_attr_init(storage.as_mut_ptr()) != 0 { return None; }
            let attr = storage.as_mut_ptr();
            let mut address = std::ptr::null_mut();
            let mut size = 0;
            let mut guard = 0;
            let valid = pthread_getattr_np(pthread_self(), attr) == 0
                && pthread_attr_getstack(attr, &mut address, &mut size) == 0
                && pthread_attr_getguardsize(attr, &mut guard) == 0;
            let destroyed = pthread_attr_destroy(attr) == 0;
            if !valid || !destroyed { return None; }
            let low = (address as usize).checked_add(guard)?;
            let high = (address as usize).checked_add(size)?;
            Bounds::new(low, high)
        }
    }
}

#[cfg(all(target_os = "macos", target_pointer_width = "64"))]
mod platform {
    use super::Bounds;
    use std::ffi::c_void;
    unsafe extern "C" {
        fn pthread_self() -> *mut c_void;
        fn pthread_get_stackaddr_np(thread: *mut c_void) -> *mut c_void;
        fn pthread_get_stacksize_np(thread: *mut c_void) -> usize;
    }
    pub(super) fn bounds() -> Option<Bounds> {
        // SAFETY: these APIs only inspect the current pthread's allocation.
        unsafe {
            let thread = pthread_self();
            let high = pthread_get_stackaddr_np(thread) as usize;
            Bounds::new(high.checked_sub(pthread_get_stacksize_np(thread))?, high)
        }
    }
}

#[cfg(all(target_os = "windows", target_pointer_width = "64"))]
mod platform {
    use super::Bounds;
    use std::ffi::c_void;
    #[repr(C)]
    struct MemoryInfo {
        base: *mut c_void, allocation: *mut c_void, allocation_protect: u32,
        partition: u16, region_size: usize, state: u32, protect: u32, kind: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualQuery(address: *const c_void, info: *mut MemoryInfo, bytes: usize) -> usize;
        fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
        fn SetThreadStackGuarantee(bytes: *mut u32) -> i32;
    }
    pub(super) fn bounds() -> Option<Bounds> {
        let marker = 0u8;
        let mut info = std::mem::MaybeUninit::<MemoryInfo>::uninit();
        let mut low = 0; let mut high = 0; let mut guarantee = 0;
        // SAFETY: the buffers match the 64-bit Windows ABI and are read only
        // after their successful system calls. VirtualQuery identifies the
        // reserved allocation, rather than only the currently committed pages.
        unsafe {
            if VirtualQuery(&marker as *const u8 as *const c_void, info.as_mut_ptr(), std::mem::size_of::<MemoryInfo>()) == 0 { return None; }
            GetCurrentThreadStackLimits(&mut low, &mut high);
            if SetThreadStackGuarantee(&mut guarantee) == 0 { return None; }
            let reserve = usize::max(guarantee as usize, 0x2000).checked_add(0x2000)?;
            let allocation = info.assume_init().allocation as usize;
            Bounds::new(allocation.checked_add(reserve)?, high)
        }
    }
}

#[cfg(not(any(
    all(target_os = "linux", target_env = "gnu", target_arch = "x86_64", target_pointer_width = "64"),
    all(target_os = "macos", target_pointer_width = "64"),
    all(target_os = "windows", target_pointer_width = "64")
)))]
mod platform {
    pub(super) fn bounds() -> Option<super::Bounds> { None }
}
