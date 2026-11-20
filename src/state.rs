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
    /// riwayat clipboard teks & gambar (terbaru di depan), hanya di memori
    pub clips: Vec<Clip>,
    /// naik setiap riwayat clipboard berubah (untuk trigger redraw)
    pub clips_rev: u64,
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
}
