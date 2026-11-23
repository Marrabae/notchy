use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Default)]
pub struct Shared {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub pos_ms: i64,
    pub dur_ms: i64,
    pub stamp: Option<Instant>,
    pub lyrics: Vec<(i64, String)>,
    pub battery: Option<(u8, bool)>, // (persen, charging)
    pub volume: f32,
    pub activity: Option<(String, Instant)>,
    pub shelf: Vec<PathBuf>,
    pub art: Option<Arc<tiny_skia::Pixmap>>,
    /// warna rata-rata artwork (untuk gradient background)
    pub tint: [u8; 3],
    /// warna paling hidup dari artwork (untuk aksen UI: progress bar, ikon aktif, glow)
    pub accent: [u8; 3],
    /// riwayat clipboard teks & gambar (terbaru di depan), hanya di memori
    pub clips: Vec<Clip>,
    /// naik setiap riwayat clipboard berubah (untuk trigger redraw)
    pub clips_rev: u64,
    /// detail baterai (daya, kapasitas, estimasi waktu)
    pub power: Option<Power>,
    pub muted: bool,
    /// nama perangkat output audio default
    pub audio_out: String,
    /// kecerahan layar internal 0..1 (None = tidak didukung)
    pub brightness: Option<f32>,
    /// UI baru saja mengubah brightness: thread polling jangan menimpa nilainya sebelum waktu ini
    pub brightness_hold: Option<Instant>,
    pub net: Net,
    /// perangkat Bluetooth yang sedang tersambung
    pub devices: Vec<Device>,
}

#[derive(Clone, Default, PartialEq)]
pub struct Power {
    /// daya (mW): positif = mengisi, negatif = terpakai
    pub rate_mw: i32,
    pub remain_mwh: u32,
    pub full_mwh: u32,
    /// estimasi detik: sisa (on battery) atau sampai penuh (charging)
    pub secs: Option<u32>,
    pub saver: bool,
    /// benar-benar sedang mengisi (bisa false walau AC tersambung, mis. batas charge 80%)
    pub charging: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum NetKind {
    #[default]
    Offline,
    Wifi,
    Ethernet,
    Cellular,
}

#[derive(Clone, Default)]
pub struct Net {
    pub kind: NetKind,
    /// SSID / nama profil jaringan
    pub name: String,
    /// byte per detik
    pub down: f32,
    pub up: f32,
    /// riwayat (down, up) terbaru di belakang
    pub hist: Vec<(f32, f32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevKind {
    Audio,
    Input,
    Phone,
    Other,
}

impl DevKind {
    /// Tebak jenis perangkat dari Class of Device Bluetooth (kalau ada), lalu dari namanya.
    pub fn guess(name: &str, cod: Option<u32>) -> DevKind {
        // major device class: 2 = phone, 4 = audio/video, 5 = peripheral (HID)
        match cod.map(|c| (c >> 8) & 0x1F) {
            Some(2) => return DevKind::Phone,
            Some(4) => return DevKind::Audio,
            Some(5) => return DevKind::Input,
            _ => {}
        }
        let n = name.to_lowercase();
        let has = |ks: &[&str]| ks.iter().any(|k| n.contains(k));
        if has(&["bud", "pod", "head", "ear", "bass", "speaker", "audio", "sound", "jbl", "wh-", "wf-"]) {
            DevKind::Audio
        } else if has(&["mouse", "keyboard", "keys", "controller", "gamepad", "pen", "mx "]) {
            DevKind::Input
        } else if has(&["phone", "galaxy", "pixel", "redmi", "xiaomi", "oppo", "vivo"]) {
            DevKind::Phone
        } else {
            DevKind::Other
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Device {
    /// DeviceInstanceId (unik walau nama sama)
    pub id: String,
    pub name: String,
    pub kind: DevKind,
    pub battery: Option<u8>,
}

/// Perintah dari UI ke thread sistem.
pub enum SysCmd {
    Volume(f32),
    ToggleMute,
}

#[derive(Clone)]
pub enum Clip {
    Text(String),
    Image(Arc<ClipImage>),
}

pub struct ClipImage {
    pub w: u32,
    pub h: u32,
    /// gambar asli, dikompres PNG agar hemat RAM
    pub png: Vec<u8>,
    /// thumbnail kecil (RGBA premultiplied) untuk daftar
    pub thumb: tiny_skia::Pixmap,
    pub hash: u64,
}

pub type State = Arc<Mutex<Shared>>;

pub enum Cmd {
    PlayPause,
    Next,
    Prev,
    /// lompat ke posisi (ms)
    Seek(i64),
}

impl Shared {
    /// Posisi lagu saat ini (ms), diinterpolasi dari update terakhir SMTC.
    pub fn pos_now(&self) -> i64 {
        let extra = match (self.playing, self.stamp) {
            (true, Some(t)) => t.elapsed().as_millis() as i64,
            _ => 0,
        };
        (self.pos_ms + extra).min(self.dur_ms.max(1))
    }

    pub fn lyric_index(&self) -> Option<usize> {
        let p = self.pos_now();
        self.lyrics.iter().rposition(|(t, _)| *t <= p)
    }

    pub fn flash(&mut self, text: String) {
        self.activity = Some((text, Instant::now()));
    }

    /// Ganti daftar perangkat Bluetooth; kalau bukan scan pertama, tampilkan notifikasi tersambung/terputus.
    pub fn update_devices(&mut self, list: Vec<Device>, first: bool) {
        if !first {
            if let Some(d) = list.iter().find(|d| !self.devices.iter().any(|o| o.id == d.id)) {
                self.flash(format!("Connected · {}", d.name));
            } else if let Some(d) = self.devices.iter().find(|o| !list.iter().any(|d| d.id == o.id)) {
                let msg = format!("Disconnected · {}", d.name);
                self.flash(msg);
            }
        }
        self.devices = list;
    }
}
