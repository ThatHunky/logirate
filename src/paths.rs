//! Where logirate keeps its files. Uses the Windows known-folder API rather than
//! %APPDATA%/%LOCALAPPDATA%, which aren't always set (e.g. when started by an installer).

use std::path::PathBuf;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, FOLDERID_RoamingAppData, SHGetKnownFolderPath};
use windows_sys::core::{GUID, PWSTR};

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let mut raw: PWSTR = std::ptr::null_mut();
        let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut raw);
        let result = if hr == 0 && !raw.is_null() {
            let len = (0..).take_while(|&i| *raw.add(i) != 0).count();
            Some(PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(raw, len))))
        } else {
            None
        };
        CoTaskMemFree(raw as *const _);
        result
    }
}

fn folder(id: &GUID, env: &str) -> PathBuf {
    known_folder(id)
        .or_else(|| std::env::var_os(env).map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
}

/// `%APPDATA%\logirate\config.toml`
pub fn config_file() -> PathBuf {
    folder(&FOLDERID_RoamingAppData, "APPDATA").join("logirate").join("config.toml")
}

/// `%LOCALAPPDATA%\logirate`
pub fn data_dir() -> PathBuf {
    folder(&FOLDERID_LocalAppData, "LOCALAPPDATA").join("logirate")
}

pub fn log_file() -> PathBuf {
    data_dir().join("logirate.log")
}
