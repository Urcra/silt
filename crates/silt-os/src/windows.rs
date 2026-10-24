use std::ffi::{OsStr, c_void};
use std::fs;
use std::io::{self, Write as _};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::MetadataExt as _;
use std::os::windows::process::CommandExt as _;
use std::path::{Component, Path, Prefix};
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LocalFree,
    SetHandleInformation,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, GetDriveTypeW};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
use windows_sys::Win32::System::Diagnostics::Debug::{DebugActiveProcess, DebugActiveProcessStop, DebugSetProcessKillOnExit};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, GetCurrentProcess, OpenProcess, OpenProcessToken,
    PROCESS_TERMINATE, TerminateProcess,
};

const DRIVE_REMOTE: u32 = 4;

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

/// Memory Win32 allocated with `LocalAlloc`, freed when dropped.
struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: the pointer came from a Win32 call that allocates with
        // `LocalAlloc`, and nothing else frees it.
        unsafe { LocalFree(self.0) };
    }
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was opened for this value alone.
        unsafe { CloseHandle(self.0) };
    }
}

/// The string SID of the user whose token `process` runs with.
fn token_user(process: HANDLE) -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `token` is written on success, and closed by `Handle`.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);
    let mut len = 0u32;
    // SAFETY: a null buffer of size 0 asks only for the size needed.
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut len) };
    let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buffer` holds `len` bytes, aligned for `TOKEN_USER`.
    if unsafe { GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr().cast(), len, &mut len) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success the buffer starts with a `TOKEN_USER`.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut text = ptr::null_mut();
    // SAFETY: the SID points into `buffer`, alive here; `text` is freed by `Local`.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _free = Local(text.cast());
    // SAFETY: `text` is a NUL-terminated wide string.
    let len = (0..).take_while(|&index| unsafe { *text.add(index) } != 0).count();
    // SAFETY: the `len` units before the NUL are initialised.
    Ok(String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) }))
}

fn current_user() -> io::Result<String> {
    // SAFETY: the pseudo-handle needs no closing.
    token_user(unsafe { GetCurrentProcess() })
}

/// A security descriptor from SDDL.
fn descriptor(sddl: &str) -> io::Result<Local> {
    let sddl = wide(OsStr::new(sddl));
    let mut descriptor = ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; the descriptor is freed by `Local`.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, ptr::null_mut())
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Local(descriptor))
}

/// Full access for this user and SYSTEM, none inherited from above. As 0700
/// does on Unix, a directory's setting reaches what is created in it later.
fn private_descriptor(inherited_by_children: bool, access: &str) -> io::Result<Local> {
    let user = current_user()?;
    let flags = if inherited_by_children { "OICI" } else { "" };
    descriptor(&format!("D:P(A;{flags};{access};;;{user})(A;{flags};{access};;;SY)"))
}

/// Gives `path` the private descriptor, for itself only: what's already in
/// a directory sits in the user's profile, closed to others already, and
/// walking a large store at every start would be slow.
fn protect(path: &Path, directory: bool) -> io::Result<()> {
    let descriptor = private_descriptor(directory, "FA")?;
    let path = wide(path.as_os_str());
    // SAFETY: `path` is NUL-terminated and `descriptor` valid for the call.
    if unsafe { SetFileSecurityW(path.as_ptr(), DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, descriptor.0) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn make_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let metadata = fs::symlink_metadata(dir)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a directory of this user's", dir.display()),
        ));
    }
    protect(dir, true)
}

pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
    protect(path, false)?;
    file.write_all(bytes)
}

pub fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

pub fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// This process's standard handles, made not inheritable for as long as it
/// lives. Windows hands a child every inheritable handle of its parent, so
/// a daemon started from a command whose output a caller reads through a
/// pipe would hold that pipe open, and the caller would wait for it as long
/// as the daemon runs.
struct Uninherited(Vec<(HANDLE, u32)>);

impl Uninherited {
    fn std_handles() -> Self {
        let mut kept = Vec::new();
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: plain queries and flag changes on this process's own handles.
            unsafe {
                let handle = GetStdHandle(which);
                let mut flags = 0;
                if !handle.is_null()
                    && handle != INVALID_HANDLE_VALUE
                    && GetHandleInformation(handle, &mut flags) != 0
                    && flags & HANDLE_FLAG_INHERIT != 0
                    && SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) != 0
                {
                    kept.push((handle, flags & HANDLE_FLAG_INHERIT));
                }
            }
        }
        Self(kept)
    }
}

impl Drop for Uninherited {
    fn drop(&mut self) {
        for (handle, flags) in &self.0 {
            // SAFETY: restores the flag this value cleared on its own handle.
            unsafe { SetHandleInformation(*handle, HANDLE_FLAG_INHERIT, *flags) };
        }
    }
}

pub fn spawn_detached(command: &mut std::process::Command) -> io::Result<std::process::Child> {
    // A hidden console of its own rather than none, so the programs it runs,
    // git above all, don't each open a window.
    let detached = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
    let _uninherited = Uninherited::std_handles();
    match command.creation_flags(detached | CREATE_BREAKAWAY_FROM_JOB).spawn() {
        // A job that doesn't allow breaking away: the daemon stays in it,
        // and a hook starts it again if the job ends (ADR 0036).
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED.cast_signed()) => command.creation_flags(detached).spawn(),
        spawned => spawned,
    }
}

pub fn own_group(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

pub fn kill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub fn kill(pid: u32) -> io::Result<()> {
    // SAFETY: a handle for terminating, closed by `Handle`.
    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = Handle(process);
    // SAFETY: the handle was opened with PROCESS_TERMINATE.
    if unsafe { TerminateProcess(process.0, 1) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Windows has no SIGSTOP: a debugger attaching stops every thread of the
/// process until it detaches.
pub fn suspend(pid: u32) -> io::Result<()> {
    // SAFETY: plain calls on a process id.
    unsafe {
        if DebugActiveProcess(pid) == 0 {
            return Err(io::Error::last_os_error());
        }
        // Detaching, or this process ending, leaves it running.
        DebugSetProcessKillOnExit(0);
    }
    Ok(())
}

pub fn resume(pid: u32) -> io::Result<()> {
    // SAFETY: a plain call on a process id this thread debugs.
    if unsafe { DebugActiveProcessStop(pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn link(target: &Path, link: &Path) -> io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link).or_else(|_| junction::create(target, link))
    } else {
        std::os::windows::fs::symlink_file(target, link).or_else(|_| fs::copy(target, link).map(|_| ()))
    }
}

pub fn is_root() -> bool {
    false
}

pub fn needs_polling(root: &Path) -> bool {
    let Some(Component::Prefix(prefix)) = root.components().next() else {
        return false;
    };
    match prefix.kind() {
        Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            let drive = wide(OsStr::new(&format!("{}:\\", char::from(letter))));
            // SAFETY: `drive` is NUL-terminated.
            unsafe { GetDriveTypeW(drive.as_ptr()) == DRIVE_REMOTE }
        }
        _ => false,
    }
}

/// The daemon's control channel: a named pipe only this user can open,
/// whose server is checked to be this user's before anything is sent.
pub mod pipe {
    use std::ffi::OsStr;
    use std::fs;
    use std::io;
    use std::os::windows::io::AsRawHandle as _;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    use super::{Handle, current_user, private_descriptor, token_user};

    const BUSY_WAIT: Duration = Duration::from_secs(2);

    /// One instance of the pipe, waiting for a client. The first one fails
    /// if the pipe exists already, so nobody can have taken the name before
    /// the daemon.
    pub fn serve(name: &OsStr, first: bool) -> io::Result<NamedPipeServer> {
        let descriptor = private_descriptor(false, "GA")?;
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).expect("small"),
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let mut options = ServerOptions::new();
        options.first_pipe_instance(first).reject_remote_clients(true);
        // SAFETY: `attributes` and the descriptor it points at outlive the call.
        unsafe { options.create_with_security_attributes_raw(name, (&raw mut attributes).cast()) }
    }

    /// The process serving the pipe `name`: the daemon, for tests and tools
    /// that need to stop it.
    pub fn server_pid(name: &Path) -> io::Result<u32> {
        let file = connect(name)?;
        let mut pid = 0;
        // SAFETY: the handle is the open pipe's; `pid` is written on success.
        if unsafe { GetNamedPipeServerProcessId(file.as_raw_handle(), &mut pid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(pid)
    }

    /// Opens the pipe, waiting while every instance is busy, and refuses one
    /// whose server runs as someone else.
    pub fn connect(name: &Path) -> io::Result<fs::File> {
        let deadline = Instant::now() + BUSY_WAIT;
        let file = loop {
            match fs::OpenOptions::new().read(true).write(true).open(name) {
                Ok(file) => break file,
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY.cast_signed()) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => return Err(error),
            }
        };
        let mut pid = 0;
        // SAFETY: the handle is the open pipe's; `pid` is written on success.
        if unsafe { GetNamedPipeServerProcessId(file.as_raw_handle(), &mut pid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let impostor = || io::Error::new(io::ErrorKind::PermissionDenied, format!("{} belongs to another user", name.display()));
        // SAFETY: a plain query; a null handle means it couldn't be opened.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return Err(impostor());
        }
        let process = Handle(process);
        if token_user(process.0).ok() != Some(current_user()?) {
            return Err(impostor());
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, BufReader, Write as _};

    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pipe_name_cannot_be_taken_before_the_daemon() {
        let name = format!(r"\\.\pipe\silt-test-first-{}", std::process::id());
        let _server = super::pipe::serve(name.as_ref(), true).expect("serves");
        assert!(super::pipe::serve(name.as_ref(), true).is_err(), "a first instance fails once the pipe exists");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_pipe_answers_this_user() {
        let name = format!(r"\\.\pipe\silt-test-{}", std::process::id());
        let mut server = super::pipe::serve(name.as_ref(), true).expect("serves");
        let client_name = name.clone();
        let client = tokio::task::spawn_blocking(move || {
            let mut pipe = super::pipe::connect(client_name.as_ref()).expect("connects");
            pipe.write_all(b"ping\n").expect("writes");
            let mut answer = String::new();
            BufReader::new(pipe).read_line(&mut answer).expect("reads");
            answer
        });
        server.connect().await.expect("a client arrives");
        let mut lines = tokio::io::BufReader::new(&mut server).lines();
        assert_eq!(lines.next_line().await.expect("reads").as_deref(), Some("ping"));
        server.write_all(b"pong\n").await.expect("answers");
        assert_eq!(client.await.expect("joins"), "pong\n");
    }
}
