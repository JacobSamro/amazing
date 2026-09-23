//! Windows renderer launch boundary. All raw handles and process attributes stay here.
use std::ffi::{c_void, OsStr, OsString};
use std::fs::File;
use std::io;
use std::mem::{size_of, ManuallyDrop};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, SetHandleInformation, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, GetAppContainerFolderPath,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenIsAppContainer, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES,
    TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess,
    InitializeProcThreadAttributeList, OpenProcess, OpenProcessToken, ResumeThread,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_SUSPENDED,
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_DUP_HANDLE,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    GetProcessHandleCount,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
fn wide_str(value: &str) -> Vec<u16> {
    wide(OsStr::new(value))
}
fn last() -> io::Error {
    io::Error::last_os_error()
}
fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(last())
    } else {
        Ok(())
    }
}
fn stage(name: &'static str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{name}: {error}"))
}
struct Owned(HANDLE);
impl Owned {
    fn new(handle: HANDLE) -> io::Result<Self> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(last())
        } else {
            Ok(Self(handle))
        }
    }
    fn into_file(self) -> File {
        let raw = ManuallyDrop::new(self).0;
        // SAFETY: ownership of this one valid HANDLE moves to File; Owned will not drop it.
        unsafe { File::from_raw_handle(raw) }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: this object uniquely owns a valid kernel handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Profile {
    name: Vec<u16>,
    sid: *mut c_void,
    executable: PathBuf,
}
impl Profile {
    fn create(nonce: &str, source: &Path) -> io::Result<Self> {
        if nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid renderer nonce",
            ));
        }
        let name = wide_str(&format!("Amazing.Renderer.{nonce}"));
        let mut sid = null_mut();
        // SAFETY: name is NUL-terminated; this new per-launch profile has no capabilities.
        let hr = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        if hr < 0 {
            return Err(io::Error::from_raw_os_error(hr));
        }
        let mut profile = Self {
            name,
            sid,
            executable: PathBuf::new(),
        };
        let mut sid_text = null_mut();
        // SAFETY: profile owns the SID; Windows allocates sid_text for the caller.
        check(unsafe { ConvertSidToStringSidW(profile.sid, &mut sid_text) })?;
        let mut folder = null_mut();
        // SAFETY: sid_text is a live, terminated SID string.
        let hr = unsafe { GetAppContainerFolderPath(sid_text, &mut folder) };
        // SAFETY: LocalFree releases the buffer returned by ConvertSidToStringSidW.
        unsafe {
            LocalFree(sid_text.cast());
        }
        if hr < 0 {
            return Err(io::Error::from_raw_os_error(hr));
        }
        let folder_path = unsafe { path_from_pwstr(folder) };
        // SAFETY: GetAppContainerFolderPath allocates folder with CoTaskMemAlloc.
        unsafe {
            windows_sys::Win32::System::Com::CoTaskMemFree(folder.cast());
        }
        profile.executable = folder_path.join("amazing-renderer.exe");
        std::fs::copy(source, &profile.executable)?;
        Ok(profile)
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        if !self.executable.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.executable);
        }
        // SAFETY: CreateAppContainerProfile returned this SID and profile name.
        unsafe {
            LocalFree(self.sid);
            DeleteAppContainerProfile(self.name.as_ptr());
        }
    }
}
unsafe fn path_from_pwstr(mut p: *const u16) -> PathBuf {
    let start = p;
    while *p != 0 {
        p = p.add(1);
    }
    PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(
        start,
        p.offset_from(start) as usize,
    )))
}

struct AttributeList {
    storage: Vec<usize>,
    initialized: bool,
}
impl AttributeList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: the first call queries the required allocation size.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size);
        }
        if size == 0 {
            return Err(last());
        }
        let mut list = Self {
            storage: vec![0; size.div_ceil(size_of::<usize>())],
            initialized: false,
        };
        // SAFETY: storage is pointer-aligned and remains fixed for the list lifetime.
        check(unsafe { InitializeProcThreadAttributeList(list.ptr(), count, 0, &mut size) })?;
        list.initialized = true;
        Ok(list)
    }
    fn ptr(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast()
    }
    fn set<T>(&mut self, kind: u32, value: &T) -> io::Result<()> {
        // SAFETY: value and the initialized list stay live through CreateProcessW.
        check(unsafe {
            UpdateProcThreadAttribute(
                self.ptr(),
                0,
                kind as usize,
                (value as *const T).cast(),
                size_of::<T>(),
                null_mut(),
                null(),
            )
        })
    }
    fn set_handles(&mut self, kind: u32, values: &[HANDLE]) -> io::Result<()> {
        // SAFETY: the slice and the initialized list stay live through CreateProcessW.
        check(unsafe {
            UpdateProcThreadAttribute(
                self.ptr(),
                0,
                kind as usize,
                values.as_ptr().cast(),
                size_of_val(values),
                null_mut(),
                null(),
            )
        })
    }
}
impl Drop for AttributeList {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: the list was successfully initialized in this allocation.
            unsafe {
                DeleteProcThreadAttributeList(self.ptr());
            }
        }
    }
}

fn pipe() -> io::Result<(Owned, Owned)> {
    let mut read = null_mut();
    let mut write = null_mut();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: writable output pointers and valid inheritable attributes.
    check(unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) })?;
    Ok((Owned::new(read)?, Owned::new(write)?))
}
fn no_inherit(handle: HANDLE) -> io::Result<()> {
    // SAFETY: handle is live; only its inheritance bit is cleared.
    check(unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) })
}
fn job() -> io::Result<Owned> {
    // SAFETY: unnamed job; no caller-supplied security descriptor.
    let job = Owned::new(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    // SAFETY: job and limits are live; structure size matches the information class.
    check(unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of_val(&limits) as u32,
        )
    })?;
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_EXITWINDOWS,
    };
    // SAFETY: UI restrictions are installed before the child joins the job.
    check(unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectBasicUIRestrictions,
            (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
            size_of_val(&ui) as u32,
        )
    })?;
    Ok(job)
}

pub struct WindowsSandboxChild {
    process: Owned,
    _job: Owned,
    _profile: Profile,
    stdin: Option<File>,
    stdout: Option<File>,
    pid: u32,
}
impl WindowsSandboxChild {
    pub fn id(&self) -> u32 {
        self.pid
    }
    pub fn take_stdin(&mut self) -> Option<File> {
        self.stdin.take()
    }
    pub fn take_stdout(&mut self) -> Option<File> {
        self.stdout.take()
    }
    pub(crate) fn process_handle(&self) -> HANDLE {
        self.process.0
    }
    pub fn try_wait(&mut self) -> io::Result<bool> {
        // SAFETY: the process handle remains owned by self.
        match unsafe { WaitForSingleObject(self.process.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(last()),
        }
    }
    pub fn wait(&mut self) -> io::Result<()> {
        // SAFETY: the process handle remains owned by self.
        if unsafe { WaitForSingleObject(self.process.0, INFINITE) } == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(last())
        }
    }
    pub fn kill(&mut self) -> io::Result<()> {
        if self.try_wait()? {
            return Ok(());
        }
        // SAFETY: only this launcher-owned process is terminated; the job also kills on close.
        check(unsafe { TerminateProcess(self.process.0, 1) })
    }
}
impl Drop for WindowsSandboxChild {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
    }
}

pub fn spawn_renderer_appcontainer(exe: &Path, nonce: &str) -> io::Result<WindowsSandboxChild> {
    let profile = Profile::create(nonce, exe).map_err(|e| stage("AppContainer profile", e))?;
    let job = job().map_err(|e| stage("job", e))?;
    let (child_stdin, parent_stdin) = pipe()?;
    let (parent_stdout, child_stdout) = pipe()?;
    no_inherit(parent_stdin.0)?;
    no_inherit(parent_stdout.0)?;
    let nul = wide_str("NUL");
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: NUL is a terminated device path; this inheritable sink receives child stderr.
    let stderr = Owned::new(unsafe {
        CreateFileW(
            nul.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &attributes,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    })?;
    let handles = [child_stdin.0, child_stdout.0, stderr.0];
    let jobs = [job.0];
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: profile.sid,
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let mut list = AttributeList::new(3)?;
    list.set_handles(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &handles)?;
    list.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?;
    list.set_handles(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = child_stdin.0;
    startup.StartupInfo.hStdOutput = child_stdout.0;
    startup.StartupInfo.hStdError = stderr.0;
    startup.lpAttributeList = list.ptr();
    let path = wide(profile.executable.as_os_str());
    let mut command = wide_str(&format!(
        "\"{}\" --type=renderer",
        profile.executable.display()
    ));
    let root = std::env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SystemRoot missing"))?;
    let env_items = [
        ("AMAZING_IPC_HANDLE_NONCE", OsString::from(nonce)),
        ("AMAZING_RENDERER_SANDBOX", OsString::from("1")),
        (
            "LOCALAPPDATA",
            profile.executable.parent().unwrap().as_os_str().to_owned(),
        ),
        ("SystemRoot", root),
        (
            "TEMP",
            profile.executable.parent().unwrap().as_os_str().to_owned(),
        ),
        (
            "TMP",
            profile.executable.parent().unwrap().as_os_str().to_owned(),
        ),
    ];
    let mut environment = Vec::new();
    for (key, value) in env_items {
        environment.extend(OsStr::new(key).encode_wide());
        environment.push(b'=' as u16);
        environment.extend(value.encode_wide());
        environment.push(0);
    }
    environment.push(0);
    let cwd = wide(profile.executable.parent().unwrap().as_os_str());
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: every pointer refers to live storage; only the three named pipe/device handles
    // are inheritable, and the job/AppContainer attributes are installed before first resume.
    check(unsafe {
        CreateProcessW(
            path.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            (&startup as *const STARTUPINFOEXW).cast(),
            &mut info,
        )
    })
    .map_err(|e| stage("CreateProcessW", e))?;
    let process = Owned::new(info.hProcess)?;
    let thread = Owned::new(info.hThread)?;
    // SAFETY: thread belongs to the suspended child already in its job and AppContainer.
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        // SAFETY: this process is still our suspended child and must not survive launch failure.
        unsafe {
            TerminateProcess(process.0, 1);
        }
        return Err(last());
    }
    Ok(WindowsSandboxChild {
        process,
        _job: job,
        _profile: profile,
        stdin: Some(parent_stdin.into_file()),
        stdout: Some(parent_stdout.into_file()),
        pid: info.dwProcessId,
    })
}

pub fn verify_current_appcontainer() -> io::Result<()> {
    let mut raw = null_mut();
    // SAFETY: current process pseudo-handle is valid and raw is writable.
    check(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) })?;
    let token = Owned::new(raw)?;
    let mut is_container = 0u32;
    let mut length = 0;
    // SAFETY: token and output storage are live; the class returns a DWORD.
    check(unsafe {
        GetTokenInformation(
            token.0,
            TokenIsAppContainer,
            (&mut is_container as *mut u32).cast(),
            size_of::<u32>() as u32,
            &mut length,
        )
    })?;
    if is_container == 1 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "renderer not in AppContainer",
        ))
    }
}

/// Negative-test probe for the browser process capability.
pub fn probe_open_process(pid: u32) -> io::Result<()> {
    // SAFETY: pid is a kernel lookup key; the desired rights are explicit.
    let handle = Owned::new(unsafe {
        OpenProcess(
            PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    })?;
    drop(handle);
    Ok(())
}

/// Diagnostic count for long-running process and mapping leak tests.
pub fn current_process_handle_count() -> io::Result<u32> {
    let mut count = 0;
    // SAFETY: current-process pseudo-handle and output pointer are valid.
    check(unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) })?;
    Ok(count)
}
