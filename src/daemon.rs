//! logirated.exe: the watcher without a console window, for autostart.
#![windows_subsystem = "windows"]

use std::io::Write;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Threading::CreateMutexW;

const MAX_LOG: u64 = 1024 * 1024;

fn main() {
    // One instance per user session.
    let name: Vec<u16> = "Local\\logirated-single-instance\0".encode_utf16().collect();
    unsafe {
        CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
    }

    let dir = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("logirate");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("logirate.log");
    if std::fs::metadata(&path).map(|m| m.len() > MAX_LOG).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("logirate.old.log"));
    }

    logirate::watch::run(&mut |msg| {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(f, "[{:02}:{:02}:{:02} UTC] {msg}", secs / 3600 % 24, secs / 60 % 60, secs % 60);
        }
    });
}
