//! Clipboard Windows: polling nomor urut clipboard (murah), baca/tulis CF_UNICODETEXT & CF_DIB, paste via SendInput.
#![allow(unsafe_op_in_unsafe_fn)] // fungsi unsafe di modul ini hanya membungkus API clipboard Win32
use crate::{
    clip::{make_image, push, MAX_CHARS},
    state::{Clip, State},
};
use std::{sync::Arc, thread, time::Duration};
use tiny_skia::Pixmap;
use windows::{
    core::w,
    Win32::{
        Foundation::{HANDLE, HGLOBAL, HWND},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
                IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::{CF_DIB, CF_UNICODETEXT},
        },
    },
};

pub fn spawn(state: State) {
    thread::spawn(move || {
        let mut seq = 0; // 0 -> isi clipboard saat start ikut terbaca
        loop {
            thread::sleep(Duration::from_millis(400));
            let now = unsafe { GetClipboardSequenceNumber() };
            if now == seq {
                continue;
            }
            seq = now;
            let Some(item) = read() else { continue };
            push(&state, item);
        }
    });
}

/// Baca isi clipboard: teks diutamakan (mis. copy dari Excel juga membawa gambar), lalu gambar.
fn read() -> Option<Clip> {
    unsafe {
        let excl = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        let hist = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if OpenClipboard(None).is_err() {
            return None;
        }
        let out = (|| {
            // password manager menandai isi sensitif -> jangan direkam
            if IsClipboardFormatAvailable(excl).is_ok() {
                return None;
            }
            if IsClipboardFormatAvailable(hist).is_ok() {
                if let Ok(h) = GetClipboardData(hist) {
                    let p = GlobalLock(HGLOBAL(h.0)) as *const u32;
                    let deny = !p.is_null() && *p == 0;
                    let _ = GlobalUnlock(HGLOBAL(h.0));
                    if deny {
                        return None;
                    }
                }
            }
            if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok() {
                let t = read_text()?;
                return (!t.trim().is_empty()).then(|| Clip::Text(t.chars().take(MAX_CHARS).collect()));
            }
            if IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() {
                let pm = read_dib()?;
                return Some(Clip::Image(Arc::new(make_image(pm)?)));
            }
            None
        })();
        let _ = CloseClipboard();
        out
    }
}

unsafe fn read_text() -> Option<String> {
    let h = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
    let g = HGLOBAL(h.0);
    let p = GlobalLock(g) as *const u16;
    if p.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
    }
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
    let _ = GlobalUnlock(g);
    Some(s)
}

/// CF_DIB (24/32 bpp) -> Pixmap RGBA premultiplied.
unsafe fn read_dib() -> Option<Pixmap> {
    let h = GetClipboardData(CF_DIB.0 as u32).ok()?;
    let g = HGLOBAL(h.0);
    let size = GlobalSize(g);
    let p = GlobalLock(g) as *const u8;
    if p.is_null() {
        return None;
    }
    let buf = std::slice::from_raw_parts(p, size);
    let pm = parse_dib(buf);
    let _ = GlobalUnlock(g);
    pm
}

fn parse_dib(buf: &[u8]) -> Option<Pixmap> {
    let rd_u32 = |o: usize| buf.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let rd_i32 = |o: usize| rd_u32(o).map(|v| v as i32);
    let rd_u16 = |o: usize| buf.get(o..o + 2).map(|b| u16::from_le_bytes(b.try_into().unwrap()));
    let hsize = rd_u32(0)? as usize;
    let w = rd_i32(4)?;
    let hraw = rd_i32(8)?;
    let bpp = rd_u16(14)? as usize;
    let comp = rd_u32(16)?;
    let used = rd_u32(32).unwrap_or(0) as usize;
    if w <= 0 || hraw == 0 || !(bpp == 24 || bpp == 32) || w > 16384 || hraw.abs() > 16384 {
        return None;
    }
    let (w, h) = (w as usize, hraw.unsigned_abs() as usize);
    let mut off = hsize + used * 4;
    if comp == 3 && hsize == 40 {
        off += 12; // BI_BITFIELDS: 3 mask setelah header
    }
    let stride = (w * bpp).div_ceil(32) * 4;
    if buf.len() < off + stride * h {
        return None;
    }
    let px = &buf[off..];
    let bytes = bpp / 8;
    let mut rgba = vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for y in 0..h {
        let sy = if hraw > 0 { h - 1 - y } else { y }; // bottom-up kalau tinggi positif
        let row = &px[sy * stride..sy * stride + w * bytes];
        for x in 0..w {
            let s = &row[x * bytes..];
            let d = &mut rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = if bytes == 4 { s[3] } else { 255 };
            any_alpha |= bytes == 4 && s[3] != 0;
        }
    }
    let mut pm = Pixmap::new(w as u32, h as u32)?;
    let data = pm.data_mut();
    for (i, c) in rgba.chunks_exact(4).enumerate() {
        // banyak app menulis alpha 0 di DIB 32-bit -> anggap opak
        let a = if any_alpha { c[3] as u32 } else { 255 };
        let o = &mut data[i * 4..i * 4 + 4];
        o[0] = (c[0] as u32 * a / 255) as u8;
        o[1] = (c[1] as u32 * a / 255) as u8;
        o[2] = (c[2] as u32 * a / 255) as u8;
        o[3] = a as u8;
    }
    Some(pm)
}

/// Salin item riwayat ke clipboard. `owner` wajib window milik kita:
/// dengan owner NULL, EmptyClipboard membuat SetClipboardData gagal (data tidak pernah masuk).
pub fn set(owner: HWND, item: &Clip) -> bool {
    let (fmt_main, main, png): (u32, Vec<u8>, Option<&[u8]>) = match item {
        Clip::Text(t) => (
            CF_UNICODETEXT.0 as u32,
            t.encode_utf16().chain(std::iter::once(0)).flat_map(|u| u.to_le_bytes()).collect(),
            None,
        ),
        Clip::Image(img) => {
            let Ok(pm) = Pixmap::decode_png(&img.png) else { return false };
            (CF_DIB.0 as u32, dib_bytes(&pm), Some(&img.png))
        }
    };
    unsafe {
        // thread polling bisa sedang membuka clipboard -> coba beberapa kali
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(Some(owner)).is_ok() {
                opened = true;
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }
        if !opened {
            return false;
        }
        let ok = EmptyClipboard().is_ok() && put(fmt_main, &main).is_some();
        if let (true, Some(png)) = (ok, png) {
            // format "PNG" dipakai browser/Discord/Office, transparansi tetap terjaga
            let _ = put(RegisterClipboardFormatW(w!("PNG")), png);
        }
        let _ = CloseClipboard();
        ok
    }
}

unsafe fn put(format: u32, bytes: &[u8]) -> Option<()> {
    let g = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?;
    let p = GlobalLock(g) as *mut u8;
    if p.is_null() {
        return None;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    let _ = GlobalUnlock(g);
    SetClipboardData(format, Some(HANDLE(g.0))).ok()?;
    Some(())
}

/// Kirim Ctrl+V ke window yang sedang fokus (notch tidak mengambil fokus, jadi field user tetap aktif).
pub fn paste() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL,
    };
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() },
        },
    };
    let v = VIRTUAL_KEY(b'V' as u16);
    let seq = [key(VK_CONTROL, false), key(v, false), key(v, true), key(VK_CONTROL, true)];
    unsafe {
        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Apakah tombol Ctrl sedang ditahan (Ctrl+klik = salin saja, tanpa paste).
pub fn ctrl_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

/// Pixmap (premultiplied) -> CF_DIB: BITMAPINFOHEADER bottom-up, 32bpp, BI_RGB, piksel BGRA lurus.
fn dib_bytes(pm: &Pixmap) -> Vec<u8> {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let mut dib = Vec::with_capacity(40 + w * h * 4);
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&(w as i32).to_le_bytes());
    dib.extend_from_slice(&(h as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    dib.extend_from_slice(&((w * h * 4) as u32).to_le_bytes());
    dib.extend_from_slice(&[0u8; 16]); // ppm x/y, clrUsed, clrImportant
    let d = pm.data();
    for y in (0..h).rev() {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let a = d[i + 3] as u32;
            let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
            dib.extend_from_slice(&[un(d[i + 2]), un(d[i + 1]), un(d[i]), a as u8]);
        }
    }
    dib
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_roundtrip() {
        // gambar 5x3 opak dengan warna berbeda per piksel (lebar ganjil -> uji stride)
        let mut pm = Pixmap::new(5, 3).unwrap();
        for (i, px) in pm.data_mut().chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[(i * 17) as u8, (i * 31) as u8, (255 - i * 7) as u8, 255]);
        }
        let back = parse_dib(&dib_bytes(&pm)).expect("parse");
        assert_eq!((back.width(), back.height()), (5, 3));
        assert_eq!(back.data(), pm.data());
    }

    #[test]
    fn dib_24bit_top_down() {
        // 2x2, 24bpp, tinggi negatif (top-down), stride 8 byte (6 + padding 2)
        let mut b = Vec::new();
        for v in [40u32, 2, (-2i32) as u32] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0u8; 24]);
        b.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]); // baris atas: merah, hijau (BGR)
        b.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]); // baris bawah: biru, putih
        let pm = parse_dib(&b).expect("parse");
        let d = pm.data();
        assert_eq!(&d[0..4], &[255, 0, 0, 255]);
        assert_eq!(&d[4..8], &[0, 255, 0, 255]);
        assert_eq!(&d[8..12], &[0, 0, 255, 255]);
        assert_eq!(&d[12..16], &[255, 255, 255, 255]);
    }
}
