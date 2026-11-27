use anyhow::{Context, Result, ensure};
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io,
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_NO_MORE_FILES, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, WAIT_OBJECT_0,
        WAIT_TIMEOUT, WPARAM,
    },
    Globalization::{CSTR_EQUAL, CompareStringOrdinal},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess,
            WaitForSingleObject,
        },
    },
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            AllowSetForegroundWindow, EnumWindows, GetForegroundWindow, GetWindowTextW,
            GetWindowThreadProcessId, PostMessageW, RegisterWindowMessageW, SMTO_ABORTIFHUNG,
            SendMessageTimeoutW, WM_CLOSE, WM_NCDESTROY,
        },
    },
};

const EXE_NAME: &str = "meme-soundboard.exe";
const APP_TITLE: &str = "Meme Soundboard";
const SUBCLASS_ID: usize = 1;
const GRACEFUL_EXIT_MS: u32 = 2_000;
const FORCED_EXIT_MS: u32 = 3_000;

static REPAINT: OnceLock<egui::Context> = OnceLock::new();
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static SHOW_REQUESTED: AtomicBool = AtomicBool::new(false);
static MAIN_WINDOW: AtomicUsize = AtomicUsize::new(0);

fn shutdown_message() -> u32 {
    static ID: OnceLock<u32> = OnceLock::new();
    *ID.get_or_init(|| {
        let message: Vec<_> = "MemeSoundboard.Update.Quit"
            .encode_utf16()
            .chain([0])
            .collect();
        unsafe { RegisterWindowMessageW(message.as_ptr()) }
    })
}

fn show_message() -> u32 {
    static ID: OnceLock<u32> = OnceLock::new();
    *ID.get_or_init(|| {
        let message: Vec<_> = "MemeSoundboard.Instance.Show"
            .encode_utf16()
            .chain([0])
            .collect();
        unsafe { RegisterWindowMessageW(message.as_ptr()) }
    })
}

pub fn listen_for_requests(context: &eframe::CreationContext<'_>) {
    let Ok(handle) = context.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(window) = handle.as_raw() else {
        return;
    };
    MAIN_WINDOW.store(window.hwnd.get() as usize, Relaxed);
    if shutdown_message() != 0 && show_message() != 0 {
        let _ = REPAINT.set(context.egui_ctx.clone());
        // The subclass lives on the main window's own thread and owns no borrowed data.
        unsafe { SetWindowSubclass(window.hwnd.get() as _, Some(window_proc), SUBCLASS_ID, 0) };
    }
}

pub fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.swap(false, Relaxed)
}

pub fn show_requested() -> bool {
    SHOW_REQUESTED.swap(false, Relaxed)
}

pub fn owns_foreground() -> bool {
    let foreground = unsafe { GetForegroundWindow() } as usize;
    foreground != 0 && foreground == MAIN_WINDOW.load(Relaxed)
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if message == shutdown_message() {
        SHUTDOWN_REQUESTED.store(true, Relaxed);
        if let Some(ctx) = REPAINT.get() {
            ctx.request_repaint();
        }
        return 0;
    }
    if message == show_message() {
        SHOW_REQUESTED.store(true, Relaxed);
        if let Some(ctx) = REPAINT.get() {
            ctx.request_repaint();
        }
        return 1;
    }
    if message == WM_NCDESTROY {
        unsafe { RemoveWindowSubclass(window, Some(window_proc), SUBCLASS_ID) };
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

pub fn close_running_install(executable: &Path) -> Result<()> {
    ensure!(
        executable.is_absolute()
            && executable
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case(OsStr::new(EXE_NAME))),
        "Invalid Soundboard executable path"
    );
    let target = fs::canonicalize(executable)?;
    for (pid, process) in running_copies(&target, PROCESS_SYNCHRONIZE | PROCESS_TERMINATE)? {
        close_process(pid, &process)?;
    }
    OpenOptions::new()
        .write(true)
        .open(target)
        .context("This installation's executable is still locked")?;
    Ok(())
}

pub fn show_running_install(executable: &Path) -> Result<bool> {
    let target = fs::canonicalize(executable)?;
    if show_message() == 0 {
        return Err(io::Error::last_os_error())
            .context("Can't register the window activation message");
    }
    for (pid, _process) in running_copies(&target, 0)? {
        let mut request = ShowRequest {
            pid,
            accepted: false,
        };
        unsafe {
            AllowSetForegroundWindow(pid);
            EnumWindows(
                Some(request_show),
                &mut request as *mut ShowRequest as LPARAM,
            );
        }
        if request.accepted {
            return Ok(true);
        }
    }
    Ok(false)
}

fn running_copies(target: &Path, access: u32) -> Result<Vec<(u32, OwnedHandle)>> {
    // Extracted Setup helpers need no access to library state or the app's instance lock.
    let own_id = unsafe { GetCurrentProcessId() };
    let session = session_id(own_id)?;
    let snapshot = owned(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })?;
    let mut matches = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) };
    while found != 0 {
        let pid = entry.th32ProcessID;
        let name = OsString::from_wide(without_nul(&entry.szExeFile));
        if pid != own_id
            && target
                .file_name()
                .is_some_and(|file| name.eq_ignore_ascii_case(file))
            && session_id(pid).is_ok_and(|other| other == session)
            && let Ok(process) =
                owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | access, 0, pid) })
            && image_path(&process).is_ok_and(|image| same_file_path(&image, target))
        {
            matches.push((pid, process));
        }
        found = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) };
    }
    let error = io::Error::last_os_error();
    ensure!(
        error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32),
        "Couldn't enumerate running applications. {error}"
    );
    Ok(matches)
}

fn close_process(pid: u32, process: &OwnedHandle) -> Result<()> {
    unsafe { EnumWindows(Some(request_close), pid as LPARAM) };
    if wait_for_exit(process, GRACEFUL_EXIT_MS)? {
        return Ok(());
    }
    // Older builds send WM_CLOSE to the tray. Keep the verified process handle for the fallback
    // so PID reuse cannot direct termination at another application.
    if unsafe { TerminateProcess(process.as_raw_handle(), 0) } == 0 {
        let error = io::Error::last_os_error();
        if !wait_for_exit(process, 0)? {
            return Err(error).context("Couldn't exit the previous version");
        }
    }
    ensure!(
        wait_for_exit(process, FORCED_EXIT_MS)?,
        "The previous version is still running"
    );
    Ok(())
}

unsafe extern "system" fn request_close(window: HWND, expected_pid: LPARAM) -> i32 {
    if is_main_window(window, expected_pid as u32) {
        let message = shutdown_message();
        if message != 0 {
            unsafe { PostMessageW(window, message, 0, 0) };
        }
        unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
    }
    1
}

struct ShowRequest {
    pid: u32,
    accepted: bool,
}

unsafe extern "system" fn request_show(window: HWND, data: LPARAM) -> i32 {
    let request = unsafe { &mut *(data as *mut ShowRequest) };
    if is_main_window(window, request.pid) {
        let mut acknowledged = 0;
        let sent = unsafe {
            SendMessageTimeoutW(
                window,
                show_message(),
                0,
                0,
                SMTO_ABORTIFHUNG,
                250,
                &mut acknowledged,
            )
        };
        request.accepted = sent != 0 && acknowledged == 1;
    }
    i32::from(!request.accepted)
}

fn is_main_window(window: HWND, expected_pid: u32) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    if pid != expected_pid {
        return false;
    }
    let mut title = [0; 64];
    let length = unsafe { GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32) };
    length > 0
        && title[..length as usize]
            .iter()
            .copied()
            .eq(APP_TITLE.encode_utf16())
}

fn wait_for_exit(process: &OwnedHandle, milliseconds: u32) -> Result<bool> {
    match unsafe { WaitForSingleObject(process.as_raw_handle(), milliseconds) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(io::Error::last_os_error()).context("Couldn't wait for Soundboard to close"),
    }
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}

fn session_id(pid: u32) -> io::Result<u32> {
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(pid, &mut session) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(session)
    }
}

fn image_path(process: &OwnedHandle) -> io::Result<PathBuf> {
    let mut image = vec![0; 32_768];
    let mut length = image.len() as u32;
    if unsafe {
        QueryFullProcessImageNameW(process.as_raw_handle(), 0, image.as_mut_ptr(), &mut length)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    fs::canonicalize(OsString::from_wide(&image[..length as usize]))
}

fn without_nul(value: &[u16]) -> &[u16] {
    &value[..value
        .iter()
        .position(|entry| *entry == 0)
        .unwrap_or(value.len())]
}

fn same_file_path(left: &Path, right: &Path) -> bool {
    let left: Vec<_> = left.as_os_str().encode_wide().collect();
    let right: Vec<_> = right.as_os_str().encode_wide().collect();
    unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        ) == CSTR_EQUAL
    }
}
