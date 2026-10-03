//! Renderer-only Linux namespace and seccomp profile. The caller has not yet
//! created IPC worker threads or accepted document input.
use std::ffi::CString;
use std::io;

fn last() -> io::Error {
    io::Error::last_os_error()
}
fn check(value: libc::c_int) -> io::Result<()> {
    if value < 0 {
        Err(last())
    } else {
        Ok(())
    }
}
fn map_id(path: &str, inside: u32, outside: u32) -> io::Result<()> {
    std::fs::write(path, format!("{inside} {outside} 1\n"))
}

fn close_unlisted_fds(frame_fd: i32) -> io::Result<()> {
    if frame_fd < 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid frame fd",
        ));
    }
    let mut fds = Vec::new();
    for entry in std::fs::read_dir("/proc/self/fd")? {
        let entry = entry?;
        if let Ok(fd) = entry.file_name().to_string_lossy().parse::<i32>() {
            if fd > 2 && fd != frame_fd {
                fds.push(fd);
            }
        }
    }
    for fd in fds {
        // SAFETY: `fd` was enumerated after exec and is not stdio or the
        // authenticated frame socket. Closing a stale read_dir FD is harmless.
        unsafe {
            libc::close(fd);
        }
    }
    Ok(())
}

pub fn enter(frame_fd: i32) -> io::Result<()> {
    close_unlisted_fds(frame_fd)?;
    // SAFETY: getuid/getgid read process credentials without pointers.
    let uid = unsafe { libc::getuid() };
    // SAFETY: getgid reads process credentials without pointers.
    let gid = unsafe { libc::getgid() };
    // SAFETY: this single-threaded child creates an unprivileged user namespace.
    check(unsafe { libc::unshare(libc::CLONE_NEWUSER) })?;
    std::fs::write("/proc/self/setgroups", "deny\n")?;
    map_id("/proc/self/uid_map", 0, uid)?;
    map_id("/proc/self/gid_map", 0, gid)?;
    // SAFETY: mapped namespace root has the capabilities to create these
    // private namespaces; failure aborts bootstrap before untrusted input.
    check(unsafe { libc::unshare(libc::CLONE_NEWNS | libc::CLONE_NEWNET) })?;
    let root = CString::new("/").unwrap();
    // SAFETY: root is a live NUL-terminated path. Propagation is changed only
    // in this child's new mount namespace, before mounting scratch storage.
    check(unsafe {
        libc::mount(
            std::ptr::null(),
            root.as_ptr(),
            std::ptr::null(),
            (libc::MS_REC | libc::MS_PRIVATE) as _,
            std::ptr::null(),
        )
    })?;
    let scratch = CString::new("/tmp").unwrap();
    let tmpfs = CString::new("tmpfs").unwrap();
    let options = CString::new("size=64m,mode=0700").unwrap();
    // SAFETY: all C strings are live; the mount is private and disallows
    // execution/devices/setuid. It becomes the child's empty filesystem root.
    check(unsafe {
        libc::mount(
            tmpfs.as_ptr(),
            scratch.as_ptr(),
            tmpfs.as_ptr(),
            (libc::MS_NODEV | libc::MS_NOSUID | libc::MS_NOEXEC) as _,
            options.as_ptr().cast(),
        )
    })?;
    // SAFETY: scratch is a live path within this private mount namespace.
    check(unsafe { libc::chroot(scratch.as_ptr()) })?;
    // SAFETY: root is a live path in the new empty root.
    check(unsafe { libc::chdir(root.as_ptr()) })?;
    // SAFETY: the namespace still has CAP_SETPCAP before capset; drop every
    // bounding capability so no later process can reacquire one.
    for cap in 0..=63 {
        let result = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) };
        if result < 0 && last().raw_os_error() != Some(libc::EINVAL) {
            return Err(last());
        }
    }
    // SAFETY: PR_CAP_AMBIENT_CLEAR_ALL has no pointer arguments.
    check(unsafe {
        libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL,
            0,
            0,
            0,
        )
    })?;
    #[repr(C)]
    struct CapHeader {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    struct CapData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = CapHeader {
        version: 0x2008_0522,
        pid: 0,
    };
    let data = [
        CapData {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
        CapData {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
    ];
    // SAFETY: header and two data records use the Linux v3 capset ABI; both
    // pointers remain live during the synchronous syscall.
    if unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) } < 0 {
        return Err(last());
    }
    // SAFETY: no-new-privs is inherited by worker threads and cannot be unset.
    check(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) })?;
    install_filter()?;
    Ok(())
}

fn stmt(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}
fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

fn install_filter() -> io::Result<()> {
    const LD: u16 = (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16;
    const JEQ: u16 = (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16;
    const RET: u16 = (libc::BPF_RET | libc::BPF_K) as u16;
    const AND: u16 = (libc::BPF_ALU | libc::BPF_AND | libc::BPF_K) as u16;
    const ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
    const DENY: u32 = libc::SECCOMP_RET_ERRNO | (libc::EPERM as u32);
    let mut filters = vec![
        stmt(LD, 4),                  // seccomp_data.arch
        jump(JEQ, 0xc000_003e, 1, 0), // AUDIT_ARCH_X86_64
        stmt(RET, libc::SECCOMP_RET_KILL_PROCESS),
        stmt(LD, 0), // seccomp_data.nr
        // glibc may probe clone3 before falling back to clone. Pretend the
        // newer syscall is absent; allowing it would bypass the clone flags.
        jump(JEQ, libc::SYS_clone3 as u32, 0, 1),
        stmt(RET, libc::SECCOMP_RET_ERRNO | (libc::ENOSYS as u32)),
        // Non-clone syscalls skip the entire five-instruction flags guard,
        // including its deny return, and continue into the explicit allowlist.
        jump(JEQ, libc::SYS_clone as u32, 0, 5),
        stmt(LD, 16), // first clone argument: flags, low 32 bits
        stmt(AND, libc::CLONE_THREAD as u32),
        jump(JEQ, libc::CLONE_THREAD as u32, 0, 1),
        stmt(RET, ALLOW),
        stmt(RET, DENY),
    ];
    // This table is intentionally explicit. The process cannot open a new
    // file, create/connect a socket, fork or exec after installing the filter.
    for syscall in [
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_close,
        libc::SYS_lseek,
        libc::SYS_fstat,
        libc::SYS_newfstatat,
        libc::SYS_mmap,
        libc::SYS_mprotect,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_brk,
        libc::SYS_futex,
        libc::SYS_clock_gettime,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_getrandom,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        libc::SYS_tgkill,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_sched_yield,
        libc::SYS_sched_getaffinity,
        libc::SYS_poll,
        libc::SYS_ppoll,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_wait,
        libc::SYS_epoll_pwait,
        libc::SYS_fcntl,
        libc::SYS_prlimit64,
        libc::SYS_set_robust_list,
        libc::SYS_rseq,
        libc::SYS_memfd_create,
        libc::SYS_ftruncate,
        libc::SYS_sendmsg,
        libc::SYS_recvmsg,
        libc::SYS_getsockopt,
    ] {
        filters.push(jump(JEQ, syscall as u32, 0, 1));
        filters.push(stmt(RET, ALLOW));
    }
    filters.push(stmt(RET, DENY));
    let program = libc::sock_fprog {
        len: filters.len() as u16,
        filter: filters.as_mut_ptr(),
    };
    // SAFETY: program and its filters remain live during the synchronous
    // seccomp call. The caller is single-threaded and has no-new-privs set.
    if unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            0,
            &program,
        )
    } < 0
    {
        return Err(last());
    }
    Ok(())
}
