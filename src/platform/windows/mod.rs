//! Backend Windows: Win32 + WinRT.
use std::path::{Path, PathBuf};
use winit::window::WindowAttributes;

pub mod bright;
pub mod clip;
pub mod devices;
pub mod dragout;
pub mod media;
pub mod sys;
mod window;

pub use window::{forget, make_overlay, present, set_activatable, wait_vsync, win_id, WinId};

use super::{FontPaths, LocalTime, Settings, WinKind};

pub fn event_loop() -> winit::event_loop::EventLoop<()> {
    winit::event_loop::EventLoop::new().unwrap()
}

/// Atribut window khusus Windows (tanpa ikon taskbar).
pub fn window_attrs(a: WindowAttributes, _kind: WinKind) -> WindowAttributes {
    use winit::platform::windows::WindowAttributesExtWindows;
    a.with_skip_taskbar(true)
}

pub fn local_time() -> LocalTime {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    LocalTime {
        year: t.wYear as u32,
        month: t.wMonth as u32,
        day: t.wDay as u32,
        weekday: t.wDayOfWeek as u32,
        hour: t.wHour as u32,
        minute: t.wMinute as u32,
    }
}

pub fn beep() {
    unsafe {
        let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK);
    }
}

/// Buka file/folder dengan aplikasi default.
pub fn open_path(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

pub fn open_settings(which: Settings) {
    let uri = match which {
        Settings::Power => "ms-settings:batterysaver",
        Settings::Network => "ms-settings:network-status",
        Settings::Bluetooth => "ms-settings:bluetooth",
    };
    let _ = std::process::Command::new("explorer.exe").arg(uri).spawn();
}

/// Folder data per-user (config, todo).
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("notchy"))
}

pub fn fonts() -> FontPaths {
    let f = |p: &str, i: u32| (p.to_string(), i);
    FontPaths {
        regular: f(r"C:\Windows\Fonts\segoeui.ttf", 0),
        bold: f(r"C:\Windows\Fonts\segoeuib.ttf", 0),
        fallback: vec![
            f(r"C:\Windows\Fonts\malgun.ttf", 0),  // Korea
            f(r"C:\Windows\Fonts\YuGothR.ttc", 0), // Jepang
            f(r"C:\Windows\Fonts\msyh.ttc", 0),    // Mandarin
        ],
    }
}

/// Penjaga single-instance; dilepas saat proses berakhir.
pub struct Guard(#[allow(dead_code)] windows::core::Result<windows::Win32::Foundation::HANDLE>);

/// Satu instance saja (startup + buka manual tidak dobel). None = sudah ada instance lain.
pub fn single_instance() -> Option<Guard> {
    use windows::Win32::{
        Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
        System::Threading::CreateMutexW,
    };
    unsafe {
        let h = CreateMutexW(None, true, windows::core::w!(r"Local\notchy-single-instance"));
        // NOTCHY_DUMP (debug screenshot) boleh jalan berdampingan dengan instance utama
        if GetLastError() == ERROR_ALREADY_EXISTS && std::env::var_os("NOTCHY_DUMP").is_none() {
            return None;
        }
        Some(Guard(h))
    }
}
