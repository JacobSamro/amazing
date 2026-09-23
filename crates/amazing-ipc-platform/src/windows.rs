use crate::{Error, FrameMetadata, FORMAT_RGBA8};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command};
use std::ptr::NonNull;
use std::sync::Mutex;
use windows_sys::Win32::Foundation::{CloseHandle, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE,
    MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessId};

struct OwnedHandle(HANDLE);
// SAFETY: HANDLE is an opaque kernel capability, and each instance is uniquely owned.
unsafe impl Send for OwnedHandle {}
// SAFETY: the owned kernel handle is never mutated through shared Rust references.
unsafe impl Sync for OwnedHandle {}
impl OwnedHandle {
    fn new(raw: HANDLE) -> Result<Self, Error> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(Self(raw))
        }
    }
}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this object uniquely owns a valid handle until Drop.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub struct FrameHandle {
    mapping: OwnedHandle,
    metadata: FrameMetadata,
}
impl FrameHandle {
    pub fn new(generation: u64, width: u32, height: u32, rgba: &[u8]) -> Result<Self, Error> {
        let stride = width.checked_mul(4).ok_or(Error::InvalidMetadata)?;
        let mut metadata = FrameMetadata {
            generation,
            width,
            height,
            stride,
            format: FORMAT_RGBA8,
            byte_len: rgba.len() as u64,
            handle_id: 1,
        };
        let len = metadata.validate()?;
        if len != rgba.len() {
            return Err(Error::InvalidMetadata);
        }
        // SAFETY: INVALID_HANDLE_VALUE requests an unnamed page-file mapping; size is bounded.
        let writable = OwnedHandle::new(unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                len as u32,
                std::ptr::null(),
            )
        })?;
        // SAFETY: writable is a live mapping handle of exactly len bytes.
        let view = unsafe { MapViewOfFile(writable.0, FILE_MAP_WRITE, 0, 0, len) };
        if view.Value.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: view is writable for len bytes and rgba has exactly len initialized bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), view.Value.cast::<u8>(), len);
        }
        // SAFETY: view is the exact live mapping returned above and no references escaped.
        if unsafe { UnmapViewOfFile(view) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut readonly: HANDLE = std::ptr::null_mut();
        // SAFETY: both process pseudo-handles and writable mapping are valid; explicit access
        // grants only FILE_MAP_READ to the newly owned mapping handle.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                writable.0,
                GetCurrentProcess(),
                &mut readonly,
                FILE_MAP_READ,
                0,
                0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        let mapping = OwnedHandle::new(readonly)?;
        drop(writable);
        metadata.handle_id = mapping.0 as usize as u64;
        Ok(Self { mapping, metadata })
    }
    pub fn metadata(&self) -> FrameMetadata {
        self.metadata
    }
    pub fn local_view(&self) -> Result<FrameView, Error> {
        let mut raw: HANDLE = std::ptr::null_mut();
        // SAFETY: mapping is a live read-only handle and duplicate grants FILE_MAP_READ only.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                self.mapping.0,
                GetCurrentProcess(),
                &mut raw,
                FILE_MAP_READ,
                0,
                0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        FrameView::from_handle(OwnedHandle::new(raw)?, self.metadata)
    }
    pub fn publish(&self, _: &HandleChannel) -> Result<(), Error> {
        Ok(())
    }
}

pub struct FrameView {
    _mapping: OwnedHandle,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    ptr: NonNull<u8>,
    len: usize,
    metadata: FrameMetadata,
}
// SAFETY: the only writable view and handle were closed before the read-only handle was published.
unsafe impl Send for FrameView {}
// SAFETY: this mapping exposes immutable bytes and only read-only access rights.
unsafe impl Sync for FrameView {}
impl FrameView {
    fn from_handle(mapping: OwnedHandle, metadata: FrameMetadata) -> Result<Self, Error> {
        let len = metadata.validate()?;
        // SAFETY: mapping is an owned FILE_MAP_READ handle; mapping len bytes verifies its type/size.
        let view = unsafe { MapViewOfFile(mapping.0, FILE_MAP_READ, 0, 0, len) };
        let ptr = NonNull::new(view.Value.cast::<u8>()).ok_or_else(io::Error::last_os_error)?;
        Ok(Self {
            _mapping: mapping,
            view,
            ptr,
            len,
            metadata,
        })
    }
    pub fn metadata(&self) -> FrameMetadata {
        self.metadata
    }
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: ptr maps len initialized read-only bytes and Drop cannot run while borrowed.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}
impl Drop for FrameView {
    fn drop(&mut self) {
        // SAFETY: this object uniquely owns the live mapped view.
        unsafe {
            UnmapViewOfFile(self.view);
        }
    }
}

/// On Windows the known Child process handle is the transfer channel: a renderer
/// sends only its local handle value over the authenticated stdio control pipe.
pub struct HandleChannel {
    child: Option<OwnedHandle>,
    child_pid: Option<u32>,
    last_generation: Mutex<u64>,
    _nonce: [u8; 16],
}
impl HandleChannel {
    pub fn nonce_hex(&self) -> String {
        hex(&self._nonce)
    }
    pub fn prepare_child(command: &mut Command) -> Result<Self, Error> {
        let mut nonce = [0; 16];
        // SAFETY: nonce is live writable storage and the system RNG fills all 16 bytes.
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                nonce.as_mut_ptr(),
                nonce.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status < 0 {
            return Err(Error::Authentication);
        }
        command.env("AMAZING_IPC_HANDLE_NONCE", hex(&nonce));
        Ok(Self {
            child: None,
            child_pid: None,
            last_generation: Mutex::new(0),
            _nonce: nonce,
        })
    }
    pub fn from_child_env() -> Result<Self, Error> {
        let nonce =
            unhex(&std::env::var("AMAZING_IPC_HANDLE_NONCE").map_err(|_| Error::Authentication)?)?;
        std::env::remove_var("AMAZING_IPC_HANDLE_NONCE");
        Ok(Self {
            child: None,
            child_pid: None,
            last_generation: Mutex::new(0),
            _nonce: nonce,
        })
    }
    pub fn attach_child(&mut self, child: &Child) -> Result<(), Error> {
        let raw = child.as_raw_handle() as HANDLE;
        let mut owned: HANDLE = std::ptr::null_mut();
        // SAFETY: Child owns raw; duplicating it into this process gives an independent lifetime.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                raw,
                GetCurrentProcess(),
                &mut owned,
                0,
                0,
                windows_sys::Win32::Foundation::DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        let owned = OwnedHandle::new(owned)?;
        // SAFETY: owned is a live process handle; GetProcessId reads its kernel identity.
        let pid = unsafe { GetProcessId(owned.0) };
        if pid == 0 || pid != child.id() {
            return Err(Error::Authentication);
        }
        self.child_pid = Some(pid);
        self.child = Some(owned);
        Ok(())
    }
    pub fn attach_sandbox_child(
        &mut self,
        child: &crate::WindowsSandboxChild,
    ) -> Result<(), Error> {
        let mut owned: HANDLE = std::ptr::null_mut();
        // SAFETY: the platform child owns this live process handle. The duplicate
        // is independent and remains browser-owned for IPC-3 frame acquisition.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                child.process_handle(),
                GetCurrentProcess(),
                &mut owned,
                0,
                0,
                windows_sys::Win32::Foundation::DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        let owned = OwnedHandle::new(owned)?;
        // SAFETY: the duplicate is a process handle created by this launcher.
        if unsafe { GetProcessId(owned.0) } != child.id() {
            return Err(Error::Authentication);
        }
        self.child_pid = Some(child.id());
        self.child = Some(owned);
        Ok(())
    }
    pub fn receive(&self, metadata: FrameMetadata, source_pid: u32) -> Result<FrameView, Error> {
        metadata.validate()?;
        if self.child_pid != Some(source_pid) {
            return Err(Error::Authentication);
        }
        let child = self.child.as_ref().ok_or(Error::Authentication)?;
        let handle =
            usize::try_from(metadata.handle_id).map_err(|_| Error::InvalidMetadata)? as HANDLE;
        let mut last_generation = self.last_generation.lock().unwrap();
        if metadata.generation <= *last_generation {
            return Err(Error::StaleGeneration);
        }
        let mut raw: HANDLE = std::ptr::null_mut();
        // SAFETY: the source process is the launcher-owned Child, never chosen by renderer input;
        // the arbitrary source value is only duplicated with FILE_MAP_READ rights and then probed.
        if unsafe {
            DuplicateHandle(
                child.0,
                handle,
                GetCurrentProcess(),
                &mut raw,
                FILE_MAP_READ,
                0,
                0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        let view = FrameView::from_handle(OwnedHandle::new(raw)?, metadata)?;
        *last_generation = metadata.generation;
        Ok(view)
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> Result<[u8; 16], Error> {
    if s.len() != 32 {
        return Err(Error::Authentication);
    }
    let mut out = [0; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| Error::Authentication)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Threading::GetProcessHandleCount;
    fn count() -> u32 {
        let mut value = 0;
        // SAFETY: GetCurrentProcess returns a valid pseudo-handle and value is writable.
        assert_ne!(
            unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut value) },
            0
        );
        value
    }
    #[test]
    fn readonly_mapping_releases_all_handles() {
        let before = count();
        let rgba = vec![0x7b; 64 * 64 * 4];
        for generation in 1..=100 {
            let handle = FrameHandle::new(generation, 64, 64, &rgba).unwrap();
            let view = handle.local_view().unwrap();
            assert_eq!(view.bytes(), rgba);
            assert_eq!(view.metadata(), handle.metadata());
        }
        let after = count();
        assert!(
            after <= before + 2,
            "handles: before={before}, after={after}"
        );
    }
}
