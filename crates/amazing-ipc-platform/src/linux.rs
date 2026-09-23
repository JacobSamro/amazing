use crate::{Error, FrameMetadata, FORMAT_RGBA8};
use std::fs::File;
use std::io::{self, Read};
use std::mem::{size_of, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::ptr::NonNull;
use std::sync::Mutex;
use std::time::Duration;

pub struct FrameHandle {
    fd: OwnedFd,
    metadata: FrameMetadata,
}
impl FrameHandle {
    pub fn new(generation: u64, width: u32, height: u32, rgba: &[u8]) -> Result<Self, Error> {
        let stride = width.checked_mul(4).ok_or(Error::InvalidMetadata)?;
        let metadata = FrameMetadata {
            generation,
            width,
            height,
            stride,
            format: FORMAT_RGBA8,
            byte_len: rgba.len() as u64,
            handle_id: generation,
        };
        let len = metadata.validate()?;
        if len != rgba.len() {
            return Err(Error::InvalidMetadata);
        }
        // SAFETY: name is NUL terminated; a successful return is a new owned descriptor.
        let raw = unsafe {
            libc::memfd_create(
                c"amazing-frame".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: memfd_create returned this descriptor exclusively to this call.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: fd is live; len is positive and bounded to 256 MiB.
        if unsafe { libc::ftruncate(raw, len as libc::off_t) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: the live memfd has len bytes and this writable mapping is private to construction.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                raw,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: ptr is writable for len bytes and rgba is exactly len initialized bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), ptr.cast::<u8>(), len);
        }
        // SAFETY: ptr and len are the mapping returned by mmap; no reference escaped.
        if unsafe { libc::munmap(ptr, len) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let seals =
            libc::F_SEAL_WRITE | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
        // SAFETY: all writable views were removed; sealing makes subsequent publication immutable.
        if unsafe { libc::fcntl(raw, libc::F_ADD_SEALS, seals) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self { fd, metadata })
    }
    pub fn metadata(&self) -> FrameMetadata {
        self.metadata
    }
    pub fn local_view(&self) -> Result<FrameView, Error> {
        // SAFETY: fd is live and dup creates a separate owned descriptor.
        let raw = unsafe { libc::dup(self.fd.as_raw_fd()) };
        if raw < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: dup returned this descriptor exclusively to this call.
        FrameView::from_fd(unsafe { OwnedFd::from_raw_fd(raw) }, self.metadata)
    }
    pub fn publish(&self, channel: &HandleChannel) -> Result<(), Error> {
        channel.send(self)
    }
}

pub struct FrameView {
    _fd: OwnedFd,
    ptr: NonNull<u8>,
    len: usize,
    metadata: FrameMetadata,
}
// SAFETY: the mapping is read-only and the memfd is sealed against writes and resize.
unsafe impl Send for FrameView {}
// SAFETY: concurrent reads of the same sealed read-only mapping are safe.
unsafe impl Sync for FrameView {}
impl FrameView {
    fn from_fd(fd: OwnedFd, metadata: FrameMetadata) -> Result<Self, Error> {
        let len = metadata.validate()?;
        // SAFETY: zeroed stat is valid output storage for fstat.
        let mut st: libc::stat = unsafe { zeroed() };
        // SAFETY: fd is live; st is writable initialized storage.
        if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        if st.st_size != len as libc::off_t {
            return Err(Error::InvalidMetadata);
        }
        // SAFETY: fd is live; fcntl reads only seal flags.
        let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
        if seals < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let required =
            libc::F_SEAL_WRITE | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
        if seals & required != required {
            return Err(Error::Authentication);
        }
        // SAFETY: exact size and all immutability seals were checked before mapping.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error().into());
        }
        let ptr = NonNull::new(ptr.cast::<u8>()).expect("successful mmap is non-null");
        Ok(Self {
            _fd: fd,
            ptr,
            len,
            metadata,
        })
    }
    pub fn metadata(&self) -> FrameMetadata {
        self.metadata
    }
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: ptr maps len initialized immutable bytes and Drop cannot run while borrowed.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}
impl Drop for FrameView {
    fn drop(&mut self) {
        // SAFETY: this object owns exactly the live ptr/len mapping returned by mmap.
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast(), self.len);
        }
    }
}

pub struct HandleChannel {
    socket: UnixStream,
    nonce: [u8; 16],
    expected_pid: Option<u32>,
    last_generation: Mutex<u64>,
}
impl HandleChannel {
    /// The only role-specific descriptor retained by the renderer sandbox.
    pub fn sandbox_keep_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
    pub fn nonce_hex(&self) -> String {
        hex(&self.nonce)
    }
    fn pair() -> Result<(Self, Self), Error> {
        let mut fds = [-1, -1];
        // SAFETY: fds is writable storage for exactly two descriptors.
        if unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                0,
                fds.as_mut_ptr(),
            )
        } != 0
        {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: socketpair returned two distinct owned descriptors.
        let left = unsafe { UnixStream::from_raw_fd(fds[0]) };
        // SAFETY: socketpair returned this second owned descriptor.
        let right = unsafe { UnixStream::from_raw_fd(fds[1]) };
        let mut nonce = [0; 16];
        File::open("/dev/urandom")?.read_exact(&mut nonce)?;
        Ok((
            Self {
                socket: left,
                nonce,
                expected_pid: None,
                last_generation: Mutex::new(0),
            },
            Self {
                socket: right,
                nonce,
                expected_pid: None,
                last_generation: Mutex::new(0),
            },
        ))
    }
    pub fn pair_for_test() -> Result<(Self, Self), Error> {
        Self::pair()
    }
    pub fn prepare_child(command: &mut Command) -> Result<Self, Error> {
        let (parent, child) = Self::pair()?;
        let fd = child.socket.as_raw_fd();
        command.env("AMAZING_IPC_HANDLE_FD", fd.to_string());
        command.env("AMAZING_IPC_HANDLE_NONCE", hex(&child.nonce));
        // SAFETY: pre_exec performs only async-signal-safe fcntl on a descriptor kept live by the closure.
        unsafe {
            command.pre_exec(move || {
                let fd = child.socket.as_raw_fd();
                // SAFETY: fd is live in the child and fcntl changes only its close-on-exec flag.
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(parent)
    }
    pub fn attach_child(&mut self, child: &Child) -> Result<(), Error> {
        self.expected_pid = Some(child.id());
        self.socket.set_read_timeout(Some(Duration::from_secs(5)))?;
        let enabled: libc::c_int = 1;
        // SAFETY: socket is live and enabled points to a valid integer option value.
        if unsafe {
            libc::setsockopt(
                self.socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const libc::c_int).cast(),
                size_of::<libc::c_int>() as libc::socklen_t,
            )
        } != 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    pub fn from_child_env() -> Result<Self, Error> {
        let fd: RawFd = std::env::var("AMAZING_IPC_HANDLE_FD")
            .map_err(|_| Error::Authentication)?
            .parse()
            .map_err(|_| Error::Authentication)?;
        let nonce =
            unhex(&std::env::var("AMAZING_IPC_HANDLE_NONCE").map_err(|_| Error::Authentication)?)?;
        std::env::remove_var("AMAZING_IPC_HANDLE_FD");
        std::env::remove_var("AMAZING_IPC_HANDLE_NONCE");
        if fd < 3 {
            return Err(Error::Authentication);
        }
        let mut kind: libc::c_int = 0;
        let mut len = size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: getsockopt only writes to kind/len; this checks the inherited fd before adoption.
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                (&mut kind as *mut libc::c_int).cast(),
                &mut len,
            )
        } != 0
            || kind != libc::SOCK_SEQPACKET
        {
            return Err(Error::Authentication);
        }
        // SAFETY: the bootstrap transfers this inherited socket descriptor to this child once.
        let socket = unsafe { UnixStream::from_raw_fd(fd) };
        Ok(Self {
            socket,
            nonce,
            expected_pid: None,
            last_generation: Mutex::new(0),
        })
    }
    fn send(&self, frame: &FrameHandle) -> Result<(), Error> {
        let mut data = [0; 24];
        data[..16].copy_from_slice(&self.nonce);
        data[16..].copy_from_slice(&frame.metadata.handle_id.to_le_bytes());
        let mut iov = libc::iovec {
            iov_base: data.as_mut_ptr().cast(),
            iov_len: data.len(),
        };
        // SAFETY: zero is a valid initial msghdr value; all used pointers are assigned below.
        let mut msg: libc::msghdr = unsafe { zeroed() };
        // SAFETY: CMSG_SPACE computes storage size for one RawFd.
        let mut control =
            vec![0u8; unsafe { libc::CMSG_SPACE(size_of::<RawFd>() as u32) } as usize];
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = control.len();
        // SAFETY: the control buffer is CMSG_SPACE sized; the header and fd fit it.
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&msg);
            if header.is_null() {
                return Err(Error::InvalidMetadata);
            }
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(size_of::<RawFd>() as u32) as _;
            std::ptr::write_unaligned(
                libc::CMSG_DATA(header).cast::<RawFd>(),
                frame.fd.as_raw_fd(),
            );
            loop {
                let sent = libc::sendmsg(self.socket.as_raw_fd(), &msg, libc::MSG_NOSIGNAL);
                if sent == data.len() as isize {
                    return Ok(());
                }
                if sent >= 0 {
                    return Err(Error::InvalidMetadata);
                }
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::Interrupted {
                    return Err(err.into());
                }
            }
        }
    }
    pub fn receive(&self, metadata: FrameMetadata, source_pid: u32) -> Result<FrameView, Error> {
        metadata.validate()?;
        if self.expected_pid != Some(source_pid) {
            return Err(Error::Authentication);
        }
        let mut last_generation = self.last_generation.lock().unwrap();
        if metadata.generation <= *last_generation {
            return Err(Error::StaleGeneration);
        }
        let mut data = [0; 24];
        let mut iov = libc::iovec {
            iov_base: data.as_mut_ptr().cast(),
            iov_len: data.len(),
        };
        // SAFETY: zero is valid initial msghdr; all used pointers are assigned below.
        let mut msg: libc::msghdr = unsafe { zeroed() };
        // SAFETY: buffer holds credentials and up to four descriptors; excess triggers MSG_CTRUNC.
        let control_len = unsafe {
            libc::CMSG_SPACE((4 * size_of::<RawFd>()) as u32)
                + libc::CMSG_SPACE(size_of::<libc::ucred>() as u32)
        } as usize;
        let mut control = vec![0u8; control_len];
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = control.len();
        let read = loop {
            // SAFETY: msg references live writable data/control buffers, and received fds get CLOEXEC.
            let result =
                unsafe { libc::recvmsg(self.socket.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
            if result >= 0 {
                break result;
            }
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                return Err(err.into());
            }
        };
        let mut received = Vec::<OwnedFd>::new();
        let mut sender_pid = None;
        let mut invalid_control = false;
        // SAFETY: libc CMSG traversal uses msg_controllen supplied by the kernel; each payload
        // is bounds-checked against the control buffer before reading or adopting descriptors.
        unsafe {
            let mut header = libc::CMSG_FIRSTHDR(&msg);
            while !header.is_null() {
                let start = (header as usize).saturating_sub(control.as_ptr() as usize);
                let header_len = libc::CMSG_LEN(0) as usize;
                let n = (*header).cmsg_len as usize;
                if start > control.len() || n < header_len || n > control.len() - start {
                    invalid_control = true;
                    break;
                }
                let payload = libc::CMSG_DATA(header).cast::<u8>();
                let payload_len = n - header_len;
                if (*header).cmsg_level == libc::SOL_SOCKET
                    && (*header).cmsg_type == libc::SCM_RIGHTS
                {
                    if payload_len % size_of::<RawFd>() != 0 {
                        invalid_control = true;
                    } else {
                        for i in 0..payload_len / size_of::<RawFd>() {
                            let raw = std::ptr::read_unaligned(
                                payload.add(i * size_of::<RawFd>()).cast::<RawFd>(),
                            );
                            if raw >= 0 {
                                received.push(OwnedFd::from_raw_fd(raw));
                            }
                        }
                    }
                } else if (*header).cmsg_level == libc::SOL_SOCKET
                    && (*header).cmsg_type == libc::SCM_CREDENTIALS
                {
                    if payload_len != size_of::<libc::ucred>() {
                        invalid_control = true;
                    } else {
                        sender_pid = Some(
                            std::ptr::read_unaligned(payload.cast::<libc::ucred>()).pid as u32,
                        );
                    }
                } else {
                    invalid_control = true;
                }
                header = libc::CMSG_NXTHDR(&msg, header);
            }
        }
        if invalid_control
            || read != data.len() as isize
            || msg.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0
            || sender_pid != Some(source_pid)
            || received.len() != 1
            || data[..16] != self.nonce
            || data[16..] != metadata.handle_id.to_le_bytes()
        {
            return Err(Error::Authentication);
        }
        let view = FrameView::from_fd(received.pop().expect("one fd checked"), metadata)?;
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
    #[test]
    fn sealed_mapping_rejects_forgery_and_closes_descriptors() {
        let before = std::fs::read_dir("/proc/self/fd").unwrap().count();
        let rgba = vec![0x7b; 64 * 64 * 4];
        for generation in 1..=100 {
            let handle = FrameHandle::new(generation, 64, 64, &rgba).unwrap();
            let view = handle.local_view().unwrap();
            assert_eq!(view.bytes(), rgba);
            let mut forged = handle.metadata();
            forged.byte_len -= 4;
            assert!(matches!(forged.validate(), Err(Error::InvalidMetadata)));
            // SAFETY: handle owns a live fd; dup returns a distinct descriptor.
            let raw = unsafe { libc::dup(handle.fd.as_raw_fd()) };
            assert!(raw >= 0);
            // SAFETY: dup returned this descriptor uniquely to the test.
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            assert!(matches!(
                FrameView::from_fd(fd, forged),
                Err(Error::InvalidMetadata)
            ));
        }
        let after = std::fs::read_dir("/proc/self/fd").unwrap().count();
        assert!(
            after <= before + 1,
            "descriptors: before={before}, after={after}"
        );
    }
}
