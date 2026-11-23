//! Riwayat clipboard teks & gambar (hanya di memori). Pembacaan/penulisan clipboard ada di `platform::clip`.
use crate::state::{Clip, ClipImage, State};
use std::hash::{Hash, Hasher};
use tiny_skia::Pixmap;

pub const MAX_ITEMS: usize = 30;
pub const MAX_IMAGES: usize = 8;
pub const MAX_CHARS: usize = 10_000;
/// thumbnail muat dalam kotak ini (px)
pub const THUMB: (u32, u32) = (72, 40);

/// Masukkan item baru ke riwayat (terbaru di depan, tanpa duplikat, jumlah dibatasi).
pub fn push(state: &State, item: Clip) {
    let mut s = state.lock().unwrap();
    // dedupe: item sama naik ke paling atas
    s.clips.retain(|c| match (c, &item) {
        (Clip::Text(a), Clip::Text(b)) => a != b,
        (Clip::Image(a), Clip::Image(b)) => a.hash != b.hash,
        _ => true,
    });
    s.clips.insert(0, item);
    s.clips.truncate(MAX_ITEMS);
    // batasi jumlah gambar (yang terlama dibuang)
    let mut n = 0;
    s.clips.retain(|c| !matches!(c, Clip::Image(_)) || { n += 1; n <= MAX_IMAGES });
    s.clips_rev += 1;
}

pub fn make_image(pm: Pixmap) -> Option<ClipImage> {
    let (w, h) = (pm.width(), pm.height());
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    (w, h).hash(&mut hs);
    pm.data().iter().step_by(61).for_each(|b| b.hash(&mut hs));
    let thumb = downscale(&pm, THUMB.0, THUMB.1)?;
    let png = pm.encode_png().ok()?;
    Some(ClipImage { w, h, png, thumb, hash: hs.finish() })
}

/// Perkecil dengan rata-rata kotak (anti-aliasing bagus untuk rasio besar).
fn downscale(src: &Pixmap, maxw: u32, maxh: u32) -> Option<Pixmap> {
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let k = (maxw as f32 / sw).min(maxh as f32 / sh).min(1.0);
    let (tw, th) = (((sw * k).round() as u32).max(1), ((sh * k).round() as u32).max(1));
    let mut out = Pixmap::new(tw, th)?;
    let (sd, swu) = (src.data(), src.width() as usize);
    let od = out.data_mut();
    for ty in 0..th as usize {
        let y0 = (ty as f32 / k) as usize;
        let y1 = (((ty + 1) as f32 / k) as usize).clamp(y0 + 1, sh as usize);
        for tx in 0..tw as usize {
            let x0 = (tx as f32 / k) as usize;
            let x1 = (((tx + 1) as f32 / k) as usize).clamp(x0 + 1, sw as usize);
            let mut acc = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * swu + x) * 4;
                    for c in 0..4 {
                        acc[c] += sd[i + c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (ty * tw as usize + tx) * 4;
            for c in 0..4 {
                od[o + c] = (acc[c] / n) as u8;
            }
        }
    }
    Some(out)
}
