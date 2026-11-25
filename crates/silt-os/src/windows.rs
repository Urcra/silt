use std::ffi::{OsStr, OsString, c_void};
use std::fs;
use std::io::{self, Write as _};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::fs::MetadataExt as _;
use std::os::windows::process::CommandExt as _;
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_MORE_DATA, ERROR_SUCCESS, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, GetDriveTypeW};
use windows_sys::Win32::System::Diagnostics::Debug::{DebugActiveProcess, DebugActiveProcessStop, DebugSetProcessKillOnExit};
use windows_sys::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_INFORMATION, PROCESS_TERMINATE, STARTUPINFOW, TerminateProcess,
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

/// The directories the machine's and the user's `Path` list now, expanded,
/// as a process Explorer started after the last change would get them.
pub fn configured_path_dirs() -> Vec<PathBuf> {
    const MACHINE: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    [(HKEY_LOCAL_MACHINE, MACHINE), (HKEY_CURRENT_USER, "Environment")]
        .into_iter()
        .filter_map(|(key, subkey)| registry_string(key, subkey, "Path"))
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect()
}

/// A string value, `REG_EXPAND_SZ` ones expanded.
fn registry_string(key: HKEY, subkey: &str, value: &str) -> Option<OsString> {
    let subkey = wide(OsStr::new(subkey));
    let value = wide(OsStr::new(value));
    let mut buffer: Vec<u16> = Vec::new();
    // An expanded value can outgrow the size asked for, so this asks again.
    for _ in 0..4 {
        let mut bytes = u32::try_from(buffer.len() * 2).ok()?;
        let data = if buffer.is_empty() { ptr::null_mut() } else { buffer.as_mut_ptr().cast() };
        // SAFETY: the names are NUL-terminated, and `data` is null or holds
        // `bytes` bytes.
        let status =
            unsafe { RegGetValueW(key, subkey.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, ptr::null_mut(), data, &mut bytes) };
        match status {
            ERROR_SUCCESS if !buffer.is_empty() => {
                let len = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
                return Some(OsString::from_wide(&buffer[..len]));
            }
            ERROR_SUCCESS | ERROR_MORE_DATA => buffer = vec![0; usize::try_from(bytes).ok()?.div_ceil(2).max(1)],
            _ => return None,
        }
    }
    None
}

pub fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// `name` without an extension `PATHEXT` lets a command leave off, as `npx`
/// runs `npx.cmd`.
pub fn without_run_extension(name: &str) -> Option<&str> {
    let (stem, extension) = name.rsplit_once('.')?;
    let run_extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    run_extensions
        .split(';')
        .any(|listed| listed.strip_prefix('.').is_some_and(|listed| listed.eq_ignore_ascii_case(extension)))
        .then_some(stem)
}

/// Starts `command` with `CreateProcessW` itself, as `Command` always lets a
/// child inherit every inheritable handle: a daemon started from a command
/// whose output a caller reads through a pipe would hold that pipe, and the
/// caller would wait for it as long as the daemon runs. The child gets no
/// handles of this process's, so the stdio set on `command` is not used.
pub fn spawn_detached(command: &mut std::process::Command) -> io::Result<()> {
    let mut line = Vec::new();
    push_quoted(&mut line, command.get_program());
    for arg in command.get_args() {
        line.push(u16::from(b' '));
        push_quoted(&mut line, arg);
    }
    line.push(0);
    let dir = command.get_current_dir().map(|dir| wide(dir.as_os_str()));
    let environment = environment_block(command);
    // A hidden console of its own rather than none, so the programs it runs,
    // git above all, don't each open a window.
    let detached = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT;
    match create_process(&mut line, dir.as_deref(), environment.as_deref(), detached | CREATE_BREAKAWAY_FROM_JOB) {
        // A job that doesn't allow breaking away: the daemon stays in it,
        // and a hook starts it again if the job ends (ADR 0036).
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED.cast_signed()) => {
            create_process(&mut line, dir.as_deref(), environment.as_deref(), detached)
        }
        started => started,
    }
}

fn create_process(line: &mut [u16], dir: Option<&[u16]>, environment: Option<&[u16]>, flags: u32) -> io::Result<()> {
    // SAFETY: zeroed is a valid value of these plain C structs.
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = u32::try_from(size_of::<STARTUPINFOW>()).unwrap_or(u32::MAX);
    // SAFETY: as above.
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the strings are NUL-terminated and outlive the call, the
    // command line is writable as CreateProcessW asks, and the handles it
    // returns are closed here.
    unsafe {
        if CreateProcessW(
            ptr::null(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            flags,
            environment.map_or(ptr::null(), |block| block.as_ptr().cast()),
            dir.map_or(ptr::null(), <[u16]>::as_ptr),
            &startup,
            &mut process,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

/// `arg` as the C runtime, and so Rust, splits a command line back.
fn push_quoted(line: &mut Vec<u16>, arg: &OsStr) {
    const QUOTE: u16 = 0x22;
    const BACKSLASH: u16 = 0x5C;
    let arg: Vec<u16> = arg.encode_wide().collect();
    if !arg.is_empty() && !arg.iter().any(|&char| char == QUOTE || char == u16::from(b' ') || char == u16::from(b'\t')) {
        line.extend(arg);
        return;
    }
    line.push(QUOTE);
    let mut backslashes = 0;
    for char in arg {
        if char == BACKSLASH {
            backslashes += 1;
        } else {
            if char == QUOTE {
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes + 1));
            }
            backslashes = 0;
        }
        line.push(char);
    }
    line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
    line.push(QUOTE);
}

/// This process's environment with `command`'s changes, sorted as Windows
/// wants it; `None` when there are none, and the child inherits it as is.
fn environment_block(command: &std::process::Command) -> Option<Vec<u16>> {
    command.get_envs().next()?;
    let key = |name: &OsStr| name.to_string_lossy().to_uppercase();
    let mut vars: std::collections::BTreeMap<String, (std::ffi::OsString, std::ffi::OsString)> =
        std::env::vars_os().map(|(name, value)| (key(&name), (name, value))).collect();
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => vars.insert(key(name), (name.to_owned(), value.to_owned())),
            None => vars.remove(&key(name)),
        };
    }
    let mut block = Vec::new();
    for (name, value) in vars.into_values() {
        block.extend(name.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Some(block)
}

pub fn own_group(command: &mut tokio::process::Command) {
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

pub fn own_group_std(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt as _;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

pub fn own_session_std(command: &mut std::process::Command) {
    own_group_std(command);
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

    #[test]
    fn arguments_are_quoted_as_the_c_runtime_splits_them() {
        let quoted = |arg: &str| {
            let mut line = Vec::new();
            super::push_quoted(&mut line, std::ffi::OsStr::new(arg));
            String::from_utf16(&line).expect("utf-16")
        };
        assert_eq!(quoted("run"), "run");
        assert_eq!(quoted(r"C:\a\silt.exe"), r"C:\a\silt.exe");
        assert_eq!(quoted(""), r#""""#);
        assert_eq!(quoted(r"C:\Program Files\"), r#""C:\Program Files\\""#);
        assert_eq!(quoted(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quoted(r#"a\"b"#), r#""a\\\"b""#);
    }

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
