//! Backend Linux: X11 (juga lewat XWayland) + D-Bus (MPRIS, BlueZ, logind) + sysfs + pactl.
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use winit::{
    event_loop::EventLoop,
    window::WindowAttributes,
};

pub mod bright;
pub mod clip;
pub mod devices;
pub mod dragout;
pub mod media;
pub mod sys;
mod window;
mod x11;

pub use window::{forget, make_overlay, present, set_activatable, wait_vsync, win_id, WinId};

use super::{FontPaths, LocalTime, Settings, WinKind};

/// Event loop selalu X11 (di Wayland lewat XWayland): posisi absolut + always-on-top hanya ada di X11.
pub fn event_loop() -> EventLoop<()> {
    use winit::platform::x11::EventLoopBuilderExtX11;
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("notchy: DISPLAY tidak ada. Butuh sesi X11 atau XWayland.");
        std::process::exit(1);
    }
    EventLoop::builder().with_x11().build().expect("gagal membuka koneksi X11")
}

/// Atribut window khusus X11: tipe window (dock = tanpa dekorasi/taskbar, selalu di atas) dan nama kelas.
pub fn window_attrs(a: WindowAttributes, kind: WinKind) -> WindowAttributes {
    use winit::platform::x11::{WindowAttributesExtX11, WindowType};
    let ty = match kind {
        WinKind::Notch => WindowType::Dock,
        WinKind::Pip => WindowType::Utility,
    };
    a.with_x11_window_type(vec![ty]).with_name("notchy", "notchy")
}

pub fn local_time() -> LocalTime {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        LocalTime {
            year: (tm.tm_year + 1900) as u32,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            weekday: tm.tm_wday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
        }
    }
}

/// Bunyi notifikasi timer: suara event dari tema freedesktop, kalau tidak ada bel terminal.
pub fn beep() {
    const SOUND: &str = "/usr/share/sounds/freedesktop/stereo/complete.oga";
    let quiet = |c: &mut Command| c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok();
    if Path::new(SOUND).exists() && quiet(Command::new("paplay").arg(SOUND)) {
        return;
    }
    if quiet(Command::new("canberra-gtk-play").args(["-i", "complete"])) {
        return;
    }
    eprint!("\x07");
}

/// Jalankan program lepas (tanpa menunggu); true kalau berhasil di-spawn.
fn spawn_detached(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

/// Buka file/folder dengan aplikasi default.
pub fn open_path(path: &Path) {
    let _ = Command::new("xdg-open").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Buka halaman pengaturan: KDE pakai systemsettings, selain itu GNOME Settings; lalu alat umum lain.
pub fn open_settings(which: Settings) {
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("KDE"));
    let candidates: &[(&str, &[&str])] = match (which, kde) {
        (Settings::Power, true) => &[("systemsettings", &["kcm_powerdevilprofilesconfig"])],
        (Settings::Network, true) => &[("systemsettings", &["kcm_networkmanagement"])],
        (Settings::Bluetooth, true) => &[("systemsettings", &["kcm_bluetooth"])],
        (Settings::Power, false) => &[("gnome-control-center", &["power"]), ("xfce4-power-manager-settings", &[])],
        (Settings::Network, false) => &[("gnome-control-center", &["network"]), ("nm-connection-editor", &[])],
        (Settings::Bluetooth, false) => {
            &[("gnome-control-center", &["bluetooth"]), ("blueman-manager", &[]), ("blueberry", &[])]
        }
    };
    for (cmd, args) in candidates {
        if spawn_detached(cmd, args) {
            return;
        }
    }
}

/// Folder data per-user (config, todo): `$XDG_CONFIG_HOME/notchy` atau `~/.config/notchy`.
pub fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("notchy"))
}

/// Cari font lewat fontconfig (`fc-match`); None kalau fc-match tidak ada atau hasilnya bukan file.
fn fc_match(pattern: &str) -> Option<(String, u32)> {
    let out = Command::new("fc-match").args(["-f", "%{file}|%{index}", pattern]).stderr(Stdio::null()).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    let (file, index) = s.rsplit_once('|')?;
    Path::new(file).is_file().then(|| (file.to_string(), index.trim().parse().unwrap_or(0)))
}

/// Cari font pertama yang ada dari daftar path umum distro.
fn first_existing(paths: &[&str]) -> Option<(String, u32)> {
    paths.iter().find(|p| Path::new(p).is_file()).map(|p| (p.to_string(), 0))
}

pub fn fonts() -> FontPaths {
    let regular = fc_match("sans-serif")
        .or_else(|| {
            first_existing(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            ])
        })
        .unwrap_or_else(|| {
            eprintln!("notchy: tidak ada font sans-serif; pasang paket font (mis. fonts-dejavu-core)");
            std::process::exit(1);
        });
    let bold = fc_match("sans-serif:bold")
        .or_else(|| {
            first_existing(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            ])
        })
        .unwrap_or_else(|| regular.clone());
    // cadangan glyph CJK (Jepang, Korea, Mandarin) dan simbol; duplikat dibuang
    let mut fallback: Vec<(String, u32)> = Vec::new();
    for lang in ["ja", "ko", "zh-cn", "ar", "emoji"] {
        let pat = if lang == "emoji" { "emoji".to_string() } else { format!("sans-serif:lang={lang}") };
        if let Some(f) = fc_match(&pat) {
            if f != regular && f != bold && !fallback.contains(&f) {
                fallback.push(f);
            }
        }
    }
    FontPaths { regular, bold, fallback }
}

/// Penjaga single-instance (flock); dilepas saat proses berakhir.
pub struct Guard(#[allow(dead_code)] Option<File>);

/// Satu instance saja. None = sudah ada instance lain.
pub fn single_instance() -> Option<Guard> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let uid = unsafe { libc::getuid() };
    let lock = dir.join(format!("notchy-{uid}.lock"));
    let file = match File::options().create(true).truncate(false).write(true).open(&lock) {
        Ok(f) => f,
        Err(e) => {
            // jangan keluar diam-diam seolah ada instance lain
            eprintln!("notchy: tidak bisa membuat {}: {e}; lanjut tanpa penjaga single-instance", lock.display());
            return Some(Guard(None));
        }
    };
    match file.try_lock() {
        Ok(()) => Some(Guard(Some(file))),
        // NOTCHY_DUMP (debug screenshot) boleh jalan berdampingan dengan instance utama
        Err(_) if std::env::var_os("NOTCHY_DUMP").is_some() => Some(Guard(None)),
        Err(_) => None,
    }
}
