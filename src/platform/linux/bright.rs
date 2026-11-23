//! Kecerahan layar internal Linux: /sys/class/backlight. Menulis: langsung ke sysfs kalau diizinkan
//! (aturan udev / grup video), kalau tidak lewat logind `SetBrightness` (tanpa root untuk sesi aktif).
use crate::state::State;
use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};
use zbus::blocking::{connection::Builder, Connection};

struct Panel {
    name: String,
    dir: PathBuf,
    max: u32,
}

/// Panel backlight pertama; utamakan tipe firmware/platform yang dikontrol firmware (lebih akurat dari raw).
fn find() -> Option<Panel> {
    let mut panels: Vec<(u8, Panel)> = fs::read_dir("/sys/class/backlight")
        .ok()?
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let max: u32 = fs::read_to_string(dir.join("max_brightness")).ok()?.trim().parse().ok()?;
            let rank = match fs::read_to_string(dir.join("type")).unwrap_or_default().trim() {
                "firmware" => 0,
                "platform" => 1,
                _ => 2,
            };
            (max > 0).then(|| (rank, Panel { name: e.file_name().to_string_lossy().into_owned(), dir, max }))
        })
        .collect();
    panels.sort_by_key(|(r, _)| *r);
    panels.into_iter().next().map(|(_, p)| p)
}

impl Panel {
    fn get(&self) -> Option<f32> {
        let cur: f32 = fs::read_to_string(self.dir.join("brightness")).ok()?.trim().parse().ok()?;
        Some((cur / self.max as f32).clamp(0.0, 1.0))
    }

    fn set(&self, conn: &mut Option<Connection>, v: f32) -> bool {
        // jangan sampai layar mati total: minimal 1%
        let raw = ((v.clamp(0.0, 1.0) * self.max as f32).round() as u32).clamp((self.max / 100).max(1), self.max);
        if fs::write(self.dir.join("brightness"), raw.to_string()).is_ok() {
            return true;
        }
        if conn.is_none() {
            *conn = Builder::system().ok().and_then(|b| b.method_timeout(Duration::from_secs(3)).build().ok());
        }
        conn.as_ref().is_some_and(|c| {
            c.call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1/session/auto",
                Some("org.freedesktop.login1.Session"),
                "SetBrightness",
                &("backlight", self.name.as_str(), raw),
            )
            .is_ok()
        })
    }
}

/// Thread kecerahan: baca berkala, set dari UI (nilai 0..1). Flash "Brightness X%" kalau diubah dari luar.
pub fn spawn(state: State) -> Sender<f32> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || run(state, rx));
    tx
}

fn run(state: State, rx: Receiver<f32>) {
    let panel = find();
    let mut system_bus: Option<Connection> = None;
    let mut first = true;
    let mut flash_lock: Option<Instant> = None;
    loop {
        // tanpa panel internal (desktop): tidak ada yang perlu dipolling
        let period = if panel.is_some() { 3000 } else { 60_000 };
        let mut want = None;
        match rx.recv_timeout(Duration::from_millis(period)) {
            Ok(v) => {
                want = Some(v);
                while let Ok(v) = rx.try_recv() {
                    want = Some(v);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let Some(p) = &panel else { continue };
        if let Some(v) = want {
            p.set(&mut system_bus, v);
            continue; // UI sudah menampilkan nilai baru; baca ulang di putaran berikut
        }
        let cur = p.get();
        let mut s = state.lock().unwrap();
        if s.brightness_hold.is_some_and(|t| Instant::now() < t) {
            continue; // UI baru mengubah nilai; jangan timpa dengan bacaan lama
        }
        if let (Some(c), Some(old)) = (cur, s.brightness) {
            // adaptive brightness meredup bertahap: abaikan perubahan kecil & batasi flash
            if !first && (c - old).abs() >= 0.03 && flash_lock.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
                s.flash(format!("Brightness {}%", (c * 100.0).round() as i32));
                flash_lock = Some(Instant::now());
            }
        }
        s.brightness = cur;
        first = false;
    }
}
