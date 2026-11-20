use crate::state::{Cmd, State};
use std::{
    sync::mpsc::Receiver,
    thread,
    time::{Duration, Instant},
};
use std::sync::Arc;
use tiny_skia::Pixmap;
use windows::{
    Graphics::Imaging::{
        BitmapAlphaMode, BitmapBounds, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
        ColorManagementMode, ExifOrientationMode,
    },
    Media::Control::{
        GlobalSystemMediaTransportControlsSessionMediaProperties as Props,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PStatus,
    },
    Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
};

/// Thread polling SMTC (Spotify, browser, dll) + terima perintah kontrol.
pub fn spawn(state: State, rx: Receiver<Cmd>) {
    thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut last_key = String::new();
        let mut art_tries = 0u8;
        loop {
            if let Err(e) = poll_once(&state, &rx, &mut last_key, &mut art_tries) {
                // tidak ada sesi media -> kosongkan
                let mut s = state.lock().unwrap();
                if s.playing || !s.title.is_empty() {
                    s.playing = false;
                    s.title.clear();
                    s.artist.clear();
                    s.lyrics.clear();
                    s.art = None;
                }
                let _ = e;
                last_key.clear();
            }
            thread::sleep(Duration::from_millis(500));
        }
    });
}

fn poll_once(state: &State, rx: &Receiver<Cmd>, last_key: &mut String, art_tries: &mut u8) -> windows::core::Result<()> {
    let mgr = Manager::RequestAsync()?.join()?;
    let session = mgr.GetCurrentSession()?;

    while let Ok(cmd) = rx.try_recv() {
        let _ = match cmd {
            Cmd::PlayPause => session.TryTogglePlayPauseAsync()?.join(),
            Cmd::Next => session.TrySkipNextAsync()?.join(),
            Cmd::Prev => session.TrySkipPreviousAsync()?.join(),
            Cmd::Seek(ms) => session.TryChangePlaybackPositionAsync(ms.max(0) * 10_000)?.join(),
        };
    }

    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let title = props.Title()?.to_string();
    let artist = props.Artist()?.to_string();
    let tl = session.GetTimelineProperties()?;
    let playing = session.GetPlaybackInfo()?.PlaybackStatus()? == PStatus::Playing;
    // Position dari SMTC bisa basi (app hanya update saat state berubah):
    // tambahkan selisih waktu sejak LastUpdatedTime (FILETIME, 100ns sejak 1601).
    let last = tl.LastUpdatedTime()?.UniversalTime;
    let now_ft = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() / 100)
        .unwrap_or(0) as i64)
        + 116_444_736_000_000_000;
    let drift = if playing && last > 0 { ((now_ft - last) / 10_000).clamp(0, 6 * 3_600_000) } else { 0 };
    let pos = tl.Position()?.Duration / 10_000 + drift;
    let end = tl.EndTime()?.Duration / 10_000;

    let key = format!("{artist}\u{1}{title}");
    let changed = key != *last_key;
    {
        let mut s = state.lock().unwrap();
        s.title = title.clone();
        s.artist = artist.clone();
        s.playing = playing;
        s.pos_ms = pos;
        s.dur_ms = end;
        s.stamp = Some(Instant::now());
        if changed {
            s.lyrics.clear();
            s.art = None;
            *art_tries = 0;
        }
    }
    if *art_tries < 8 {
        *art_tries += 1;
        if let Ok((pm, tint)) = load_art(&props) {
            *art_tries = u8::MAX;
            let mut s = state.lock().unwrap();
            s.art = Some(Arc::new(pm));
            s.tint = tint;
        }
    }
    if changed {
        *last_key = key;
        let st = state.clone();
        thread::spawn(move || {
            let lyrics = fetch_lyrics(&artist, &title, end / 1000).unwrap_or_default();
            let mut s = st.lock().unwrap();
            if s.title == title && s.artist == artist {
                s.lyrics = lyrics;
            }
        });
    }
    Ok(())
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

/// Ambil thumbnail dari SMTC, crop tengah jadi kotak 128px (RGBA premultiplied) + warna rata-rata.
fn load_art(props: &Props) -> windows::core::Result<(Pixmap, [u8; 3])> {
    const S: u32 = 128;
    let stream = props.Thumbnail()?.OpenReadAsync()?.join()?;
    let dec = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let (w, h) = (dec.PixelWidth()?, dec.PixelHeight()?);
    let m = w.min(h).max(1);
    let (sw, sh) = ((w * S).div_ceil(m), (h * S).div_ceil(m));
    let tf = BitmapTransform::new()?;
    tf.SetScaledWidth(sw)?;
    tf.SetScaledHeight(sh)?;
    tf.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    tf.SetBounds(BitmapBounds { X: (sw - S) / 2, Y: (sh - S) / 2, Width: S, Height: S })?;
    let pd = dec
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Rgba8,
            BitmapAlphaMode::Premultiplied,
            &tf,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?;
    let data = pd.DetachPixelData()?;
    let n = (S * S * 4) as usize;
    if data.len() < n {
        return Err(windows::core::Error::empty());
    }
    let mut pm = Pixmap::new(S, S).unwrap();
    pm.data_mut().copy_from_slice(&data[..n]);

    let mut acc = [0u64; 3];
    for px in data[..n].chunks_exact(4) {
        for k in 0..3 {
            acc[k] += px[k] as u64;
        }
    }
    let cnt = (S * S) as u64;
    let avg = [acc[0] / cnt, acc[1] / cnt, acc[2] / cnt];
    // normalisasi: kanal tertinggi = 150 agar tint tetap kelihatan tapi tidak menyilaukan
    let mx = (*avg.iter().max().unwrap()).max(1) as f32;
    let tint = avg.map(|c| ((c as f32 / mx) * 150.0) as u8);
    Ok((pm, tint))
}
