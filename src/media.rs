//! Logika media yang dipakai semua platform: sinkronisasi ke state, artwork (tint/aksen), lirik lrclib.
use crate::state::State;
use std::{sync::Arc, thread, time::Instant};
use tiny_skia::Pixmap;

/// Sisi artwork (px).
pub const ART: u32 = 128;

/// Snapshot lagu dari backend platform (SMTC / MPRIS).
pub struct Track {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub pos_ms: i64,
    pub dur_ms: i64,
}

pub type Art = (Pixmap, [u8; 3], [u8; 3]);

/// Status polling: lagu terakhir + jumlah percobaan memuat artwork.
#[derive(Default)]
pub struct Tracker {
    last_key: String,
    art_tries: u8,
}

impl Tracker {
    /// Tidak ada sesi media -> kosongkan state.
    pub fn clear(&mut self, state: &State) {
        let mut s = state.lock().unwrap();
        if s.playing || !s.title.is_empty() {
            s.playing = false;
            s.title.clear();
            s.artist.clear();
            s.lyrics.clear();
            s.art = None;
        }
        self.last_key.clear();
    }

    /// Tulis lagu ke state; muat artwork (maks 8 percobaan per lagu) dan ambil lirik saat lagu berganti.
    pub fn publish(&mut self, state: &State, t: &Track, load_art: impl FnOnce() -> Option<Art>) {
        let key = format!("{}\u{1}{}", t.artist, t.title);
        let changed = key != self.last_key;
        {
            let mut s = state.lock().unwrap();
            s.title = t.title.clone();
            s.artist = t.artist.clone();
            s.playing = t.playing;
            s.pos_ms = t.pos_ms;
            s.dur_ms = t.dur_ms;
            s.stamp = Some(Instant::now());
            if changed {
                s.lyrics.clear();
                s.art = None;
                self.art_tries = 0;
            }
        }
        if self.art_tries < 8 {
            self.art_tries += 1;
            if let Some((pm, tint, accent)) = load_art() {
                self.art_tries = u8::MAX;
                let mut s = state.lock().unwrap();
                s.art = Some(Arc::new(pm));
                s.tint = tint;
                s.accent = accent;
            }
        }
        if changed {
            self.last_key = key;
            let (st, title, artist, end) = (state.clone(), t.title.clone(), t.artist.clone(), t.dur_ms);
            thread::spawn(move || {
                let lyrics = fetch_lyrics(&artist, &title, end / 1000).unwrap_or_default();
                let mut s = st.lock().unwrap();
                if s.title == title && s.artist == artist {
                    s.lyrics = lyrics;
                }
            });
        }
    }
}

/// Ambil lirik tersinkron (LRC) dari lrclib.net. Hanya artist/judul/durasi yang dikirim.
fn fetch_lyrics(artist: &str, title: &str, dur_s: i64) -> Option<Vec<(i64, String)>> {
    let (artist, title) = clean_meta(artist, title);
    // 1) cocok persis (artist + judul + durasi)
    let exact = ureq::get("https://lrclib.net/api/get")
        .query("artist_name", &artist)
        .query("track_name", &title)
        .query("duration", dur_s.to_string())
        .call()
        .ok()
        .and_then(|mut r| r.body_mut().read_to_string().ok())
        .and_then(|b| serde_json::from_str::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("syncedLyrics")?.as_str().map(str::to_string));
    if let Some(lrc) = exact {
        return Some(parse_lrc(&lrc));
    }
    // 2) fallback: pencarian bebas, pilih hasil ber-syncedLyrics dengan durasi terdekat
    let body = ureq::get("https://lrclib.net/api/search")
        .query("q", format!("{artist} {title}"))
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let arr: Vec<serde_json::Value> = serde_json::from_str(&body).ok()?;
    let best = arr
        .iter()
        .filter_map(|v| {
            let lrc = v.get("syncedLyrics")?.as_str()?;
            let d = v.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0);
            Some(((d - dur_s as f64).abs(), lrc))
        })
        .filter(|(diff, _)| dur_s <= 0 || *diff <= 8.0)
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(parse_lrc(best.1))
}

/// Rapikan metadata dari YouTube/browser:
/// "Perunggu - Gemilang (Official Lyric Video)" + "Perunggu Official" -> ("Perunggu", "Gemilang").
fn clean_meta(artist: &str, title: &str) -> (String, String) {
    fn strip_brackets(s: &str) -> String {
        let mut out = String::new();
        let mut depth = 0i32;
        for c in s.chars() {
            match c {
                '(' | '[' | '【' => depth += 1,
                ')' | ']' | '】' => depth = (depth - 1).max(0),
                _ if depth == 0 => out.push(c),
                _ => {}
            }
        }
        out
    }
    const JUNK: [&str; 9] = [
        "official music video", "official lyric video", "official video", "lyric video", "lyrics",
        "official audio", "audio", "mv", "m/v",
    ];
    let mut t = strip_brackets(title);
    let mut a = artist
        .trim_end_matches(" - Topic")
        .trim_end_matches("VEVO")
        .trim_end_matches(" Official")
        .trim_end_matches(" official")
        .trim()
        .to_string();
    // "Artist - Judul"
    if let Some((left, right)) = t.split_once(" - ").map(|(l, r)| (l.trim().to_string(), r.trim().to_string())) {
        if a.is_empty() || left.to_lowercase().contains(&a.to_lowercase()) || a.to_lowercase().contains(&left.to_lowercase()) {
            a = left;
            t = right;
        }
    }
    // buang "| ..." dan kata sampah di ujung
    if let Some((l, _)) = t.split_once('|') {
        t = l.to_string();
    }
    let mut tl = t.trim().to_string();
    for j in JUNK {
        let low = tl.to_lowercase();
        // hanya kata utuh di ujung (didahului spasi), dan jangan sampai judul jadi kosong
        if low.len() == tl.len() && low.len() > j.len() && low.ends_with(j) && low[..low.len() - j.len()].ends_with(' ') {
            tl.truncate(tl.len() - j.len());
            tl = tl.trim_end_matches([' ', '-', '–']).to_string();
        }
    }
    // "ft." / "feat." tidak ikut dicari
    for f in [" ft.", " feat.", " Ft.", " Feat."] {
        if let Some(i) = tl.find(f) {
            tl.truncate(i);
        }
    }
    (a.trim().to_string(), tl.trim().to_string())
}

fn parse_lrc(lrc: &str) -> Vec<(i64, String)> {
    let mut out = Vec::new();
    for line in lrc.lines() {
        let Some(rest) = line.strip_prefix('[') else { continue };
        let Some((tag, text)) = rest.split_once(']') else { continue };
        let Some((m, s)) = tag.split_once(':') else { continue };
        let (Ok(m), Ok(s)) = (m.parse::<f64>(), s.parse::<f64>()) else { continue };
        out.push((((m * 60.0 + s) * 1000.0) as i64, text.trim().to_string()));
    }
    out.sort_by_key(|l| l.0);
    out
}

/// Pixmap ART x ART dari data RGBA premultiplied + warna rata-rata (tint) + warna paling hidup (aksen).
pub fn finish_art(data: &[u8]) -> Option<Art> {
    let n = (ART * ART * 4) as usize;
    if data.len() < n {
        return None;
    }
    let data = &data[..n];
    let mut pm = Pixmap::new(ART, ART)?;
    pm.data_mut().copy_from_slice(data);

    let mut acc = [0u64; 3];
    for px in data.chunks_exact(4) {
        for k in 0..3 {
            acc[k] += px[k] as u64;
        }
    }
    let cnt = (ART * ART) as u64;
    let avg = [acc[0] / cnt, acc[1] / cnt, acc[2] / cnt];
    // normalisasi: kanal tertinggi = 150 agar tint tetap kelihatan tapi tidak menyilaukan
    let mx = (*avg.iter().max().unwrap()).max(1) as f32;
    let tint = avg.map(|c| ((c as f32 / mx) * 150.0) as u8);

    // aksen: rata-rata berbobot saturasi² x kecerahan -> warna "paling hidup" artwork (bukan rata-rata keruh)
    let (mut vac, mut vw) = ([0f32; 3], 0f32);
    for px in data.chunks_exact(4) {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
        if hi < 40.0 {
            continue;
        }
        let sat = (hi - lo) / hi;
        let w = sat * sat * (hi / 255.0);
        vac[0] += w * r;
        vac[1] += w * g;
        vac[2] += w * b;
        vw += w;
    }
    // artwork hampir abu-abu / gelap: aksen netral terang
    let accent = if vw < cnt as f32 * 0.01 {
        [235, 235, 240]
    } else {
        let m = vac.iter().cloned().fold(1.0, f32::max);
        vac.map(|c| (c / m * 235.0) as u8)
    };
    Some((pm, tint, accent))
}
