//! Lapisan platform: semua akses OS (window overlay, clipboard, media, sistem, drag) ada di sini.
//! `windows/` dan `linux/` mengekspos API yang sama; sisanya (render, UI, state) lintas-platform.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use self::linux::*;

/// Jenis window: notch (dock, tanpa fokus) atau jendela PiP lirik (bisa di-resize/drag).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WinKind {
    Notch,
    Pip,
}

/// Halaman pengaturan OS yang bisa dibuka dari halaman Status.
#[derive(Clone, Copy)]
pub enum Settings {
    Power,
    Network,
    Bluetooth,
}

pub struct LocalTime {
    pub year: u32,
    pub month: u32,
    /// 1..=31
    pub day: u32,
    /// 0 = Minggu
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

/// Font (path, index koleksi) untuk teks UI + cadangan CJK.
pub struct FontPaths {
    pub regular: (String, u32),
    pub bold: (String, u32),
    pub fallback: Vec<(String, u32)>,
}
