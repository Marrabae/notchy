//! Window X11: overlay ARGB (butuh compositor) yang digambar langsung dengan XPutImage.
//! Region input (XShape) dipangkas ke area piksel tak-transparan, jadi klik di area kosong tembus ke window di belakang.
use super::x11::{self, X};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tiny_skia::Pixmap;
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};
use x11rb::{
    connection::{Connection, RequestConnection},
    protocol::{
        randr::ConnectionExt as _,
        shape::{ConnectionExt as _, SK, SO},
        xproto::{
            AtomEnum, ClientMessageEvent, ClipOrdering, ConnectionExt as _, CreateGCAux, EventMask, Gcontext,
            ImageFormat, InputFocus, PropMode, Rectangle,
        },
    },
    wrapper::ConnectionExt as _,
};

pub type WinId = u32;

/// Window milik notchy (notch + PiP): dilewati saat mencari target drag-out.
static OWN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub fn own_windows() -> Vec<u32> {
    OWN.lock().map(|v| v.clone()).unwrap_or_default()
}

pub fn win_id(window: &Window) -> WinId {
    match window.window_handle().expect("window handle").as_raw() {
        RawWindowHandle::Xlib(h) => h.window as u32,
        RawWindowHandle::Xcb(h) => h.window.get(),
        _ => panic!("notchy di Linux butuh X11 (atau XWayland)"),
    }
}

/// Alpha minimum agar piksel ikut hit-test; sama dengan layered window Windows (alpha 0 = tembus).
/// Area hover mode peek sengaja digambar dengan alpha 1.
const HIT_ALPHA: u8 = 1;

struct Win {
    gc: Gcontext,
    depth: u8,
    /// region input yang terakhir dipasang (x, y, w, h)
    shape: Option<(i16, i16, u16, u16)>,
}

thread_local! {
    static X11: RefCell<Option<X>> = const { RefCell::new(None) };
    static WINS: RefCell<HashMap<u32, Win>> = RefCell::default();
    static BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// fokus sebelum notch mengambil fokus (untuk todo), dikembalikan setelah selesai
    static PREV_FOCUS: Cell<u32> = const { Cell::new(0) };
    static NEXT_FRAME: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Pakai koneksi X milik thread ini (dibuat saat pertama dipakai).
fn with_x<R>(f: impl FnOnce(&X) -> R) -> Option<R> {
    X11.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = x11::connect();
        }
        c.as_ref().map(f)
    })
}

/// Kirim client message ke root (permintaan ke window manager).
fn wm_message(x: &X, win: u32, ty: &str, data: [u32; 5]) {
    let ev = ClientMessageEvent::new(32, win, x.atom(ty), data);
    let _ = x.conn.send_event(false, x.root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev);
}

/// WM_HINTS minimal: hanya flag InputHint. input=false -> klik tidak memindahkan fokus keyboard ke notch.
fn set_input_hint(x: &X, win: u32, input: bool) {
    let _ = x.conn.change_property32(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_HINTS,
        AtomEnum::WM_HINTS,
        &[1, input as u32, 0, 0, 0, 0, 0, 0, 0],
    );
}

/// Jadikan window overlay: selalu di atas, di semua workspace, tanpa taskbar/pager, tanpa fokus.
pub fn make_overlay(win: WinId) {
    with_x(|x| {
        let a = |n: &str| x.atom(n);
        // window yang belum dikelola WM mengabaikan client message: set juga propertinya langsung
        let _ = x.conn.change_property32(
            PropMode::REPLACE,
            win,
            a("_NET_WM_STATE"),
            AtomEnum::ATOM,
            &[a("_NET_WM_STATE_ABOVE"), a("_NET_WM_STATE_SKIP_TASKBAR"), a("_NET_WM_STATE_SKIP_PAGER"), a("_NET_WM_STATE_STICKY")],
        );
        // 1 = _NET_WM_STATE_ADD, 1 = sumber aplikasi
        wm_message(x, win, "_NET_WM_STATE", [1, a("_NET_WM_STATE_ABOVE"), a("_NET_WM_STATE_SKIP_TASKBAR"), 1, 0]);
        wm_message(x, win, "_NET_WM_STATE", [1, a("_NET_WM_STATE_SKIP_PAGER"), a("_NET_WM_STATE_STICKY"), 1, 0]);
        set_input_hint(x, win, false);
        let _ = x.conn.flush();
    });
    if let Ok(mut v) = OWN.lock() {
        if !v.contains(&win) {
            v.push(win);
        }
    }
}

/// Izinkan/larang window menerima fokus keyboard (untuk mengetik di todo).
pub fn set_activatable(win: WinId, on: bool) {
    with_x(|x| {
        set_input_hint(x, win, on);
        if on {
            let prev = x.conn.get_input_focus().ok().and_then(|c| c.reply().ok()).map(|r| r.focus).unwrap_or(0);
            if prev != win {
                PREV_FOCUS.with(|p| p.set(prev));
            }
            wm_message(x, win, "_NET_ACTIVE_WINDOW", [1, 0, 0, 0, 0]);
            let _ = x.conn.set_input_focus(InputFocus::PARENT, win, x11rb::CURRENT_TIME);
        } else {
            let prev = PREV_FOCUS.with(|p| p.replace(0));
            // kembalikan fokus ke window sebelumnya (0 / NONE = biarkan window manager yang memilih)
            if prev > 1 {
                let _ = x.conn.set_input_focus(InputFocus::PARENT, prev, x11rb::CURRENT_TIME);
            } else {
                let _ = x.conn.set_input_focus(InputFocus::POINTER_ROOT, x11rb::NONE, x11rb::CURRENT_TIME);
            }
        }
        let _ = x.conn.flush();
    });
}

/// Lepas resource milik window yang sudah ditutup.
pub fn forget(win: WinId) {
    if let Some(w) = WINS.with(|c| c.borrow_mut().remove(&win)) {
        with_x(|x| {
            let _ = x.conn.free_gc(w.gc);
            let _ = x.conn.flush();
        });
    }
    if let Ok(mut v) = OWN.lock() {
        v.retain(|w| *w != win);
    }
}

/// Kirim pixmap (RGBA premultiplied) ke window (BGRA premultiplied = ARGB little-endian).
pub fn present(win: WinId, pm: &Pixmap) {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    if w == 0 || h == 0 {
        return;
    }
    with_x(|x| {
        WINS.with(|c| {
            let mut c = c.borrow_mut();
            if !c.contains_key(&win) {
                let Some(depth) = x.conn.get_geometry(win).ok().and_then(|r| r.reply().ok()).map(|g| g.depth) else {
                    return;
                };
                let gc = x.conn.generate_id().unwrap_or(0);
                if x.conn.create_gc(gc, win, &CreateGCAux::new()).is_err() {
                    return;
                }
                c.insert(win, Win { gc, depth, shape: None });
            }
            let ws = c.get_mut(&win).unwrap();
            BUF.with(|b| {
                let mut buf = b.borrow_mut();
                buf.resize(w * h * 4, 0);
                // RGBA -> BGRA (tukar byte R dan B) sambil mencari kotak piksel yang terlihat
                let src = pm.data();
                let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
                for y in 0..h {
                    let (srow, drow) = (&src[y * w * 4..(y + 1) * w * 4], &mut buf[y * w * 4..(y + 1) * w * 4]);
                    for (i, (s, d)) in srow.chunks_exact(4).zip(drow.chunks_exact_mut(4)).enumerate() {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                        d[3] = s[3];
                        if s[3] >= HIT_ALPHA {
                            x0 = x0.min(i);
                            x1 = x1.max(i);
                            y0 = y0.min(y);
                            y1 = y1.max(y);
                        }
                    }
                }
                let rect = (x0 != usize::MAX).then(|| (x0 as i16, y0 as i16, (x1 - x0 + 1) as u16, (y1 - y0 + 1) as u16));
                if rect != ws.shape {
                    ws.shape = rect;
                    let rects: Vec<Rectangle> =
                        rect.into_iter().map(|(x, y, width, height)| Rectangle { x, y, width, height }).collect();
                    let _ = x.conn.shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, win, 0, 0, &rects);
                }
                // satu request PutImage dibatasi ukuran maksimum request: kirim per potongan baris
                let max = x.conn.maximum_request_bytes().saturating_sub(64);
                let rows = (max / (w * 4)).clamp(1, h);
                let mut y = 0;
                while y < h {
                    let n = rows.min(h - y);
                    let _ = x.conn.put_image(
                        ImageFormat::Z_PIXMAP,
                        win,
                        ws.gc,
                        w as u16,
                        n as u16,
                        0,
                        y as i16,
                        0,
                        ws.depth,
                        &buf[y * w * 4..(y + n) * w * 4],
                    );
                    y += n;
                }
                let _ = x.conn.flush();
            });
        });
    });
}

/// Periode refresh monitor tercepat (RandR); 60 Hz kalau tidak terbaca.
fn frame_period() -> Duration {
    static P: OnceLock<Duration> = OnceLock::new();
    *P.get_or_init(|| {
        let hz = (|| {
            let x = x11::connect()?;
            let res = x.conn.randr_get_screen_resources_current(x.root).ok()?.reply().ok()?;
            let mut best = 0.0f64;
            for crtc in &res.crtcs {
                let info = x.conn.randr_get_crtc_info(*crtc, res.config_timestamp).ok()?.reply().ok()?;
                if let Some(m) = res.modes.iter().find(|m| m.id == info.mode && info.mode != 0) {
                    let total = m.htotal as f64 * m.vtotal as f64;
                    if total > 0.0 {
                        best = best.max(m.dot_clock as f64 / total);
                    }
                }
            }
            (best >= 24.0).then_some(best)
        })()
        .unwrap_or(60.0);
        Duration::from_secs_f64(1.0 / hz.min(240.0))
    })
}

/// Atur laju frame ke refresh rate monitor (tanpa vsync asli: tidur sampai slot frame berikutnya).
pub fn wait_vsync() {
    let period = frame_period();
    let now = Instant::now();
    let next = NEXT_FRAME.with(|n| n.get()).filter(|t| *t + period > now).unwrap_or(now);
    if next > now {
        std::thread::sleep(next - now);
    }
    NEXT_FRAME.with(|n| n.set(Some(next + period)));
}
