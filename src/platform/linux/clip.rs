//! Clipboard Linux: arboard (CLIPBOARD selection) untuk baca/tulis; perubahan dideteksi lewat XFixes
//! (event saat pemilik selection berganti, tanpa polling). Paste = Ctrl+V sintetis lewat XTest.
use super::x11::{self, XK_CONTROL_L, XK_V};
use crate::{
    clip::{make_image, push, MAX_CHARS},
    state::{Clip, State},
};
use arboard::{Clipboard, ImageData};
use std::{
    borrow::Cow,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    thread,
    time::{Duration, Instant},
};
use tiny_skia::Pixmap;
use x11rb::{
    connection::Connection,
    protocol::{
        xfixes::{ConnectionExt as _, SelectionEventMask},
        xproto::{AtomEnum, ConnectionExt as _, CreateWindowAux, KeyButMask, WindowClass},
        xtest::ConnectionExt as _,
        Event,
    },
    COPY_DEPTH_FROM_PARENT,
};

/// Handle arboard untuk `set`: di X11 data yang kita set hanya tersedia selama handle ini hidup.
/// Thread pembaca punya handle sendiri, jadi pemilik clipboard yang hang (arboard menunggu s.d. 4 detik)
/// tidak menahan klik paste di UI.
fn board() -> Option<MutexGuard<'static, Clipboard>> {
    static B: OnceLock<Option<Mutex<Clipboard>>> = OnceLock::new();
    B.get_or_init(|| Clipboard::new().ok().map(Mutex::new)).as_ref()?.lock().ok()
}

pub fn spawn(state: State) {
    thread::spawn(move || {
        let Some(x) = x11::connect() else { return };
        // window kecil tak terlihat sebagai penerima event XFixes
        let Ok(win) = x.conn.generate_id() else { return };
        let _ = x.conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            win,
            x.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new(),
        );
        let _ = x.conn.xfixes_query_version(5, 0).map(|c| c.reply());
        let clipboard = x.atom("CLIPBOARD");
        let _ = x.conn.xfixes_select_selection_input(win, clipboard, SelectionEventMask::SET_SELECTION_OWNER);
        let _ = x.conn.flush();
        let Ok(mut cb) = Clipboard::new() else { return };

        loop {
            // isi clipboard saat start ikut terbaca; sesudahnya hanya saat pemilik berganti
            if !is_secret(&x, win) {
                if let Some(item) = read(&mut cb) {
                    push(&state, item);
                }
            }
            loop {
                match x.conn.wait_for_event() {
                    Ok(Event::XfixesSelectionNotify(_)) => break,
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
            // pemilik baru biasanya belum siap melayani konversi pada saat event datang; kumpulkan event beruntun
            thread::sleep(Duration::from_millis(120));
            while let Ok(Some(_)) = x.conn.poll_for_event() {}
        }
    });
}

/// Password manager (KeePassXC, Bitwarden, dll) menandai isi sensitif lewat target `x-kde-passwordManagerHint` = "secret".
fn is_secret(x: &x11::X, win: u32) -> bool {
    let (target, prop) = (x.atom("x-kde-passwordManagerHint"), x.atom("NOTCHY_HINT"));
    let _ = x.conn.convert_selection(win, x.atom("CLIPBOARD"), target, prop, x11rb::CURRENT_TIME);
    let _ = x.conn.flush();
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        match x.conn.poll_for_event() {
            Ok(Some(Event::SelectionNotify(e))) if e.requestor == win => {
                if e.property == x11rb::NONE {
                    return false;
                }
                let v = x.conn.get_property(true, win, prop, AtomEnum::ANY, 0, 16).ok().and_then(|c| c.reply().ok());
                return v.is_some_and(|r| r.value == b"secret");
            }
            Ok(Some(_)) => {}
            _ => thread::sleep(Duration::from_millis(5)),
        }
    }
    false
}

/// Baca isi clipboard: teks diutamakan, lalu gambar.
fn read(b: &mut Clipboard) -> Option<Clip> {
    if let Ok(t) = b.get_text() {
        return (!t.trim().is_empty()).then(|| Clip::Text(t.chars().take(MAX_CHARS).collect()));
    }
    let img = b.get_image().ok()?;
    let (w, h) = (img.width as u32, img.height as u32);
    if w == 0 || h == 0 || w > 16384 || h > 16384 || img.bytes.len() < (w * h * 4) as usize {
        return None;
    }
    // arboard: RGBA lurus (non-premultiplied) -> premultiplied untuk tiny-skia
    let mut pm = Pixmap::new(w, h)?;
    for (s, d) in img.bytes.chunks_exact(4).zip(pm.data_mut().chunks_exact_mut(4)) {
        let a = s[3] as u32;
        d[0] = (s[0] as u32 * a / 255) as u8;
        d[1] = (s[1] as u32 * a / 255) as u8;
        d[2] = (s[2] as u32 * a / 255) as u8;
        d[3] = s[3];
    }
    Some(Clip::Image(Arc::new(make_image(pm)?)))
}

/// Salin item riwayat ke clipboard. `owner` tidak dipakai di X11 (arboard memegang ownership sendiri).
pub fn set(_owner: super::WinId, item: &Clip) -> bool {
    let Some(mut b) = board() else { return false };
    match item {
        Clip::Text(t) => b.set_text(t.clone()).is_ok(),
        Clip::Image(img) => {
            let Ok(pm) = Pixmap::decode_png(&img.png) else { return false };
            let mut bytes = Vec::with_capacity(pm.data().len());
            for p in pm.data().chunks_exact(4) {
                let a = p[3] as u32;
                let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a / 2) / a).min(255) as u8 };
                bytes.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), p[3]]);
            }
            b.set_image(ImageData { width: pm.width() as usize, height: pm.height() as usize, bytes: Cow::Owned(bytes) })
                .is_ok()
        }
    }
}

/// Kirim Ctrl+V ke window yang sedang fokus (notch tidak mengambil fokus, jadi field user tetap aktif).
pub fn paste() {
    let Some(x) = x11::connect() else { return };
    let (Some(ctrl), Some(v)) = (x.keycode(XK_CONTROL_L), x.keycode(XK_V)) else { return };
    const PRESS: u8 = 2; // KeyPress
    const RELEASE: u8 = 3; // KeyRelease
    for (ty, key) in [(PRESS, ctrl), (PRESS, v), (RELEASE, v), (RELEASE, ctrl)] {
        let _ = x.conn.xtest_fake_input(ty, key, 0, x.root, 0, 0, 0);
    }
    let _ = x.conn.flush();
    // pastikan request terkirim sebelum koneksi ditutup
    let _ = x.conn.get_input_focus().map(|c| c.reply());
}

/// Apakah tombol Ctrl sedang ditahan (Ctrl+klik = salin saja, tanpa paste).
pub fn ctrl_down() -> bool {
    let Some(x) = x11::connect() else { return false };
    x.conn
        .query_pointer(x.root)
        .ok()
        .and_then(|c| c.reply().ok())
        .is_some_and(|r| r.mask.contains(KeyButMask::CONTROL))
}
