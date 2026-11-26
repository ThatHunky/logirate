//! logirated.exe: the watcher without a console window, for autostart.
//!
//! It owns a hidden window so it can be asked to close (by the logirate window, the
//! installer, or Windows signing out / shutting down). On close it hands the mouse back
//! to its onboard profile first, so it never leaves the mouse stuck at a game's rate.
#![windows_subsystem = "windows"]

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW, TranslateMessage, WM_CLOSE,
    WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW,
};

const MAX_LOG: u64 = 1024 * 1024;
/// Window class the logirate window looks for to ask the service to stop.
pub const WINDOW_CLASS: &str = "logirated-window";

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

fn log(msg: &str) {
    let Some(path) = LOG_PATH.get() else { return };
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let line = format!("[{:02}:{:02}:{:02} UTC] {msg}\n", secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
    if let Err(e) = written {
        // Leave a trace somewhere if the real log can't be written.
        let fallback = std::env::temp_dir().join("logirated-error.txt");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(fallback) {
            let _ = writeln!(f, "can't write {}: {e}; message was: {msg}", path.display());
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            logirate::watch::shutdown(&mut log);
            std::process::exit(0);
        }
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION if wparam != 0 => {
            logirate::watch::shutdown(&mut log);
            std::process::exit(0);
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() {
    // One instance per user session.
    let mutex_name = wide("Local\\logirated-single-instance");
    unsafe {
        CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
    }

    let dir = logirate::paths::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("logirate.log");
    if std::fs::metadata(&path).map(|m| m.len() > MAX_LOG).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("logirate.old.log"));
    }
    let _ = LOG_PATH.set(path);

    std::thread::spawn(|| logirate::watch::run(&mut log));

    unsafe {
        let class = wide(WINDOW_CLASS);
        let instance = GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(window_proc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);
        // A real (never shown) top-level window: installers and taskkill-style tools only
        // send WM_CLOSE to top-level windows, not to message-only ones.
        let title = wide("logirate service");
        CreateWindowExW(0, class.as_ptr(), title.as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
