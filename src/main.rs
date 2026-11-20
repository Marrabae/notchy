#![windows_subsystem = "windows"]

mod bright;
mod clip;
mod devices;
mod dragout;
mod gfx;
mod media;
mod state;
mod sys;
mod timer;
mod todo;

use gfx::{Ctx, Fonts};
use state::{Clip, Cmd, DevKind, Device, Net, NetKind, Power, Shared, State, SysCmd};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use tiny_skia::Pixmap;
use timer::{Phase, Pomo};
use todo::Todos;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    platform::windows::WindowAttributesExtWindows,
    window::{CursorIcon, ResizeDirection, Window, WindowId, WindowLevel},
};

// ---------------------------------------------------------------- ukuran (logical)
const WIN_W: f32 = 600.0;
const WIN_H: f32 = 224.0;
const PIP_W: f32 = 380.0;
const PIP_H: f32 = 160.0;
const EXP: (f32, f32) = (460.0, 162.0); // expanded biasa
const LYR: (f32, f32) = (580.0, 146.0); // expanded + lirik inline
const STA: (f32, f32) = (520.0, 198.0); // halaman system info

const WHITE: [u8; 4] = [255, 255, 255, 255];
const GRAY: [u8; 4] = [150, 150, 158, 255];
const DIM: [u8; 4] = [95, 95, 104, 255];
const ACCENT: [u8; 4] = [120, 200, 255, 255];
const CHIP: [u8; 4] = [48, 48, 54, 255];
const CHIP_HI: [u8; 4] = [66, 66, 74, 255];
const TRACK: [u8; 4] = [70, 70, 76, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const GREEN: [u8; 4] = [80, 220, 110, 255];
const RED: [u8; 4] = [240, 90, 90, 255];
const ORANGE: [u8; 4] = [255, 140, 80, 255];

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Page {
    Media,
    Shelf,
    Clipboard,
    Todo,
    Timer,
    Status,
}
const PAGES: [Page; 6] = [Page::Media, Page::Shelf, Page::Clipboard, Page::Todo, Page::Timer, Page::Status];

type Rect = (f32, f32, f32, f32);

fn inside(p: (f32, f32), r: Rect) -> bool {
    p.0 >= r.0 && p.0 <= r.0 + r.2 && p.1 >= r.1 && p.1 <= r.1 + r.3
}
fn near(p: (f32, f32), c: (f32, f32), r: f32) -> bool {
    (p.0 - c.0).hypot(p.1 - c.1) <= r
}
fn grow(r: Rect, dx: f32, dy: f32) -> Rect {
    (r.0 - dx, r.1 - dy, r.2 + 2.0 * dx, r.3 + 2.0 * dy)
}

// ---------------------------------------------------------------- layout (relatif kiri pill x0)
mod lay {
    use super::Rect;
    // mode expanded biasa (EXP)
    pub fn arrows(x0: f32, w: f32) -> [(f32, f32); 2] {
        [(x0 + 16.0, 72.0), (x0 + w - 16.0, 72.0)]
    }
    pub const ART: (f32, f32, f32) = (34.0, 10.0, 62.0); // dx, y, size
    pub const PROG_Y: f32 = 90.0;
    pub const CTRL_Y: f32 = 124.0;
    pub fn bar(x0: f32, w: f32) -> Rect {
        (x0 + 84.0, PROG_Y + 6.0, w - 84.0 - 92.0, 4.0)
    }
    pub fn lyric_icon(x0: f32) -> (f32, f32) {
        (x0 + 60.0, CTRL_Y)
    }
    pub fn shelf(x0: f32, w: f32, i: usize) -> Rect {
        let cw = (w - 68.0 - 3.0 * 6.0) / 4.0;
        (x0 + 34.0 + (i % 4) as f32 * (cw + 6.0), 34.0 + (i / 4) as f32 * 34.0, cw, 28.0)
    }
    // halaman list (clipboard / todo)
    pub const ROWS: usize = 4;
    pub fn row(x0: f32, w: f32, i: usize) -> Rect {
        (x0 + 34.0, 34.0 + i as f32 * 27.0, w - 68.0, 24.0)
    }
    pub fn head_btn(x0: f32, w: f32, bw: f32) -> Rect {
        (x0 + w - 34.0 - bw, 8.0, bw, 20.0)
    }
    // timer
    pub const RING: (f32, f32, f32) = (96.0, 80.0, 40.0); // dx, y, r
    pub fn setting_minus(x0: f32, w: f32, i: usize) -> Rect {
        (x0 + w - 34.0 - 54.0, 34.0 + i as f32 * 20.0, 24.0, 17.0)
    }
    pub fn setting_plus(x0: f32, w: f32, i: usize) -> Rect {
        (x0 + w - 34.0 - 26.0, 34.0 + i as f32 * 20.0, 24.0, 17.0)
    }
    pub fn timer_btn(x0: f32, i: usize) -> Rect {
        let xs = [176.0, 258.0, 330.0];
        let ws = [76.0, 66.0, 60.0];
        (x0 + xs[i], 118.0, ws[i], 22.0)
    }
    // system info (STA): grid 2 kolom
    fn st_cw(w: f32) -> f32 {
        (w - 68.0 - 10.0) / 2.0
    }
    pub fn st_battery(x0: f32, w: f32) -> Rect {
        (x0 + 34.0, 10.0, st_cw(w), 66.0)
    }
    pub fn st_net(x0: f32, w: f32) -> Rect {
        (x0 + 44.0 + st_cw(w), 10.0, st_cw(w), 66.0)
    }
    pub fn st_volume(x0: f32, w: f32) -> Rect {
        (x0 + 34.0, 82.0, st_cw(w), 44.0)
    }
    pub fn st_bright(x0: f32, w: f32) -> Rect {
        (x0 + 44.0 + st_cw(w), 82.0, st_cw(w), 44.0)
    }
    pub fn st_devices(x0: f32, w: f32) -> Rect {
        (x0 + 34.0, 132.0, w - 68.0 - 54.0, 44.0)
    }
    pub fn quit(x0: f32, w: f32) -> Rect {
        (x0 + w - 34.0 - 44.0, 132.0, 44.0, 44.0)
    }
    /// track slider di dalam tile volume/brightness
    pub fn slider(r: Rect) -> Rect {
        (r.0 + 42.0, r.1 + 27.0, r.2 - 56.0, 7.0)
    }
    /// ikon slider (klik ikon volume = mute)
    pub fn slider_icon(r: Rect) -> (f32, f32) {
        (r.0 + 21.0, r.1 + r.3 / 2.0)
    }
    // mode lirik inline (LYR)
    pub const L_ART: (f32, f32, f32) = (22.0, 12.0, 50.0);
    pub const L_PROG_Y: f32 = 74.0;
    pub const L_CTRL_Y: f32 = 112.0;
    pub const L_SPLIT: f32 = 300.0;
    pub fn l_bar(x0: f32) -> Rect {
        (x0 + 68.0, L_PROG_Y + 6.0, L_SPLIT - 68.0 - 72.0, 4.0)
    }
    pub fn l_ctrls(x0: f32) -> [(f32, f32); 4] {
        // lyric icon, prev, play, next
        [(x0 + 36.0, L_CTRL_Y), (x0 + 88.0, L_CTRL_Y), (x0 + 134.0, L_CTRL_Y), (x0 + 180.0, L_CTRL_Y)]
    }
    pub fn l_popout(x0: f32, w: f32) -> Rect {
        (x0 + w - 34.0, 6.0, 22.0, 20.0)
    }
}

// ---------------------------------------------------------------- util teks
#[allow(clippy::too_many_arguments)]
fn center(ctx: &mut Ctx, t: &str, cx: f32, y: f32, px: f32, c: [u8; 4], max_w: f32, bold: bool) {
    let f = if bold { ctx.bold } else { ctx.fonts };
    let t = f.fit(t, px * ctx.s, max_w * ctx.s);
    let tw = f.width(&t, px * ctx.s) / ctx.s;
    if bold {
        ctx.text_b(&t, cx - tw / 2.0, y, px, c, max_w);
    } else {
        ctx.text(&t, cx - tw / 2.0, y, px, c, max_w);
    }
}

fn text_w(ctx: &Ctx, t: &str, px: f32, bold: bool) -> f32 {
    let f = if bold { ctx.bold } else { ctx.fonts };
    f.width(t, px * ctx.s) / ctx.s
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn fade(c: [u8; 4], f: f32) -> [u8; 4] {
    [c[0], c[1], c[2], (c[3] as f32 * f) as u8]
}

fn mmss(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn local_time() -> (String, String) {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    const D: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    (
        format!("{} {} {} {}", D[t.wDayOfWeek as usize % 7], t.wDay, M[(t.wMonth as usize + 11) % 12], t.wYear),
        format!("{:02}:{:02}", t.wHour, t.wMinute),
    )
}

/// Chip tombol berlabel (logical).
fn button(ctx: &mut Ctx, r: Rect, label: &str, bg: [u8; 4], fg: [u8; 4], a: f32) {
    ctx.rrect(r.0, r.1, r.2, r.3, r.3 / 2.0, fade(bg, a));
    center(ctx, label, r.0 + r.2 / 2.0, r.1 + (r.3 - 12.0) / 2.0 - 1.0, 12.0, fade(fg, a), r.2 - 6.0, false);
}

// ---------------------------------------------------------------- ikon
fn icon_prev(ctx: &mut Ctx, x: f32, y: f32, c: [u8; 4]) {
    ctx.poly(&[(x + 1.0, y - 6.5), (x + 1.0, y + 6.5), (x - 8.0, y)], c);
    ctx.poly(&[(x + 9.0, y - 6.5), (x + 9.0, y + 6.5), (x, y)], c);
}
fn icon_next(ctx: &mut Ctx, x: f32, y: f32, c: [u8; 4]) {
    ctx.poly(&[(x - 1.0, y - 6.5), (x - 1.0, y + 6.5), (x + 8.0, y)], c);
    ctx.poly(&[(x - 9.0, y - 6.5), (x - 9.0, y + 6.5), (x, y)], c);
}
fn icon_play(ctx: &mut Ctx, x: f32, y: f32, playing: bool, c: [u8; 4]) {
    if playing {
        ctx.rrect(x - 6.5, y - 8.0, 4.5, 16.0, 1.5, c);
        ctx.rrect(x + 2.0, y - 8.0, 4.5, 16.0, 1.5, c);
    } else {
        ctx.poly(&[(x - 5.0, y - 8.0), (x - 5.0, y + 8.0), (x + 8.0, y)], c);
    }
}
/// Gelembung kutipan (ikon lirik)
fn icon_lyrics(ctx: &mut Ctx, x: f32, y: f32, c: [u8; 4]) {
    ctx.rrect(x - 9.0, y - 7.5, 18.0, 13.0, 4.0, c);
    ctx.poly(&[(x - 5.0, y + 5.0), (x - 6.5, y + 9.5), (x + 0.5, y + 5.0)], c);
    let d = [0, 0, 0, c[3]];
    ctx.circle(x - 3.0, y - 1.5, 1.8, d);
    ctx.circle(x + 3.0, y - 1.5, 1.8, d);
    ctx.poly(&[(x - 4.6, y - 1.0), (x - 1.4, y - 1.0), (x - 3.6, y + 2.6)], d);
    ctx.poly(&[(x + 1.4, y - 1.0), (x + 4.6, y - 1.0), (x + 2.4, y + 2.6)], d);
}
fn icon_chevron(ctx: &mut Ctx, x: f32, y: f32, left: bool, a: f32) {
    ctx.circle(x, y, 9.0, fade(CHIP, a));
    let s = if left { -1.0 } else { 1.0 };
    ctx.poly(&[(x - 2.0 * s, y - 4.0), (x - 2.0 * s, y + 4.0), (x + 3.0 * s, y)], fade(WHITE, a));
}
fn icon_battery(ctx: &mut Ctx, x: f32, y: f32, pct: u8, chg: bool, a: f32) {
    let col = if chg { GREEN } else if pct <= 20 { RED } else { WHITE };
    ctx.rrect(x, y, 21.0, 11.0, 3.0, fade(GRAY, a));
    ctx.rrect(x + 1.2, y + 1.2, 18.6, 8.6, 2.2, fade(BLACK, a));
    ctx.rrect(x + 2.2, y + 2.2, 16.6 * (pct as f32 / 100.0).clamp(0.08, 1.0), 6.6, 1.5, fade(col, a));
    ctx.rrect(x + 21.5, y + 3.5, 1.8, 4.0, 0.8, fade(GRAY, a));
}
/// Visualizer 4 bar (animasi sintetis saat playing)
fn icon_bars(ctx: &mut Ctx, x: f32, y_bottom: f32, h: f32, t: f32, playing: bool, c: [u8; 4]) {
    const SPD: [f32; 4] = [7.1, 9.3, 6.2, 8.4];
    const PH: [f32; 4] = [0.0, 1.7, 3.1, 4.4];
    for i in 0..4 {
        let k = if playing { ((t * SPD[i] + PH[i]).sin() * 0.5 + 0.5) * 0.85 + 0.15 } else { 0.2 };
        let bh = (h * k).max(3.0);
        ctx.rrect(x + i as f32 * 5.0, y_bottom - bh, 3.0, bh, 1.5, c);
    }
}
/// Cincin baterai: busur = persen, petir di tengah saat charging.
fn icon_battery_ring(ctx: &mut Ctx, x: f32, y: f32, pct: u8, chg: bool, a: f32) {
    let col = fade(if chg { GREEN } else if pct <= 20 { RED } else { WHITE }, a);
    ctx.arc(x, y, 8.0, 2.2, 1.0, fade([60, 60, 66, 255], a));
    ctx.arc(x, y, 8.0, 2.2, pct as f32 / 100.0, col);
    if chg {
        ctx.poly(&[(x + 1.2, y - 5.5), (x - 3.2, y + 0.8), (x - 0.2, y + 0.8), (x - 1.2, y + 5.5), (x + 3.2, y - 0.8), (x + 0.2, y - 0.8)], col);
    }
}
/// Not musik kecil (pill lirik)
fn icon_note(ctx: &mut Ctx, x: f32, y: f32, c: [u8; 4]) {
    ctx.circle(x - 2.5, y + 4.0, 3.4, c);
    ctx.rrect(x - 0.2, y - 8.0, 1.9, 12.5, 0.8, c);
    ctx.poly(&[(x + 0.5, y - 8.0), (x + 6.5, y - 5.0), (x + 6.5, y - 2.0), (x + 0.5, y - 4.6)], c);
}
fn icon_popout(ctx: &mut Ctx, r: Rect, on: bool, a: f32) {
    let (x, y, _, _) = r;
    let c = fade(if on { ACCENT } else { GRAY }, a);
    ctx.rrect(x + 3.0, y + 5.0, 14.0, 11.0, 2.5, c);
    ctx.rrect(x + 4.5, y + 6.5, 11.0, 8.0, 1.5, fade(BLACK, a));
    ctx.rrect(x + 10.0, y + 2.0, 9.0, 7.0, 1.5, c);
}
fn icon_check(ctx: &mut Ctx, x: f32, y: f32, done: bool, a: f32) {
    if done {
        ctx.circle(x, y, 7.5, fade(GREEN, a));
        ctx.poly(&[(x - 4.0, y), (x - 1.0, y + 3.2), (x + 4.2, y - 2.8), (x + 3.0, y - 4.0), (x - 1.0, y + 0.8), (x - 2.8, y - 1.2)], fade(BLACK, a));
    } else {
        ctx.arc(x, y, 6.5, 1.6, 1.0, fade(GRAY, a));
    }
}
fn phase_color(p: Phase) -> [u8; 4] {
    match p {
        Phase::Focus => ORANGE,
        Phase::Break => GREEN,
        Phase::LongBreak => ACCENT,
    }
}

// ---------------------------------------------------------------- window
struct Win {
    window: Window,
    hwnd: windows::Win32::Foundation::HWND,
    pm: Pixmap,
    /// kanvas konten notch sebelum dipotong mask bentuk
    layer: Option<Pixmap>,
    s: f32,
}

impl Win {
    fn new(el: &ActiveEventLoop, w: f32, h: f32, pos: impl FnOnce(f32, &ActiveEventLoop) -> PhysicalPosition<i32>) -> Self {
        let s = el.primary_monitor().map(|m| m.scale_factor() as f32).unwrap_or(1.0);
        let attrs = Window::default_attributes()
            .with_title("notchy")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_skip_taskbar(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(PhysicalSize::new((w * s) as u32, (h * s) as u32))
            .with_position(pos(s, el));
        let window = el.create_window(attrs).expect("gagal bikin window");
        let hwnd = gfx::hwnd_of(&window);
        gfx::make_overlay(hwnd);
        let pm = Pixmap::new((w * s) as u32, (h * s) as u32).unwrap();
        Win { window, hwnd, pm, layer: None, s }
    }
}

#[derive(Default)]
struct Spring {
    cur: f32,
    vel: f32,
}

impl Spring {
    /// Spring umum (scroll lirik).
    fn step(&mut self, target: f32, dt: f32) {
        self.step_with(target, dt, 260.0, 0.95);
    }

    /// Spring bentuk notch: membesar sedikit memantul, mengecil tanpa overshoot (tidak "patah").
    fn step_shape(&mut self, target: f32, dt: f32) {
        if target >= self.cur {
            self.step_with(target, dt, 150.0, 0.78);
        } else {
            self.step_with(target, dt, 165.0, 1.0);
        }
    }

    /// k = kekakuan, zeta = rasio redaman (1.0 = critically damped). Substep <= 4 ms agar stabil.
    fn step_with(&mut self, target: f32, dt: f32, k: f32, zeta: f32) {
        let c = 2.0 * zeta * k.sqrt();
        let n = (dt / 0.004).ceil().max(1.0) as usize;
        let h = dt / n as f32;
        for _ in 0..n {
            self.vel += ((target - self.cur) * k - self.vel * c) * h;
            self.cur += self.vel * h;
        }
    }
    fn settled(&self, target: f32) -> bool {
        (target - self.cur).abs() < 0.25 && self.vel.abs() < 0.5
    }
}

/// State animasi scroll daftar lirik.
struct LyricScroll {
    idx: isize,
    off: Spring,
    frame: Instant,
}

impl Default for LyricScroll {
    fn default() -> Self {
        LyricScroll { idx: -1, off: Spring::default(), frame: Instant::now() }
    }
}

/// Gaya daftar lirik (semua logical).
struct ListStyle {
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    cpx: f32,
    opx: f32,
    gap: f32,
    anchor: f32,
    centered: bool,
}

/// Area klik baris lirik -> timestamp (ms).
type LyricHits = Vec<(Rect, i64)>;

/// Gambar daftar lirik yang menggulung, baris aktif bold putih di `anchor`.
fn draw_lyric_list(ctx: &mut Ctx, snap: &Snap, st: &ListStyle, sc: &mut LyricScroll, a: f32, hits: &mut LyricHits) {
    hits.clear();
    let s = ctx.s;
    let (reg, bold) = (ctx.fonts, ctx.bold);
    let lines_of = |i: isize, active: bool| -> std::rc::Rc<Vec<String>> {
        let t = if i < 0 { "…" } else { snap.all.get(i as usize).map(|s| s.as_str()).unwrap_or("") };
        let t = if t.trim().is_empty() { "…" } else { t };
        let (f, px) = if active { (bold, st.cpx) } else { (reg, st.opx) };
        f.wrap(t, px * s, st.w * s)
    };
    let bh = |n: usize, px: f32| n as f32 * px * 1.3;

    let now = Instant::now();
    let dt = now.duration_since(sc.frame).as_secs_f32().min(0.05);
    sc.frame = now;
    let idx = snap.idx;
    if idx != sc.idx {
        if idx == sc.idx + 1 {
            let old = bh(lines_of(sc.idx, false).len(), st.opx);
            let new = bh(lines_of(idx, true).len(), st.cpx);
            sc.off.cur += old / 2.0 + st.gap + new / 2.0;
        } else {
            sc.off = Spring::default();
        }
        sc.idx = idx;
    }
    sc.off.step(0.0, dt);

    let area = st.bottom - st.top;
    let anchor = st.top + area * st.anchor + sc.off.cur;
    let mut put = |ctx: &mut Ctx, i: isize, lines: &[String], y0: f32, px: f32, c: [u8; 4], active: bool| {
        let mut y = y0;
        for l in lines {
            if y >= st.top - 1.0 && y + px * 1.15 <= st.bottom + 1.0 {
                let f = if active { 1.0 } else { (1.0 - (y - anchor).abs() / (area * 0.9)).clamp(0.25, 1.0) };
                let col = fade(c, f * a);
                if st.centered {
                    center(ctx, l, st.x + st.w / 2.0, y, px, col, st.w, active);
                } else if active {
                    ctx.text_b(l, st.x, y, px, col, st.w);
                } else {
                    ctx.text(l, st.x, y, px, col, st.w);
                }
            }
            y += px * 1.3;
        }
        if let Some(&t) = (i >= 0).then(|| snap.times.get(i as usize)).flatten() {
            hits.push(((st.x, y0 - 2.0, st.w, y - y0 + 2.0), t));
        }
    };

    let cur = lines_of(idx, true);
    let ch = bh(cur.len(), st.cpx);
    let cur_top = anchor - ch / 2.0;
    put(ctx, idx, &cur, cur_top, st.cpx, WHITE, true);

    let mut y = cur_top + ch + st.gap;
    let mut i = idx + 1;
    while y < st.bottom && (i as usize) < snap.all.len() {
        let ls = lines_of(i, false);
        put(ctx, i, &ls, y, st.opx, GRAY, false);
        y += bh(ls.len(), st.opx) + st.gap;
        i += 1;
    }
    let mut y = cur_top - st.gap;
    let mut i = idx - 1;
    while y > st.top && i >= 0 {
        let ls = lines_of(i, false);
        y -= bh(ls.len(), st.opx);
        put(ctx, i, &ls, y, st.opx, GRAY, false);
        y -= st.gap;
        i -= 1;
    }
}

/// State transisi baris lirik di pill collapsed.
struct PillLyric {
    idx: isize,
    cur: String,
    prev: String,
    at: Instant,
}

impl Default for PillLyric {
    fn default() -> Self {
        PillLyric { idx: -2, cur: String::new(), prev: String::new(), at: Instant::now() }
    }
}

fn pill_text(snap: &Snap) -> String {
    if !snap.has_lyrics {
        return "Lyrics not available".into();
    }
    match snap.all.get(snap.idx.max(0) as usize) {
        Some(l) if snap.idx >= 0 && !l.trim().is_empty() => l.clone(),
        _ => "…".into(),
    }
}

// ---------------------------------------------------------------- snapshot state
struct Snap {
    title: String,
    artist: String,
    playing: bool,
    pos: i64,
    dur: i64,
    has_lyrics: bool,
    all: Vec<String>,
    times: Vec<i64>,
    idx: isize,
    activity: Option<String>,
    shelf: Vec<String>,
    battery: Option<(u8, bool)>,
    volume: f32,
    art: Option<Arc<Pixmap>>,
    tint: [u8; 3],
    clips: Vec<Clip>,
    clips_rev: u64,
    power: Option<Power>,
    muted: bool,
    audio_out: String,
    brightness: Option<f32>,
    net: Net,
    devices: Vec<Device>,
}

impl Snap {
    fn take(s: &Shared) -> Self {
        Snap {
            title: s.title.clone(),
            artist: s.artist.clone(),
            playing: s.playing,
            pos: s.pos_now(),
            dur: s.dur_ms,
            has_lyrics: !s.lyrics.is_empty(),
            all: s.lyrics.iter().map(|l| l.1.clone()).collect(),
            times: s.lyrics.iter().map(|l| l.0).collect(),
            idx: s.lyric_index().map(|i| i as isize).unwrap_or(-1),
            activity: s
                .activity
                .as_ref()
                .filter(|(_, t)| t.elapsed() < Duration::from_millis(2500))
                .map(|(t, _)| t.clone()),
            shelf: s
                .shelf
                .iter()
                .map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                .collect(),
            battery: s.battery,
            volume: s.volume,
            art: s.art.clone(),
            tint: s.tint,
            // hanya potongan pendek untuk tampilan
            clips: s
                .clips
                .iter()
                .map(|c| match c {
                    Clip::Text(t) => Clip::Text(t.chars().take(160).map(|ch| if ch.is_control() { ' ' } else { ch }).collect()),
                    Clip::Image(i) => Clip::Image(i.clone()),
                })
                .collect(),
            clips_rev: s.clips_rev,
            power: s.power.clone(),
            muted: s.muted,
            audio_out: s.audio_out.clone(),
            brightness: s.brightness,
            net: s.net.clone(),
            devices: s.devices.clone(),
        }
    }
}

// ---------------------------------------------------------------- input mouse
/// Tombol kiri sedang ditekan pada sesuatu yang butuh drag/click terpisah.
enum Press {
    /// geser timeline (rect bar)
    Seek(Rect),
    /// item shelf: index & posisi awal
    Shelf(usize, (f32, f32)),
    /// isi PiP: posisi awal (drag window kalau bergerak, klik = seek lirik)
    Pip((f32, f32)),
    /// geser slider system info (rect track)
    Slider(Slider, Rect),
}

#[derive(Clone, Copy, PartialEq)]
enum Slider {
    Volume,
    Brightness,
}

// ---------------------------------------------------------------- app
struct App {
    shared: State,
    tx: mpsc::Sender<Cmd>,
    sys_tx: mpsc::Sender<SysCmd>,
    bright_tx: mpsc::Sender<f32>,
    fonts: Fonts,
    bold: Fonts,
    notch: Option<Win>,
    pip: Option<Win>,
    page: Page,
    lyrics_inline: bool,
    want_pip: bool,
    hover: bool,
    file_hover: bool,
    left_at: Instant,
    cursor: (f32, f32),
    w: Spring,
    h: Spring,
    last_frame: Instant,
    last_sig: u64,
    t0: Instant,
    inline_scroll: LyricScroll,
    pip_scroll: LyricScroll,
    inline_hits: LyricHits,
    pip_hits: LyricHits,
    pill: PillLyric,
    pip_size: (f32, f32),
    pip_font: f32,
    pip_pos: Option<(i32, i32)>,
    cfg_dirty: Option<Instant>,
    /// debug: NOTCHY_EXPAND=1|lyrics paksa expanded (untuk screenshot)
    force_expand: bool,
    next_frame: Instant,
    /// sedang animasi halus -> redraw tiap vsync
    vsync: bool,
    shape_moving: bool,
    /// debug: NOTCHY_ANIMTEST=<folder> rekam frame animasi buka/tutup
    animtest: Option<(PathBuf, Instant)>,
    anim_n: u32,
    quit: bool,
    press: Option<Press>,
    /// posisi seek sementara (0..1) selama drag timeline
    seek: Option<f32>,
    last_click: Option<(Instant, usize)>,
    scroll: usize,
    todos: Todos,
    editing: Option<String>,
    pomo: Pomo,
    /// debug: NOTCHY_CLIPTEST=1 uji salin item clipboard teratas
    cliptest: bool,
}

impl App {
    fn expanded(&self) -> bool {
        if let Some((_, t0)) = &self.animtest {
            let t = t0.elapsed().as_secs_f32();
            return (0.4..1.6).contains(&t) || (2.6..3.8).contains(&t);
        }
        self.force_expand
            || self.hover
            || self.file_hover
            || self.editing.is_some()
            || self.press.is_some()
            || self.left_at.elapsed() < Duration::from_millis(250)
    }

    fn inline_on(&self) -> bool {
        self.lyrics_inline && self.page == Page::Media
    }

    /// Collapsed + mode lirik aktif: pill lebar berisi baris lirik saat ini.
    fn lyric_pill(&self, snap: &Snap) -> bool {
        !self.expanded() && snap.activity.is_none() && self.lyrics_inline && !snap.title.is_empty()
    }

    fn target(&self, snap: &Snap) -> (f32, f32) {
        if self.expanded() {
            if self.inline_on() {
                LYR
            } else if self.page == Page::Status {
                STA
            } else {
                EXP
            }
        } else if snap.activity.is_some() {
            (280.0, 44.0)
        } else if self.lyric_pill(snap) {
            (380.0, 32.0)
        } else {
            (230.0, 32.0)
        }
    }

    fn x0(&self) -> f32 {
        (WIN_W - self.w.cur) / 2.0
    }

    fn set_page(&mut self, p: Page) {
        if self.page != p {
            self.page = p;
            self.scroll = 0;
            self.end_edit(false);
        }
    }

    fn mark_dirty(&mut self) {
        self.cfg_dirty = Some(Instant::now());
        self.last_sig = 0;
    }

    // ---------------- todo: input teks
    fn begin_edit(&mut self) {
        if self.editing.is_none() {
            self.editing = Some(String::new());
            if let Some(w) = &self.notch {
                gfx::set_activatable(w.hwnd, true);
            }
        }
    }

    fn end_edit(&mut self, commit: bool) {
        if let Some(t) = self.editing.take() {
            if commit {
                self.todos.add(&t);
            }
            if let Some(w) = &self.notch {
                gfx::set_activatable(w.hwnd, false);
            }
            self.last_sig = 0;
        }
    }

    fn seek_to(&mut self, ms: i64) {
        let _ = self.tx.send(Cmd::Seek(ms));
        let mut s = self.shared.lock().unwrap();
        s.pos_ms = ms;
        s.stamp = Some(Instant::now());
    }

    // ---------------- gambar
    fn draw_notch(&mut self, snap: &Snap) {
        let (page, inline, want_pip) = (self.page, self.inline_on(), self.want_pip);
        let pill = self.lyric_pill(snap);
        let t = self.t0.elapsed().as_secs_f32();
        // transisi baris lirik di pill: baris lama naik & memudar, baris baru masuk dari bawah
        if snap.idx != self.pill.idx {
            self.pill.prev = self.pill.cur.clone();
            self.pill.idx = snap.idx;
            self.pill.at = Instant::now();
        }
        self.pill.cur = pill_text(snap);
        let Some(win) = self.notch.as_mut() else { return };
        let (pw, ph) = (win.pm.width(), win.pm.height());
        // layer + mask hanya saat bentuk notch bergerak; saat diam konten sudah muat -> gambar langsung (hemat CPU)
        let clip = self.shape_moving;
        let mut layer = if clip {
            win.layer.take().filter(|l| l.width() == pw && l.height() == ph).unwrap_or_else(|| Pixmap::new(pw, ph).unwrap())
        } else {
            std::mem::replace(&mut win.pm, Pixmap::new(1, 1).unwrap())
        };
        layer.fill(tiny_skia::Color::TRANSPARENT);
        let (w, h) = (self.w.cur.max(40.0), self.h.cur.max(20.0));
        let x0 = (WIN_W - w) / 2.0;
        let mut ctx = Ctx { pm: &mut layer, fonts: &self.fonts, bold: &self.bold, s: win.s };
        // fade konten: expanded muncul setelah notch cukup besar, konten collapsed muncul saat hampir kecil
        let a = smoothstep((h - 78.0) / 56.0);
        let ca = smoothstep((76.0 - h) / 30.0);
        let r = (h * 0.4).clamp(12.0, 30.0);

        // latar: hitam, saat expanded/pill lirik ada gradient warna artwork di bawah
        if snap.art.is_some() && (a > 0.0 || pill) {
            let tc = snap.tint;
            let k = if pill { 0.6 } else { 0.55 * a };
            let bot = [(tc[0] as f32 * k) as u8, (tc[1] as f32 * k) as u8, (tc[2] as f32 * k) as u8, 255];
            ctx.rrect_vgrad(x0, -r, w, h + r, r, &[(0.0, BLACK), (0.45, BLACK), (1.0, bot)]);
        } else {
            ctx.rrect(x0, -r, w, h + r, r, BLACK);
        }

        if a > 0.0 && inline {
            draw_inline(&mut ctx, snap, x0, w, h, a, want_pip, &mut self.inline_scroll, &mut self.inline_hits);
        } else if a > 0.0 {
            let cx = x0 + w / 2.0;
            let mw = w - 68.0;
            for (i, (ax, ay)) in lay::arrows(x0, w).into_iter().enumerate() {
                icon_chevron(&mut ctx, ax, ay, i == 0, a);
            }
            // page dots
            let n = PAGES.len() as f32;
            let dots_w = 14.0 + (n - 1.0) * 4.0 + (n - 1.0) * 5.0;
            let mut dx = cx - dots_w / 2.0;
            for p in PAGES {
                let on = p == page;
                let dw = if on { 14.0 } else { 4.0 };
                ctx.rrect(dx, h - 9.0, dw, 4.0, 2.0, fade(if on { WHITE } else { DIM }, a));
                dx += dw + 5.0;
            }

            match page {
                Page::Media => {
                    let (adx, ay, asz) = lay::ART;
                    match &snap.art {
                        Some(img) => ctx.image(img, x0 + adx, ay, asz, 12.0, a),
                        None => ctx.rrect(x0 + adx, ay, asz, asz, 12.0, fade(CHIP, a)),
                    }
                    let tx = x0 + adx + asz + 14.0;
                    let tw = x0 + w - 34.0 - tx;
                    // baris status: baterai + tanggal + jam
                    let (date, time) = local_time();
                    let mut sx = tx;
                    if let Some((p, chg)) = snap.battery {
                        icon_battery(&mut ctx, sx, 12.0, p, chg, a);
                        sx += 32.0;
                    }
                    sx += ctx.text(&date, sx, 9.0, 12.5, fade(GRAY, a), 160.0) + 8.0;
                    ctx.text_b(&time, sx, 9.0, 12.5, fade(WHITE, a), 60.0);
                    icon_bars(&mut ctx, x0 + w - 52.0, 22.0, 12.0, t, snap.playing, fade(GRAY, a));

                    let title = if snap.title.is_empty() { "Nothing playing" } else { &snap.title };
                    ctx.text_b(title, tx, 36.0, 16.0, fade(WHITE, a), tw);
                    ctx.text(&snap.artist, tx, 58.0, 12.0, fade(GRAY, a), tw);

                    let py = lay::PROG_Y;
                    ctx.text(&mmss(snap.pos), x0 + 34.0, py, 13.0, fade(WHITE, a), 50.0);
                    let rem = format!("-{}", mmss(snap.dur - snap.pos));
                    let rw = text_w(&ctx, &rem, 13.0, false);
                    ctx.text(&rem, x0 + w - 34.0 - rw, py, 13.0, fade(WHITE, a), 60.0);
                    draw_bar(&mut ctx, lay::bar(x0, w), snap, a);

                    let cy = lay::CTRL_Y;
                    let (lx, ly) = lay::lyric_icon(x0);
                    icon_lyrics(&mut ctx, lx, ly, fade(if snap.has_lyrics { WHITE } else { DIM }, a));
                    icon_prev(&mut ctx, cx - 46.0, cy, fade(WHITE, a));
                    icon_play(&mut ctx, cx, cy, snap.playing, fade(WHITE, a));
                    icon_next(&mut ctx, cx + 46.0, cy, fade(WHITE, a));
                }
                Page::Shelf => {
                    ctx.text_b("Shelf", x0 + 34.0, 9.0, 13.0, fade(WHITE, a), 100.0);
                    if snap.shelf.is_empty() {
                        center(&mut ctx, "Drop files here", cx, 56.0, 15.0, fade(GRAY, a), mw, true);
                        center(&mut ctx, "drag out to move/copy · double-click to open · right-click to remove", cx, 82.0, 11.0, fade(DIM, a), mw, false);
                    } else {
                        let hint = "drag out · double-click open · right-click remove";
                        let hw = text_w(&ctx, hint, 10.5, false);
                        ctx.text(hint, x0 + w - 34.0 - hw, 11.0, 10.5, fade(DIM, a), mw);
                    }
                    for (i, name) in snap.shelf.iter().take(8).enumerate() {
                        let (x, y, cw, chh) = lay::shelf(x0, w, i);
                        let hov = inside(self.cursor, (x, y, cw, chh));
                        ctx.rrect(x, y, cw, chh, 8.0, fade(if hov { CHIP_HI } else { CHIP }, a));
                        ctx.text(name, x + 8.0, y + 7.0, 11.0, fade(WHITE, a), cw - 16.0);
                    }
                }
                Page::Clipboard => {
                    ctx.text_b("Clipboard", x0 + 34.0, 9.0, 13.0, fade(WHITE, a), 120.0);
                    ctx.text("click: paste · ctrl+click: copy · right-click: remove", x0 + 108.0, 11.0, 10.5, fade(DIM, a), w - 108.0 - 34.0 - 60.0);
                    if !snap.clips.is_empty() {
                        button(&mut ctx, lay::head_btn(x0, w, 52.0), "Clear", CHIP, GRAY, a);
                    }
                    if snap.clips.is_empty() {
                        center(&mut ctx, "Copy text or a screenshot — it shows up here", cx, 62.0, 13.0, fade(GRAY, a), mw, false);
                    }
                    for (row, (i, c)) in snap.clips.iter().enumerate().skip(self.scroll).take(lay::ROWS).enumerate() {
                        let r = lay::row(x0, w, row);
                        let hov = inside(self.cursor, r);
                        ctx.rrect(r.0, r.1, r.2, r.3, 7.0, fade(if hov { CHIP_HI } else { CHIP }, a));
                        ctx.text(&format!("{}", i + 1), r.0 + 8.0, r.1 + 5.0, 11.0, fade(DIM, a), 20.0);
                        match c {
                            Clip::Text(t) => {
                                ctx.text(t, r.0 + 26.0, r.1 + 5.0, 12.0, fade(WHITE, a), r.2 - 34.0);
                            }
                            Clip::Image(img) => {
                                let th = 20.0;
                                let tw = (th * img.thumb.width() as f32 / img.thumb.height() as f32).clamp(12.0, 44.0);
                                ctx.image_rect(&img.thumb, r.0 + 26.0, r.1 + 2.0, tw, th, 3.0, a);
                                let label = format!("Image · {}×{}", img.w, img.h);
                                ctx.text(&label, r.0 + 34.0 + tw, r.1 + 5.0, 12.0, fade(GRAY, a), r.2 - 44.0 - tw);
                            }
                        }
                    }
                    draw_scrollbar(&mut ctx, x0, w, snap.clips.len(), self.scroll, a);
                }
                Page::Todo => {
                    let open = self.todos.items.iter().filter(|t| !t.0).count();
                    ctx.text_b("Tasks", x0 + 34.0, 9.0, 13.0, fade(WHITE, a), 80.0);
                    ctx.text(&format!("{open} open"), x0 + 80.0, 10.0, 11.5, fade(DIM, a), 80.0);
                    // donut progress: done / total (rata kanan header)
                    let total = self.todos.items.len();
                    if total > 0 {
                        let done = total - open;
                        let (dcx, dcy) = (x0 + w - 34.0 - 8.0, 17.0);
                        let col = if done == total { GREEN } else { ACCENT };
                        ctx.arc(dcx, dcy, 7.0, 2.6, 1.0, fade(TRACK, a));
                        ctx.arc(dcx, dcy, 7.0, 2.6, done as f32 / total as f32, fade(col, a));
                        let label = format!("{done}/{total} done");
                        let lw = text_w(&ctx, &label, 11.5, false);
                        ctx.text(&label, dcx - 13.0 - lw, 10.0, 11.5, fade(if done == total { GREEN } else { GRAY }, a), 90.0);
                    }
                    // baris input
                    let r = lay::row(x0, w, 0);
                    match &self.editing {
                        Some(buf) => {
                            ctx.rrect(r.0, r.1, r.2, r.3, 7.0, fade([40, 52, 66, 255], a));
                            let shown = if buf.is_empty() { "Type a task, Enter to add, Esc to cancel" } else { buf };
                            let col = if buf.is_empty() { DIM } else { WHITE };
                            let tw = ctx.text(shown, r.0 + 26.0, r.1 + 5.0, 12.0, fade(col, a), r.2 - 34.0);
                            if (t * 2.0) as i32 % 2 == 0 {
                                let cx_ = if buf.is_empty() { r.0 + 26.0 } else { r.0 + 26.0 + tw + 1.0 };
                                ctx.rrect(cx_, r.1 + 5.0, 1.5, 14.0, 0.5, fade(WHITE, a));
                            }
                        }
                        None => {
                            ctx.rrect(r.0, r.1, r.2, r.3, 7.0, fade(CHIP, a));
                            ctx.text("+  Add a task…", r.0 + 10.0, r.1 + 5.0, 12.0, fade(GRAY, a), r.2 - 20.0);
                        }
                    }
                    for (row, (_, (done, text))) in self.todos.items.iter().enumerate().skip(self.scroll).take(lay::ROWS - 1).enumerate() {
                        let r = lay::row(x0, w, row + 1);
                        icon_check(&mut ctx, r.0 + 12.0, r.1 + 12.0, *done, a);
                        ctx.text(text, r.0 + 26.0, r.1 + 5.0, 12.0, fade(if *done { DIM } else { WHITE }, a), r.2 - 56.0);
                        if inside(self.cursor, r) {
                            ctx.text("×", r.0 + r.2 - 16.0, r.1 + 2.0, 15.0, fade(GRAY, a), 14.0);
                        }
                    }
                    draw_scrollbar(&mut ctx, x0, w, self.todos.items.len() + 1, self.scroll, a);
                }
                Page::Timer => {
                    let p = &self.pomo;
                    let (rdx, ry, rr) = lay::RING;
                    let col = phase_color(p.phase);
                    ctx.arc(x0 + rdx, ry, rr, 5.0, 1.0, fade(TRACK, a));
                    ctx.arc(x0 + rdx, ry, rr, 5.0, p.progress().max(0.001), fade(col, a));
                    let left = p.left().as_millis() as i64;
                    center(&mut ctx, &mmss(left + 999), x0 + rdx, ry - 14.0, 20.0, fade(WHITE, a), 80.0, true);
                    center(&mut ctx, p.label(), x0 + rdx, ry + 10.0, 11.0, fade(col, a), 80.0, false);

                    let lx = x0 + 176.0;
                    ctx.text_b(&format!("Session {}", p.done + 1), lx, 9.0, 13.0, fade(WHITE, a), 140.0);
                    let rows = [
                        ("Focus", format!("{} min", p.work)),
                        ("Break", format!("{} min", p.brk)),
                        ("Long break", format!("{} min", p.long)),
                        ("Long every", format!("{} sessions", p.every)),
                    ];
                    for (i, (k, v)) in rows.iter().enumerate() {
                        let y = 34.0 + i as f32 * 20.0;
                        ctx.text(k, lx, y + 1.0, 12.0, fade(GRAY, a), 80.0);
                        ctx.text_b(v, lx + 80.0, y + 1.0, 12.0, fade(WHITE, a), 90.0);
                        button(&mut ctx, lay::setting_minus(x0, w, i), "−", CHIP, WHITE, a);
                        button(&mut ctx, lay::setting_plus(x0, w, i), "+", CHIP, WHITE, a);
                    }
                    let start = if p.running() { "Pause" } else if p.active() { "Resume" } else { "Start" };
                    button(&mut ctx, lay::timer_btn(x0, 0), start, if p.running() { CHIP } else { col }, if p.running() { WHITE } else { BLACK }, a);
                    button(&mut ctx, lay::timer_btn(x0, 1), "Reset", CHIP, WHITE, a);
                    button(&mut ctx, lay::timer_btn(x0, 2), "Skip", CHIP, WHITE, a);
                }
                Page::Status => draw_status(&mut ctx, snap, x0, w, a, self.cursor),
            }
        } else if let Some(act) = &snap.activity {
            let tw = text_w(&ctx, act, 14.0, true);
            ctx.text_b(act, x0 + (w - tw.min(w - 24.0)) / 2.0, 12.0, 14.0, fade(WHITE, ca), w - 24.0);
        } else if pill && w > 240.0 {
            // collapsed + lirik: ikon not di kiri, baris lirik aktif di tengah
            icon_note(&mut ctx, x0 + 26.0, 16.0, fade(GRAY, ca));
            let (cx, mw) = (x0 + w / 2.0 + 8.0, w - 90.0);
            let p = (self.pill.at.elapsed().as_secs_f32() / 0.28).min(1.0);
            let e = 1.0 - (1.0 - p) * (1.0 - p); // ease-out
            let avail = snap.has_lyrics;
            let col = if avail { WHITE } else { GRAY };
            if e < 1.0 && !self.pill.prev.is_empty() {
                center(&mut ctx, &self.pill.prev, cx, 8.0 - 8.0 * e, 13.0, fade(WHITE, (1.0 - e) * ca), mw, true);
            }
            center(&mut ctx, &self.pill.cur, cx, 8.0 + 8.0 * (1.0 - e), 13.0, fade(col, e * ca), mw, avail);
        } else if w > 180.0 {
            // collapsed idle: cincin baterai / timer kiri, jam / hitung mundur tengah, visualizer kanan
            let p = &self.pomo;
            if p.active() {
                let col = phase_color(p.phase);
                let col = fade(col, if p.running() { ca } else { 0.5 * ca });
                ctx.arc(x0 + 28.0, 16.0, 8.0, 2.2, 1.0, fade([60, 60, 66, 255], ca));
                ctx.arc(x0 + 28.0, 16.0, 8.0, 2.2, p.progress().max(0.001), col);
                center(&mut ctx, &mmss(p.left().as_millis() as i64 + 999), x0 + w / 2.0, 7.0, 14.0, col, 80.0, true);
            } else {
                if let Some((p, chg)) = snap.battery {
                    icon_battery_ring(&mut ctx, x0 + 28.0, 16.0, p, chg, ca);
                }
                center(&mut ctx, &local_time().1, x0 + w / 2.0, 7.0, 14.0, fade(WHITE, ca), 80.0, true);
            }
            if !snap.title.is_empty() {
                icon_bars(&mut ctx, x0 + w - 42.0, 22.0, 12.0, t, snap.playing, fade(if snap.playing { WHITE } else { GRAY }, ca));
            }
        }
        // potong semua konten ke bentuk notch saat ini (tidak ada teks "bocor" saat animasi)
        if clip {
            win.pm.fill(tiny_skia::Color::TRANSPARENT);
            gfx::fill_rrect_from(&mut win.pm, &layer, win.s, x0, -r, w, h + r, r);
            win.layer = Some(layer);
        } else {
            win.pm = layer;
        }
        if let Some(path) = std::env::var_os("NOTCHY_DUMP") {
            let _ = win.pm.save_png(path);
        }
        if let Some((dir, t0)) = &self.animtest {
            let ms = t0.elapsed().as_millis();
            if ms < 4800 {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("frames.txt")) {
                    let _ = writeln!(f, "{} {:.2} {:.2}", t0.elapsed().as_micros(), w, h);
                }
                // simpan sebagian frame saja (PNG lambat, jangan ganggu pacing)
                if std::env::var_os("NOTCHY_ANIMPNG").is_some() && self.anim_n % 6 == 0 {
                    let _ = win.pm.save_png(dir.join(format!("f{ms:05}.png")));
                }
                self.anim_n += 1;
            }
        }
        gfx::present(win.hwnd, &win.pm);
    }

    fn draw_pip(&mut self, snap: &Snap) {
        let fs = self.pip_font;
        let Some(win) = self.pip.as_mut() else { return };
        win.pm.fill(tiny_skia::Color::TRANSPARENT);
        let sc = win.s;
        let (w, h) = (win.pm.width() as f32 / sc, win.pm.height() as f32 / sc);
        let mw = w - 32.0;
        let mut ctx = Ctx { pm: &mut win.pm, fonts: &self.fonts, bold: &self.bold, s: sc };
        if snap.art.is_some() {
            let tc = snap.tint;
            let bot = [(tc[0] as f32 * 0.5) as u8, (tc[1] as f32 * 0.5) as u8, (tc[2] as f32 * 0.5) as u8, 240];
            ctx.rrect_vgrad(0.0, 0.0, w, h, 18.0, &[(0.0, [12, 12, 16, 240]), (1.0, bot)]);
        } else {
            ctx.rrect(0.0, 0.0, w, h, 18.0, [16, 16, 22, 236]);
        }
        ctx.text("×", w - 22.0, 4.0, 16.0, GRAY, 16.0);
        center(&mut ctx, &format!("{} — {}", snap.title, snap.artist), w / 2.0, h - 20.0, 10.0, DIM, mw, false);

        if !snap.has_lyrics {
            self.pip_hits.clear();
            let msg = if snap.title.is_empty() { "Nothing playing" } else { "No synced lyrics found" };
            center(&mut ctx, msg, w / 2.0, h / 2.0 - 14.0, 14.0, GRAY, mw, false);
        } else {
            let st = ListStyle {
                x: 16.0,
                w: mw,
                top: 14.0,
                bottom: h - 28.0,
                cpx: 20.0 * fs,
                opx: 15.0 * fs,
                gap: 8.0 * fs,
                anchor: 0.42,
                centered: true,
            };
            draw_lyric_list(&mut ctx, snap, &st, &mut self.pip_scroll, 1.0, &mut self.pip_hits);
        }
        gfx::present(win.hwnd, &win.pm);
    }

    fn sync_pip(&mut self, el: &ActiveEventLoop) {
        if self.want_pip && self.pip.is_none() {
            let (pw, ph) = self.pip_size;
            let saved = self.pip_pos.filter(|&(x, y)| {
                el.available_monitors().any(|m| {
                    let (p, z) = (m.position(), m.size());
                    x >= p.x && y >= p.y && x < p.x + z.width as i32 - 40 && y < p.y + z.height as i32 - 40
                })
            });
            self.pip = Some(Win::new(el, pw, ph, |s, el| {
                if let Some((x, y)) = saved {
                    return PhysicalPosition::new(x, y);
                }
                let (mx, my, mw) = el
                    .primary_monitor()
                    .map(|m| (m.position().x, m.position().y, m.size().width as i32))
                    .unwrap_or((0, 0, 1920));
                PhysicalPosition::new(mx + (mw - (pw * s) as i32) / 2, my + (260.0 * s) as i32)
            }));
            if let Some(p) = &self.pip {
                p.window.set_resizable(true);
                p.window.set_min_inner_size(Some(LogicalSize::new(220.0, 80.0)));
                // winit menimpa ex-style saat set_resizable -> pasang ulang LAYERED/TOPMOST/dll
                gfx::make_overlay(p.hwnd);
            }
            self.pip_scroll = LyricScroll::default();
            self.last_sig = 0;
        } else if !self.want_pip && self.pip.is_some() {
            if let Some(p) = &self.pip {
                gfx::forget(p.hwnd);
            }
            self.pip = None;
        }
    }

    // ---------------- klik
    /// Tombol kiri ditekan di notch.
    fn press_notch(&mut self) {
        let (w, h) = (self.w.cur, self.h.cur);
        if h < 100.0 {
            return;
        }
        let x0 = self.x0();
        let p = self.cursor;
        self.last_sig = 0;

        // timeline: mulai drag seek
        let bar = if self.inline_on() {
            Some(lay::l_bar(x0))
        } else if self.page == Page::Media {
            Some(lay::bar(x0, w))
        } else {
            None
        };
        if let Some(b) = bar {
            if inside(p, grow(b, 4.0, 9.0)) {
                self.seek = Some(((p.0 - b.0) / b.2).clamp(0.0, 1.0));
                self.press = Some(Press::Seek(b));
                return;
            }
        }

        if self.inline_on() {
            let c = lay::l_ctrls(x0);
            if near(p, c[0], 14.0) {
                self.lyrics_inline = false;
                self.mark_dirty();
            } else if near(p, c[1], 14.0) {
                let _ = self.tx.send(Cmd::Prev);
            } else if near(p, c[2], 16.0) {
                let _ = self.tx.send(Cmd::PlayPause);
            } else if near(p, c[3], 14.0) {
                let _ = self.tx.send(Cmd::Next);
            } else if inside(p, lay::l_popout(x0, w)) {
                self.want_pip = !self.want_pip;
            } else if let Some(&(_, ms)) = self.inline_hits.iter().find(|(r, _)| inside(p, *r)) {
                self.seek_to(ms);
            }
            return;
        }

        let arrows = lay::arrows(x0, w);
        let n = PAGES.len();
        let cur = PAGES.iter().position(|&x| x == self.page).unwrap_or(0);
        if near(p, arrows[0], 14.0) {
            self.set_page(PAGES[(cur + n - 1) % n]);
            return;
        }
        if near(p, arrows[1], 14.0) {
            self.set_page(PAGES[(cur + 1) % n]);
            return;
        }
        match self.page {
            Page::Media => {
                let (cx, cy) = (x0 + w / 2.0, lay::CTRL_Y);
                if near(p, lay::lyric_icon(x0), 14.0) {
                    self.lyrics_inline = true;
                    self.inline_scroll = LyricScroll::default();
                    self.mark_dirty();
                } else if near(p, (cx - 46.0, cy), 15.0) {
                    let _ = self.tx.send(Cmd::Prev);
                } else if near(p, (cx, cy), 16.0) {
                    let _ = self.tx.send(Cmd::PlayPause);
                } else if near(p, (cx + 46.0, cy), 15.0) {
                    let _ = self.tx.send(Cmd::Next);
                }
            }
            Page::Shelf => {
                let n = self.shared.lock().unwrap().shelf.len().min(8);
                if let Some(i) = (0..n).find(|&i| inside(p, lay::shelf(x0, w, i))) {
                    self.press = Some(Press::Shelf(i, p));
                }
            }
            Page::Clipboard => {
                if inside(p, lay::head_btn(x0, w, 52.0)) {
                    let mut s = self.shared.lock().unwrap();
                    s.clips.clear();
                    s.clips_rev += 1;
                    return;
                }
                if let Some(row) = (0..lay::ROWS).find(|&r| inside(p, lay::row(x0, w, r))) {
                    let item = self.shared.lock().unwrap().clips.get(row + self.scroll).cloned();
                    if let (Some(item), Some(win)) = (item, self.notch.as_ref()) {
                        // klik = langsung paste ke field yang aktif (seperti Win+V); Ctrl+klik = salin saja
                        let copy_only = clip::ctrl_down();
                        let ok = clip::set(win.hwnd, &item);
                        if ok && !copy_only {
                            clip::paste();
                        }
                        let msg = match (ok, copy_only) {
                            (false, _) => "Clipboard busy",
                            (true, true) => "Copied",
                            (true, false) => "Pasted",
                        };
                        self.shared.lock().unwrap().flash(msg.into());
                    }
                }
            }
            Page::Todo => {
                if inside(p, lay::row(x0, w, 0)) {
                    self.begin_edit();
                    return;
                }
                self.end_edit(true);
                if let Some(row) = (1..lay::ROWS).find(|&r| inside(p, lay::row(x0, w, r))) {
                    let i = row - 1 + self.scroll;
                    let r = lay::row(x0, w, row);
                    if p.0 > r.0 + r.2 - 24.0 {
                        self.todos.remove(i);
                    } else {
                        self.todos.toggle(i);
                    }
                }
            }
            Page::Timer => {
                for i in 0..4 {
                    if inside(p, lay::setting_minus(x0, w, i)) {
                        self.pomo.adjust(i, -1);
                        self.mark_dirty();
                    } else if inside(p, lay::setting_plus(x0, w, i)) {
                        self.pomo.adjust(i, 1);
                        self.mark_dirty();
                    }
                }
                if inside(p, lay::timer_btn(x0, 0)) {
                    self.pomo.toggle();
                } else if inside(p, lay::timer_btn(x0, 1)) {
                    self.pomo.reset();
                } else if inside(p, lay::timer_btn(x0, 2)) {
                    self.pomo.skip();
                }
            }
            Page::Status => {
                let (vr, br) = (lay::st_volume(x0, w), lay::st_bright(x0, w));
                let settings = |uri: &str| {
                    let _ = std::process::Command::new("explorer.exe").arg(uri).spawn();
                };
                if inside(p, lay::quit(x0, w)) {
                    self.quit = true;
                } else if inside(p, vr) {
                    if near(p, lay::slider_icon(vr), 15.0) {
                        let _ = self.sys_tx.send(SysCmd::ToggleMute);
                        let mut s = self.shared.lock().unwrap();
                        s.muted = !s.muted;
                    } else if inside(p, grow(lay::slider(vr), 8.0, 10.0)) {
                        self.press = Some(Press::Slider(Slider::Volume, lay::slider(vr)));
                        self.set_slider(Slider::Volume, lay::slider(vr));
                    }
                } else if inside(p, br) {
                    if inside(p, grow(lay::slider(br), 8.0, 10.0)) && self.shared.lock().unwrap().brightness.is_some() {
                        self.press = Some(Press::Slider(Slider::Brightness, lay::slider(br)));
                        self.set_slider(Slider::Brightness, lay::slider(br));
                    }
                } else if inside(p, lay::st_battery(x0, w)) {
                    settings("ms-settings:batterysaver");
                } else if inside(p, lay::st_net(x0, w)) {
                    settings("ms-settings:network-status");
                } else if inside(p, lay::st_devices(x0, w)) {
                    settings("ms-settings:bluetooth");
                }
            }
        }
    }

    /// Set nilai slider dari posisi kursor (0..1), langsung tampil di UI lalu dikirim ke thread sistem.
    fn set_slider(&mut self, k: Slider, r: Rect) {
        let f = (((self.cursor.0 - r.0) / r.2).clamp(0.0, 1.0) * 100.0).round() / 100.0;
        self.apply_slider(k, f);
    }

    fn apply_slider(&mut self, k: Slider, f: f32) {
        let f = f.clamp(0.0, 1.0);
        let mut s = self.shared.lock().unwrap();
        match k {
            Slider::Volume => {
                s.volume = f;
                s.muted = false;
                let _ = self.sys_tx.send(SysCmd::Volume(f));
            }
            Slider::Brightness => {
                s.brightness = Some(f);
                s.brightness_hold = Some(Instant::now() + Duration::from_millis(2500));
                let _ = self.bright_tx.send(f);
            }
        }
        self.last_sig = 0;
    }

    /// Tombol kiri dilepas (di window manapun).
    fn release(&mut self) {
        match self.press.take() {
            Some(Press::Seek(_)) => {
                if let Some(f) = self.seek.take() {
                    let dur = self.shared.lock().unwrap().dur_ms;
                    if dur > 0 {
                        self.seek_to((f as f64 * dur as f64) as i64);
                    }
                }
            }
            Some(Press::Shelf(i, _)) => {
                // klik biasa: dobel-klik untuk membuka
                let dbl = self.last_click.is_some_and(|(t, j)| j == i && t.elapsed() < Duration::from_millis(450));
                if dbl {
                    self.last_click = None;
                    let path = self.shared.lock().unwrap().shelf.get(i).cloned();
                    if let Some(path) = path {
                        let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
                    }
                } else {
                    self.last_click = Some((Instant::now(), i));
                }
            }
            Some(Press::Slider(..)) => {}
            Some(Press::Pip(_)) => {
                let p = self.cursor;
                if let Some(&(_, ms)) = self.pip_hits.iter().find(|(r, _)| inside(p, *r)) {
                    self.seek_to(ms);
                }
            }
            None => {}
        }
        self.last_sig = 0;
    }

    /// Kursor bergerak sambil menekan tombol kiri.
    fn drag_move(&mut self, is_pip: bool) {
        let p = self.cursor;
        match &self.press {
            Some(Press::Seek(b)) if !is_pip => {
                self.seek = Some(((p.0 - b.0) / b.2).clamp(0.0, 1.0));
            }
            Some(Press::Slider(k, r)) if !is_pip => {
                let (k, r) = (*k, *r);
                self.set_slider(k, r);
            }
            Some(Press::Shelf(i, at)) if !is_pip && (p.0 - at.0).hypot(p.1 - at.1) > 5.0 => {
                let i = *i;
                self.press = None;
                let path = self.shared.lock().unwrap().shelf.get(i).cloned();
                if let Some(path) = path {
                    // modal sampai file dilepas di Explorer/app lain
                    let _ = dragout::drag_files(&[path]);
                    // file yang dipindah (move) tidak ada lagi -> keluarkan dari shelf
                    self.shared.lock().unwrap().shelf.retain(|f: &PathBuf| f.exists());
                }
                self.left_at = Instant::now();
                self.last_sig = 0;
            }
            Some(Press::Pip(at)) if is_pip && (p.0 - at.0).hypot(p.1 - at.1) > 4.0 => {
                self.press = None;
                if let Some(w) = &self.pip {
                    let _ = w.window.drag_window();
                }
            }
            _ => {}
        }
    }

    fn right_click_notch(&mut self) {
        let (w, h) = (self.w.cur, self.h.cur);
        if h < 100.0 || self.inline_on() {
            return;
        }
        let x0 = self.x0();
        match self.page {
            Page::Shelf => {
                let mut s = self.shared.lock().unwrap();
                let n = s.shelf.len().min(8);
                if let Some(i) = (0..n).find(|&i| inside(self.cursor, lay::shelf(x0, w, i))) {
                    s.shelf.remove(i);
                }
            }
            Page::Clipboard => {
                if let Some(row) = (0..lay::ROWS).find(|&r| inside(self.cursor, lay::row(x0, w, r))) {
                    let mut s = self.shared.lock().unwrap();
                    if row + self.scroll < s.clips.len() {
                        s.clips.remove(row + self.scroll);
                        s.clips_rev += 1;
                    }
                }
            }
            Page::Todo => {
                if let Some(row) = (1..lay::ROWS).find(|&r| inside(self.cursor, lay::row(x0, w, r))) {
                    self.todos.remove(row - 1 + self.scroll);
                }
            }
            _ => {}
        }
        self.last_sig = 0;
    }

    fn wheel_notch(&mut self, dy: f32) {
        if self.page == Page::Status && !self.inline_on() {
            let (x0, w, p) = (self.x0(), self.w.cur, self.cursor);
            let step = dy.signum() * 0.02;
            let (vol, br) = {
                let s = self.shared.lock().unwrap();
                (s.volume, s.brightness)
            };
            if inside(p, lay::st_volume(x0, w)) {
                self.apply_slider(Slider::Volume, ((vol + step) * 50.0).round() / 50.0);
            } else if let (true, Some(b)) = (inside(p, lay::st_bright(x0, w)), br) {
                self.apply_slider(Slider::Brightness, ((b + step * 2.5) * 20.0).round() / 20.0);
            }
            return;
        }
        let len = match self.page {
            Page::Clipboard => self.shared.lock().unwrap().clips.len(),
            Page::Todo => self.todos.items.len() + 1,
            _ => return,
        };
        let max = len.saturating_sub(lay::ROWS);
        self.scroll = if dy > 0.0 { self.scroll.saturating_sub(1) } else { (self.scroll + 1).min(max) };
        self.last_sig = 0;
    }

    fn key(&mut self, ev: &winit::event::KeyEvent) {
        if ev.state != ElementState::Pressed || self.editing.is_none() {
            return;
        }
        match &ev.logical_key {
            Key::Named(NamedKey::Enter) => self.end_edit(true),
            Key::Named(NamedKey::Escape) => self.end_edit(false),
            Key::Named(NamedKey::Backspace) => {
                if let Some(b) = self.editing.as_mut() {
                    b.pop();
                }
            }
            _ => {
                if let (Some(t), Some(b)) = (&ev.text, self.editing.as_mut()) {
                    b.extend(t.chars().filter(|c| !c.is_control()));
                    b.truncate(300);
                }
            }
        }
        self.last_sig = 0;
    }

    fn save_cfg(&self) {
        let Some(p) = cfg_path() else { return };
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let (w, h) = self.pip_size;
        let mut out = format!("pip_w={w:.0}\npip_h={h:.0}\npip_font={:.1}\n", self.pip_font);
        if let Some((x, y)) = self.pip_pos {
            out += &format!("pip_x={x}\npip_y={y}\n");
        }
        out += &format!("lyrics={}\n", self.lyrics_inline as u8);
        let p_ = &self.pomo;
        out += &format!("pomo_work={}\npomo_break={}\npomo_long={}\npomo_every={}\n", p_.work, p_.brk, p_.long, p_.every);
        let _ = std::fs::write(p, out);
    }
}

// ---------------------------------------------------------------- system info
const TILE: [u8; 4] = [255, 255, 255, 15];
const TILE_HI: [u8; 4] = [255, 255, 255, 26];
const TILE_EDGE: [u8; 4] = [255, 255, 255, 18];

fn tile(ctx: &mut Ctx, r: Rect, hov: bool, a: f32) {
    ctx.rrect(r.0, r.1, r.2, r.3, 13.0, fade(if hov { TILE_HI } else { TILE }, a));
    ctx.rrect_stroke(r.0, r.1, r.2, r.3, 13.0, 1.0, fade(TILE_EDGE, a));
}

fn fmt_speed(b: f32) -> String {
    let k = b / 1024.0;
    if k < 1000.0 {
        format!("{:.0} KB/s", k)
    } else if k < 10.0 * 1024.0 {
        format!("{:.1} MB/s", k / 1024.0)
    } else {
        format!("{:.0} MB/s", k / 1024.0)
    }
}

fn fmt_hm(secs: u32) -> String {
    let m = (secs + 30) / 60;
    if m >= 60 { format!("{}h {:02}m", m / 60, m % 60) } else { format!("{m}m") }
}

fn icon_speaker(ctx: &mut Ctx, x: f32, y: f32, level: f32, muted: bool, c: [u8; 4]) {
    ctx.rrect(x - 9.0, y - 3.5, 4.5, 7.0, 1.2, c);
    ctx.poly(&[(x - 5.5, y - 3.5), (x - 0.5, y - 7.5), (x - 0.5, y + 7.5), (x - 5.5, y + 3.5)], c);
    if muted {
        ctx.line(x + 3.0, y - 3.5, x + 9.0, y + 3.5, 1.7, c);
        ctx.line(x + 3.0, y + 3.5, x + 9.0, y - 3.5, 1.7, c);
        return;
    }
    if level > 0.0 {
        ctx.arc_span(x - 0.5, y, 4.5, 1.7, -50.0, 100.0, c);
    }
    if level >= 0.5 {
        ctx.arc_span(x - 0.5, y, 8.5, 1.7, -50.0, 100.0, c);
    }
}

fn icon_sun(ctx: &mut Ctx, x: f32, y: f32, c: [u8; 4]) {
    ctx.circle(x, y, 3.8, c);
    for i in 0..8 {
        let t = (i as f32 * 45.0).to_radians();
        let (dx, dy) = (t.cos(), t.sin());
        ctx.line(x + dx * 6.6, y + dy * 6.6, x + dx * 8.8, y + dy * 8.8, 1.7, c);
    }
}

fn icon_net(ctx: &mut Ctx, x: f32, y: f32, k: NetKind, c: [u8; 4]) {
    match k {
        NetKind::Wifi | NetKind::Offline => {
            let c = if k == NetKind::Offline { fade(c, 0.4) } else { c };
            ctx.circle(x, y + 5.0, 1.8, c);
            for r in [5.0, 8.8, 12.6] {
                ctx.arc_span(x, y + 5.0, r, 1.9, -135.0, 90.0, c);
            }
            if k == NetKind::Offline {
                ctx.line(x - 7.0, y - 7.0, x + 7.0, y + 7.0, 1.8, RED);
            }
        }
        NetKind::Ethernet => {
            ctx.rrect_stroke(x - 7.0, y - 6.5, 14.0, 11.0, 2.5, 1.6, c);
            ctx.rrect(x - 3.0, y + 4.0, 6.0, 3.5, 1.0, c);
            for i in 0..3 {
                ctx.line(x - 3.0 + i as f32 * 3.0, y - 3.0, x - 3.0 + i as f32 * 3.0, y - 0.5, 1.3, c);
            }
        }
        NetKind::Cellular => {
            for i in 0..4 {
                let h = 4.0 + i as f32 * 3.0;
                ctx.rrect(x - 7.0 + i as f32 * 4.0, y + 6.0 - h, 2.8, h, 1.0, c);
            }
        }
    }
}

fn icon_arrow(ctx: &mut Ctx, x: f32, y: f32, down: bool, c: [u8; 4]) {
    let d = if down { 1.0 } else { -1.0 };
    ctx.line(x, y - 4.5 * d, x, y + 4.0 * d, 1.7, c);
    ctx.polyline(&[(x - 3.5, y + 0.8 * d), (x, y + 4.3 * d), (x + 3.5, y + 0.8 * d)], 1.7, c);
}

fn icon_bt(ctx: &mut Ctx, x: f32, y: f32, c: [u8; 4]) {
    ctx.polyline(&[(x - 4.0, y - 4.0), (x + 4.0, y + 4.0), (x, y + 8.0), (x, y - 8.0), (x + 4.0, y - 4.0), (x - 4.0, y + 4.0)], 1.7, c);
}

fn icon_device(ctx: &mut Ctx, x: f32, y: f32, k: DevKind, c: [u8; 4]) {
    match k {
        DevKind::Audio => {
            ctx.arc_span(x, y + 1.0, 6.0, 1.7, 180.0, 180.0, c);
            ctx.rrect(x - 7.5, y, 3.6, 6.5, 1.5, c);
            ctx.rrect(x + 3.9, y, 3.6, 6.5, 1.5, c);
        }
        DevKind::Input => {
            ctx.rrect_stroke(x - 4.5, y - 7.0, 9.0, 14.0, 4.5, 1.6, c);
            ctx.line(x, y - 4.8, x, y - 2.2, 1.6, c);
        }
        DevKind::Phone => {
            ctx.rrect_stroke(x - 4.5, y - 7.5, 9.0, 15.0, 2.5, 1.6, c);
            ctx.line(x - 1.5, y + 4.8, x + 1.5, y + 4.8, 1.4, c);
        }
        DevKind::Other => icon_bt(ctx, x, y, c),
    }
}

fn icon_power(ctx: &mut Ctx, x: f32, y: f32, c: [u8; 4]) {
    ctx.arc_span(x, y + 1.0, 7.5, 2.0, -55.0, 290.0, c);
    ctx.line(x, y - 8.0, x, y - 1.0, 2.0, c);
}

fn icon_bolt(ctx: &mut Ctx, x: f32, y: f32, k: f32, c: [u8; 4]) {
    let p = |dx: f32, dy: f32| (x + dx * k, y + dy * k);
    ctx.poly(&[p(1.2, -5.5), p(-3.2, 0.8), p(-0.2, 0.8), p(-1.2, 5.5), p(3.2, -0.8), p(0.2, -0.8)], c);
}

/// Tile slider (volume / brightness): ikon kiri, label + nilai di atas, track bisa digeser.
#[allow(clippy::too_many_arguments)]
fn slider_tile(ctx: &mut Ctx, r: Rect, label: &str, value: &str, f: Option<f32>, dim: bool, hov: bool, a: f32) {
    tile(ctx, r, hov, a);
    let lw = r.2 - 42.0 - 14.0;
    let vw = text_w(ctx, value, 11.5, true);
    ctx.text(label, r.0 + 42.0, r.1 + 7.0, 11.5, fade(GRAY, a), lw - vw - 8.0);
    ctx.text_b(value, r.0 + r.2 - 14.0 - vw, r.1 + 7.0, 11.5, fade(if dim { DIM } else { WHITE }, a), vw + 2.0);
    let t = lay::slider(r);
    ctx.rrect(t.0, t.1, t.2, t.3, t.3 / 2.0, fade([255, 255, 255, 34], a));
    if let Some(f) = f {
        let fw = (t.2 * f.clamp(0.0, 1.0)).max(t.3);
        let (c0, c1) = if dim { ([110, 110, 118, 255], [140, 140, 148, 255]) } else { ([200, 200, 210, 255], WHITE) };
        ctx.rrect_hgrad(t.0, t.1, fw, t.3, t.3 / 2.0, fade(c0, a), fade(c1, a));
        let kx = t.0 + fw - t.3 / 2.0;
        ctx.circle(kx, t.1 + t.3 / 2.0 + 0.6, 7.0, fade([0, 0, 0, 90], a));
        ctx.circle(kx, t.1 + t.3 / 2.0, 6.2, fade(if dim { [170, 170, 178, 255] } else { WHITE }, a));
    }
}

fn draw_status(ctx: &mut Ctx, snap: &Snap, x0: f32, w: f32, a: f32, cur: (f32, f32)) {
    // ---- baterai
    let r = lay::st_battery(x0, w);
    tile(ctx, r, inside(cur, r), a);
    let (cx, cy) = (r.0 + 34.0, r.1 + r.3 / 2.0);
    match (snap.battery, &snap.power) {
        (Some((pct, ac)), pw) => {
            let pw = pw.clone().unwrap_or_default();
            let charging = ac && pw.charging;
            let col = if charging { GREEN } else if pct <= 20 { RED } else if pw.saver { ORANGE } else { WHITE };
            ctx.arc(cx, cy, 21.0, 4.5, 1.0, fade([255, 255, 255, 30], a));
            ctx.arc(cx, cy, 21.0, 4.5, (pct as f32 / 100.0).max(0.01), fade(col, a));
            let px = if pct >= 100 { 11.5 } else { 13.0 };
            center(ctx, &format!("{pct}%"), cx, cy - px * 0.66, px, fade(WHITE, a), 36.0, true);
            let tx = r.0 + 66.0;
            let tw = r.0 + r.2 - 10.0 - tx;
            let mut x = tx;
            if charging {
                icon_bolt(ctx, x + 4.0, r.1 + 18.5, 1.25, fade(GREEN, a));
                x += 12.0;
            }
            let state = if charging {
                "Charging"
            } else if ac {
                if pct >= 99 { "Fully charged" } else { "Plugged in" }
            } else {
                "On battery"
            };
            ctx.text_b(state, x, r.1 + 10.0, 13.0, fade(WHITE, a), tw - (x - tx));
            let eta = match (pw.secs, charging, ac) {
                (Some(s), true, _) => format!("{} until full", fmt_hm(s)),
                (Some(s), false, false) => format!("{} remaining", fmt_hm(s)),
                (None, false, true) => "Connected to power".into(),
                _ => "Estimating…".into(),
            };
            ctx.text(&eta, tx, r.1 + 29.0, 11.5, fade(GRAY, a), tw);
            let mut info = Vec::new();
            if pw.rate_mw != 0 {
                info.push(format!("{:.1} W", pw.rate_mw.unsigned_abs() as f32 / 1000.0));
            }
            if pw.full_mwh > 0 {
                info.push(format!("{:.0}/{:.0} Wh", pw.remain_mwh as f32 / 1000.0, pw.full_mwh as f32 / 1000.0));
            }
            let iw = ctx.text(&info.join(" · "), tx, r.1 + 46.0, 11.0, fade(DIM, a), tw);
            if pw.saver {
                let sx = tx + iw + if iw > 0.0 { 6.0 } else { 0.0 };
                ctx.rrect(sx, r.1 + 46.0, 40.0, 15.0, 7.5, fade([255, 140, 80, 40], a));
                center(ctx, "Saver", sx + 20.0, r.1 + 47.0, 10.0, fade(ORANGE, a), 38.0, true);
            }
        }
        (None, _) => {
            ctx.arc(cx, cy, 21.0, 4.5, 1.0, fade([255, 255, 255, 30], a));
            icon_bolt(ctx, cx, cy, 2.0, fade(WHITE, a));
            ctx.text_b("AC power", r.0 + 66.0, r.1 + 17.0, 13.0, fade(WHITE, a), r.2 - 76.0);
            ctx.text("No battery", r.0 + 66.0, r.1 + 36.0, 11.5, fade(GRAY, a), r.2 - 76.0);
        }
    }

    // ---- jaringan
    let r = lay::st_net(x0, w);
    tile(ctx, r, inside(cur, r), a);
    let n = &snap.net;
    icon_net(ctx, r.0 + 20.0, r.1 + 17.0, n.kind, fade(WHITE, a));
    let name = match (n.kind, n.name.is_empty()) {
        (NetKind::Offline, _) => "Offline",
        (_, false) => n.name.as_str(),
        (NetKind::Wifi, true) => "Wi-Fi",
        (NetKind::Cellular, true) => "Cellular",
        (NetKind::Ethernet, true) => "Ethernet",
    };
    ctx.text_b(name, r.0 + 38.0, r.1 + 9.0, 13.0, fade(WHITE, a), r.2 - 48.0);
    let sy = r.1 + 28.0;
    icon_arrow(ctx, r.0 + 16.0, sy + 7.0, true, fade(ACCENT, a));
    let dw = ctx.text_b(&fmt_speed(n.down), r.0 + 24.0, sy, 12.0, fade(WHITE, a), 80.0);
    let ux = (r.0 + 24.0 + dw + 14.0).max(r.0 + r.2 / 2.0);
    icon_arrow(ctx, ux, sy + 7.0, false, fade(ORANGE, a));
    ctx.text(&fmt_speed(n.up), ux + 8.0, sy, 12.0, fade(GRAY, a), r.0 + r.2 - 10.0 - ux - 8.0);
    // sparkline riwayat (tinggi pakai akar supaya lalu lintas kecil tetap kelihatan)
    let (gx, gw, gb, gh) = (r.0 + 12.0, r.2 - 24.0, r.1 + r.3 - 7.0, 13.0);
    let cnt = sys::NET_HIST;
    let bw = (gw - (cnt as f32 - 1.0) * 1.6) / cnt as f32;
    let mx = n.hist.iter().map(|(d, u)| d.max(*u)).fold(48.0 * 1024.0, f32::max);
    let off = cnt - n.hist.len().min(cnt);
    for i in 0..cnt {
        let bx = gx + i as f32 * (bw + 1.6);
        match i.checked_sub(off).and_then(|j| n.hist.get(j)) {
            Some(&(d, u)) => {
                let hd = (gh * (d / mx).sqrt()).max(1.5);
                let hu = (gh * (u / mx).sqrt()).max(1.5);
                ctx.rrect(bx, gb - hd, bw, hd, bw / 2.0, fade([120, 200, 255, 200], a));
                ctx.rrect(bx + bw * 0.25, gb - hu, bw * 0.5, hu, bw / 4.0, fade([255, 150, 90, 220], a));
            }
            None => ctx.rrect(bx, gb - 1.5, bw, 1.5, 0.75, fade([255, 255, 255, 22], a)),
        }
    }

    // ---- volume
    let r = lay::st_volume(x0, w);
    let (ix, iy) = lay::slider_icon(r);
    let hov = inside(cur, r);
    let vlabel = if snap.audio_out.is_empty() { "Volume" } else { snap.audio_out.as_str() };
    let vval = if snap.muted { "Muted".to_string() } else { format!("{}%", (snap.volume * 100.0).round() as i32) };
    slider_tile(ctx, r, vlabel, &vval, Some(snap.volume), snap.muted, hov, a);
    if near(cur, (ix, iy), 15.0) {
        ctx.circle(ix, iy, 14.0, fade([255, 255, 255, 22], a));
    }
    icon_speaker(ctx, ix, iy, snap.volume, snap.muted, fade(if snap.muted { RED } else { WHITE }, a));

    // ---- brightness
    let r = lay::st_bright(x0, w);
    let (ix, iy) = lay::slider_icon(r);
    match snap.brightness {
        Some(b) => {
            slider_tile(ctx, r, "Brightness", &format!("{}%", (b * 100.0).round() as i32), Some(b), false, inside(cur, r), a);
            icon_sun(ctx, ix, iy, fade([255, 214, 110, 255], a));
        }
        None => {
            slider_tile(ctx, r, "Brightness", "n/a", None, true, false, a);
            icon_sun(ctx, ix, iy, fade(DIM, a));
        }
    }

    // ---- perangkat bluetooth
    let r = lay::st_devices(x0, w);
    tile(ctx, r, inside(cur, r), a);
    icon_bt(ctx, r.0 + 21.0, r.1 + r.3 / 2.0, fade(ACCENT, a));
    let mut x = r.0 + 40.0;
    let right = r.0 + r.2 - 8.0;
    if snap.devices.is_empty() {
        ctx.text("No Bluetooth devices connected", x, r.1 + 15.0, 12.0, fade(GRAY, a), right - x);
    }
    for (i, d) in snap.devices.iter().enumerate() {
        let bat = d.battery.map(|p| format!("{p}%"));
        let bw_ = bat.as_ref().map(|b| text_w(ctx, b, 10.5, true) + 6.0).unwrap_or(0.0);
        let nw = text_w(ctx, &d.name, 11.5, false).min(130.0);
        let cw = 28.0 + nw + bw_ + 12.0;
        let more = snap.devices.len() - i;
        if x + cw > right && i > 0 {
            let t = format!("+{more}");
            let tw = text_w(ctx, &t, 11.5, true) + 18.0;
            ctx.rrect(x, r.1 + 8.0, tw, 28.0, 14.0, fade(TILE_HI, a));
            center(ctx, &t, x + tw / 2.0, r.1 + 15.0, 11.5, fade(GRAY, a), tw, true);
            break;
        }
        let cw = cw.min(right - x);
        ctx.rrect(x, r.1 + 8.0, cw, 28.0, 14.0, fade([255, 255, 255, 20], a));
        icon_device(ctx, x + 15.0, r.1 + 22.0, d.kind, fade(WHITE, a));
        let tw = ctx.text(&d.name, x + 28.0, r.1 + 14.5, 11.5, fade(WHITE, a), (cw - 40.0 - bw_).max(10.0));
        if let Some(b) = &bat {
            let col = if d.battery.is_some_and(|p| p <= 20) { RED } else { GREEN };
            ctx.text_b(b, x + 28.0 + tw + 6.0, r.1 + 15.0, 10.5, fade(col, a), bw_);
        }
        x += cw + 6.0;
    }

    // ---- quit
    let r = lay::quit(x0, w);
    let hov = inside(cur, r);
    ctx.rrect(r.0, r.1, r.2, r.3, 13.0, fade(if hov { [240, 90, 90, 60] } else { TILE }, a));
    ctx.rrect_stroke(r.0, r.1, r.2, r.3, 13.0, 1.0, fade(if hov { [240, 90, 90, 90] } else { TILE_EDGE }, a));
    icon_power(ctx, r.0 + r.2 / 2.0, r.1 + r.3 / 2.0, fade(if hov { [255, 120, 120, 255] } else { GRAY }, a));
}

/// Progress bar timeline (bisa digeser).
fn draw_bar(ctx: &mut Ctx, b: Rect, snap: &Snap, a: f32) {
    ctx.rrect(b.0, b.1, b.2, b.3, 2.0, fade(TRACK, a));
    if snap.dur > 0 {
        let f = (snap.pos as f32 / snap.dur as f32).clamp(0.0, 1.0);
        ctx.rrect(b.0, b.1, (b.2 * f).max(4.0), b.3, 2.0, fade(WHITE, a));
        ctx.circle(b.0 + b.2 * f, b.1 + b.3 / 2.0, 5.0, fade(WHITE, a));
    }
}

fn draw_scrollbar(ctx: &mut Ctx, x0: f32, w: f32, len: usize, scroll: usize, a: f32) {
    if len <= lay::ROWS {
        return;
    }
    let (top, hgt) = (34.0, lay::ROWS as f32 * 27.0 - 3.0);
    let th = (hgt * lay::ROWS as f32 / len as f32).max(14.0);
    let ty = top + (hgt - th) * scroll as f32 / (len - lay::ROWS) as f32;
    ctx.rrect(x0 + w - 28.0, ty, 3.0, th, 1.5, fade(DIM, a));
}

/// Mode lirik inline: media ringkas di kiri, daftar lirik di kanan.
#[allow(clippy::too_many_arguments)]
fn draw_inline(ctx: &mut Ctx, snap: &Snap, x0: f32, w: f32, h: f32, a: f32, want_pip: bool, sc: &mut LyricScroll, hits: &mut LyricHits) {
    let (adx, ay, asz) = lay::L_ART;
    match &snap.art {
        Some(img) => ctx.image(img, x0 + adx, ay, asz, 10.0, a),
        None => ctx.rrect(x0 + adx, ay, asz, asz, 10.0, fade(CHIP, a)),
    }
    let tx = x0 + adx + asz + 12.0;
    let tw = x0 + lay::L_SPLIT - 14.0 - tx;
    let title = if snap.title.is_empty() { "Nothing playing" } else { &snap.title };
    ctx.text_b(title, tx, 20.0, 14.5, fade(WHITE, a), tw);
    ctx.text(&snap.artist, tx, 40.0, 11.5, fade(GRAY, a), tw);

    let py = lay::L_PROG_Y;
    ctx.text(&mmss(snap.pos), x0 + 22.0, py, 12.5, fade(WHITE, a), 46.0);
    let rem = format!("-{}", mmss(snap.dur - snap.pos));
    let rw = text_w(ctx, &rem, 12.5, false);
    ctx.text(&rem, x0 + lay::L_SPLIT - 16.0 - rw, py, 12.5, fade(WHITE, a), 60.0);
    draw_bar(ctx, lay::l_bar(x0), snap, a);
    let c = lay::l_ctrls(x0);
    icon_lyrics(ctx, c[0].0, c[0].1, fade(ACCENT, a));
    icon_prev(ctx, c[1].0, c[1].1, fade(WHITE, a));
    icon_play(ctx, c[2].0, c[2].1, snap.playing, fade(WHITE, a));
    icon_next(ctx, c[3].0, c[3].1, fade(WHITE, a));

    // pemisah + panel lirik
    ctx.rrect(x0 + lay::L_SPLIT, 14.0, 1.0, h - 28.0, 0.5, fade([80, 80, 88, 255], a));
    icon_popout(ctx, lay::l_popout(x0, w), want_pip, a);
    let lx = x0 + lay::L_SPLIT + 18.0;
    let lw = x0 + w - 40.0 - lx;
    if !snap.has_lyrics {
        hits.clear();
        let msg = if snap.title.is_empty() { "Nothing playing" } else { "No synced lyrics found" };
        ctx.text(msg, lx, h / 2.0 - 10.0, 14.0, fade(GRAY, a), lw);
        return;
    }
    let st = ListStyle { x: lx, w: lw, top: 10.0, bottom: h - 10.0, cpx: 15.0, opx: 15.0, gap: 6.0, anchor: 0.45, centered: false };
    draw_lyric_list(ctx, snap, &st, sc, a, hits);
}

// ---------------------------------------------------------------- event loop
impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.notch.is_some() {
            return;
        }
        let win = Win::new(el, WIN_W, WIN_H, |s, el| {
            let (mx, my, mw) = el
                .primary_monitor()
                .map(|m| (m.position().x, m.position().y, m.size().width as i32))
                .unwrap_or((0, 0, 1920));
            PhysicalPosition::new(mx + (mw - (WIN_W * s) as i32) / 2, my)
        });
        self.notch = Some(win);
        self.w.cur = 150.0;
        self.h.cur = 32.0;
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(16)));
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, ev: WindowEvent) {
        let is_notch = self.notch.as_ref().is_some_and(|w| w.window.id() == id);
        let is_pip = self.pip.as_ref().is_some_and(|w| w.window.id() == id);
        match ev {
            WindowEvent::CloseRequested if is_notch => el.exit(),
            WindowEvent::CloseRequested if is_pip => self.want_pip = false,
            WindowEvent::CursorEntered { .. } if is_notch => self.hover = true,
            WindowEvent::CursorLeft { .. } if is_notch => {
                self.hover = false;
                self.left_at = Instant::now();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let w = if is_notch { self.notch.as_ref() } else { self.pip.as_ref() };
                if let Some(w) = w {
                    self.cursor = (position.x as f32 / w.s, position.y as f32 / w.s);
                }
                if is_pip && self.press.is_none() {
                    if let Some(w) = &self.pip {
                        let (pw, ph) = (w.pm.width() as f32 / w.s, w.pm.height() as f32 / w.s);
                        w.window.set_cursor(edge(self.cursor, pw, ph).map(resize_icon).unwrap_or(CursorIcon::Default));
                    }
                }
                if self.press.is_some() {
                    self.drag_move(is_pip);
                }
                if is_notch && matches!(self.page, Page::Shelf | Page::Clipboard | Page::Todo | Page::Status) {
                    self.last_sig = 0; // efek hover baris
                }
            }
            WindowEvent::Resized(size) if is_pip && size.width > 0 && size.height > 0 => {
                if let Some(w) = self.pip.as_mut() {
                    if let Some(pm) = Pixmap::new(size.width, size.height) {
                        w.pm = pm;
                        self.pip_size = (size.width as f32 / w.s, size.height as f32 / w.s);
                        self.cfg_dirty = Some(Instant::now());
                        w.window.request_redraw();
                    }
                }
            }
            WindowEvent::Moved(pos) if is_pip => {
                self.pip_pos = Some((pos.x, pos.y));
                self.cfg_dirty = Some(Instant::now());
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if is_pip {
                    self.pip_font = (self.pip_font + dy.signum() * 0.1).clamp(0.6, 3.0);
                    self.cfg_dirty = Some(Instant::now());
                } else if is_notch && dy != 0.0 {
                    self.wheel_notch(dy);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if is_notch => self.key(&event),
            WindowEvent::Focused(false) if is_notch => self.end_edit(true),
            WindowEvent::HoveredFile(_) if is_notch => {
                self.file_hover = true;
                self.set_page(Page::Shelf);
                self.lyrics_inline = false;
            }
            WindowEvent::HoveredFileCancelled if is_notch => {
                self.file_hover = false;
                self.left_at = Instant::now();
            }
            WindowEvent::DroppedFile(p) if is_notch => {
                self.file_hover = false;
                self.set_page(Page::Shelf);
                let mut s = self.shared.lock().unwrap();
                if !s.shelf.contains(&p) {
                    s.shelf.push(p);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => match (state, button) {
                (ElementState::Pressed, MouseButton::Left) if is_notch => self.press_notch(),
                (ElementState::Pressed, MouseButton::Right) if is_notch => self.right_click_notch(),
                (ElementState::Pressed, MouseButton::Left) if is_pip => {
                    let p = self.cursor;
                    let (pw, ph) = self.pip_size;
                    if p.0 > pw - 32.0 && p.1 < 28.0 {
                        self.want_pip = false;
                    } else if let Some(d) = edge(p, pw, ph) {
                        if let Some(win) = &self.pip {
                            let _ = win.window.drag_resize_window(d);
                        }
                    } else {
                        self.press = Some(Press::Pip(p));
                    }
                }
                (ElementState::Pressed, MouseButton::Right) if is_pip => self.want_pip = false,
                (ElementState::Released, MouseButton::Left) => self.release(),
                _ => {}
            },
            WindowEvent::RedrawRequested => {
                let mut snap = Snap::take(&self.shared.lock().unwrap());
                if let Some(f) = self.seek {
                    snap.pos = (f as f64 * snap.dur as f64) as i64;
                }
                if is_notch {
                    let now = Instant::now();
                    let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
                    self.last_frame = now;
                    let (tw, th) = self.target(&snap);
                    self.w.step_shape(tw, dt);
                    self.h.step_shape(th, dt);
                    self.shape_moving = !(self.w.settled(tw) && self.h.settled(th));
                    let td = Instant::now();
                    self.draw_notch(&snap);
                    if let Some((dir, _)) = &self.animtest {
                        use std::io::Write;
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("draw.txt")) {
                            let _ = writeln!(f, "{} {}", td.elapsed().as_micros(), self.shape_moving as u8);
                        }
                    }
                    if self.vsync {
                        gfx::wait_vsync();
                    }
                } else if is_pip {
                    self.draw_pip(&snap);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.quit {
            self.end_edit(true);
            if self.cfg_dirty.is_some() {
                self.save_cfg();
            }
            el.exit();
            return;
        }
        self.sync_pip(el);
        if self.cliptest && self.t0.elapsed() > Duration::from_secs(3) {
            self.cliptest = false;
            let item = self.shared.lock().unwrap().clips.first().cloned();
            if let (Some(item), Some(win)) = (item, self.notch.as_ref()) {
                let ok = clip::set(win.hwnd, &item);
                let _ = std::fs::write(std::env::temp_dir().join("notchy_cliptest.txt"), if ok { "ok" } else { "fail" });
            }
        }
        if let Some(msg) = self.pomo.tick() {
            self.shared.lock().unwrap().flash(msg);
            unsafe {
                let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK);
            }
        }
        let snap = Snap::take(&self.shared.lock().unwrap());
        let (tw, th) = self.target(&snap);
        let notch_anim = !(self.w.settled(tw) && self.h.settled(th));
        let scroll_anim = (self.pip.is_some() && !self.pip_scroll.off.settled(0.0))
            || (self.inline_on() && self.expanded() && !self.inline_scroll.off.settled(0.0));
        // visualizer bar hanya bergerak saat musik jalan & bar sedang terlihat
        let bars_anim = snap.playing
            && snap.activity.is_none()
            && ((!self.expanded() && !self.lyric_pill(&snap)) || (self.expanded() && self.page == Page::Media && !self.inline_on()));
        let pill_anim = self.lyric_pill(&snap) && self.pill.at.elapsed() < Duration::from_millis(320);
        let caret_anim = self.editing.is_some();
        let seek_anim = self.seek.is_some();

        let mut hs = DefaultHasher::new();
        (&snap.title, &snap.artist, snap.playing, snap.pos / 1000, snap.idx, &snap.activity, &snap.shelf).hash(&mut hs);
        (snap.battery, (snap.volume * 100.0) as i32, snap.has_lyrics, snap.art.as_ref().map(|a| Arc::as_ptr(a) as usize)).hash(&mut hs);
        (self.hover, self.file_hover, self.want_pip, self.pip.is_some(), self.page, self.lyrics_inline).hash(&mut hs);
        ((self.pip_font * 10.0) as i32, self.pip_size.0 as i32, self.pip_size.1 as i32, local_time().1).hash(&mut hs);
        (snap.clips.len(), snap.clips_rev, self.scroll, &self.editing, self.todos.items.len()).hash(&mut hs);
        (self.pomo.left().as_secs(), self.pomo.running(), self.pomo.phase).hash(&mut hs);
        (snap.muted, &snap.audio_out, snap.brightness.map(|b| (b * 100.0).round() as i32), &snap.devices).hash(&mut hs);
        if self.expanded() && self.page == Page::Status {
            let pw = snap.power.as_ref().map(|p| (p.rate_mw / 100, p.secs.map(|s| s / 60), p.saver, p.remain_mwh / 100, p.full_mwh));
            (pw, snap.net.down as u32, snap.net.up as u32, snap.net.kind, &snap.net.name, snap.net.hist.len()).hash(&mut hs);
        }
        let sig = hs.finish();

        let changed = sig != self.last_sig;
        let anim = notch_anim || bars_anim || scroll_anim || pill_anim || caret_anim || seek_anim;
        // frame pacing: redraw animasi hanya kalau sudah waktunya (hindari busy loop)
        let interval = if notch_anim || scroll_anim || pill_anim || seek_anim {
            12
        } else if bars_anim {
            66
        } else {
            250
        };
        let now = Instant::now();
        // animasi utama: redraw tiap frame layar, dipacing DwmFlush (vsync) di RedrawRequested
        self.vsync = notch_anim || scroll_anim || pill_anim || seek_anim;
        if self.vsync {
            self.next_frame = now;
        }
        if changed || (anim && now >= self.next_frame) {
            self.last_sig = sig;
            self.next_frame = now + Duration::from_millis(interval);
            if let Some(w) = &self.notch {
                w.window.request_redraw();
            }
            if scroll_anim || changed || seek_anim {
                if let Some(w) = &self.pip {
                    w.window.request_redraw();
                }
            }
        }
        if self.cfg_dirty.is_some_and(|t| t.elapsed() > Duration::from_millis(400)) {
            self.cfg_dirty = None;
            self.save_cfg();
        }
        let idle = if self.pomo.running() { 250 } else { 120 };
        if self.vsync {
            el.set_control_flow(ControlFlow::Poll);
        } else {
            let wake = if anim { self.next_frame.max(now + Duration::from_millis(1)) } else { now + Duration::from_millis(idle) };
            el.set_control_flow(ControlFlow::WaitUntil(wake));
        }
    }
}

// ---------------------------------------------------------------- resize PiP
/// Zona tepi 8px untuk resize PiP (koordinat logical).
fn edge(p: (f32, f32), w: f32, h: f32) -> Option<ResizeDirection> {
    let (l, r, t, b) = (p.0 < 8.0, p.0 > w - 8.0, p.1 < 8.0, p.1 > h - 8.0);
    Some(match (l, r, t, b) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return None,
    })
}

fn resize_icon(d: ResizeDirection) -> CursorIcon {
    match d {
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::NwseResize,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::NeswResize,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::EwResize,
        _ => CursorIcon::NsResize,
    }
}

// ---------------------------------------------------------------- config
fn cfg_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("notchy").join("pip.cfg"))
}

struct Cfg {
    pip_size: (f32, f32),
    pip_font: f32,
    pip_pos: Option<(i32, i32)>,
    lyrics: bool,
    pomo: (u32, u32, u32, u32),
}

/// Format baris `key=value`. Format lama "w h font x y" tetap terbaca.
fn load_cfg() -> Cfg {
    let mut c = Cfg { pip_size: (PIP_W, PIP_H), pip_font: 1.0, pip_pos: None, lyrics: false, pomo: (25, 5, 15, 4) };
    let Some(t) = cfg_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return c };
    let (mut x, mut y) = (None, None);
    if t.contains('=') {
        for line in t.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            let num = |d: u32| v.parse::<u32>().unwrap_or(d).max(1);
            match k.trim() {
                "pip_w" => c.pip_size.0 = v.parse().unwrap_or(PIP_W),
                "pip_h" => c.pip_size.1 = v.parse().unwrap_or(PIP_H),
                "pip_font" => c.pip_font = v.parse().unwrap_or(1.0),
                "pip_x" => x = v.parse().ok(),
                "pip_y" => y = v.parse().ok(),
                "lyrics" => c.lyrics = v == "1",
                "pomo_work" => c.pomo.0 = num(25),
                "pomo_break" => c.pomo.1 = num(5),
                "pomo_long" => c.pomo.2 = num(15),
                "pomo_every" => c.pomo.3 = num(4),
                _ => {}
            }
        }
    } else {
        let v: Vec<&str> = t.split_whitespace().collect();
        let f = |i: usize| v.get(i).and_then(|x| x.parse::<f32>().ok());
        if let (Some(w), Some(h), Some(fs)) = (f(0), f(1), f(2)) {
            c.pip_size = (w, h);
            c.pip_font = fs;
        }
        x = v.get(3).and_then(|s| s.parse().ok());
        y = v.get(4).and_then(|s| s.parse().ok());
    }
    if let (Some(x), Some(y)) = (x, y) {
        c.pip_pos = Some((x, y));
    }
    c.pip_size = (c.pip_size.0.max(220.0), c.pip_size.1.max(80.0));
    c.pip_font = c.pip_font.clamp(0.6, 3.0);
    c
}

fn main() {
    // satu instance saja (startup + buka manual tidak dobel)
    let _guard = unsafe {
        use windows::Win32::{
            Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
            System::Threading::CreateMutexW,
        };
        let h = CreateMutexW(None, true, windows::core::w!(r"Local\notchy-single-instance"));
        // NOTCHY_DUMP (debug screenshot) boleh jalan berdampingan dengan instance utama
        if GetLastError() == ERROR_ALREADY_EXISTS && std::env::var_os("NOTCHY_DUMP").is_none() {
            return;
        }
        h
    };
    let shared: State = Arc::new(Mutex::new(Shared::default()));
    let (tx, rx) = mpsc::channel();
    media::spawn(shared.clone(), rx);
    let sys_tx = sys::spawn(shared.clone());
    let bright_tx = bright::spawn(shared.clone());
    devices::spawn(shared.clone());
    clip::spawn(shared.clone());

    let cfg = load_cfg();
    let el = EventLoop::new().unwrap();
    let mut app = App {
        shared,
        tx,
        sys_tx,
        bright_tx,
        fonts: Fonts::load(r"C:\Windows\Fonts\segoeui.ttf"),
        bold: Fonts::load(r"C:\Windows\Fonts\segoeuib.ttf"),
        notch: None,
        pip: None,
        page: Page::Media,
        lyrics_inline: match std::env::var("NOTCHY_LYRICS").ok().as_deref() {
            Some("0") => false,
            Some(_) => true,
            None => cfg.lyrics || std::env::var("NOTCHY_EXPAND").is_ok_and(|v| v == "lyrics"),
        },
        want_pip: false,
        hover: false,
        file_hover: false,
        left_at: Instant::now() - Duration::from_secs(10),
        cursor: (0.0, 0.0),
        w: Spring::default(),
        h: Spring::default(),
        last_frame: Instant::now(),
        last_sig: 0,
        t0: Instant::now(),
        inline_scroll: LyricScroll::default(),
        pip_scroll: LyricScroll::default(),
        inline_hits: Vec::new(),
        pip_hits: Vec::new(),
        pill: PillLyric::default(),
        pip_size: cfg.pip_size,
        pip_font: cfg.pip_font,
        pip_pos: cfg.pip_pos,
        cfg_dirty: None,
        force_expand: std::env::var_os("NOTCHY_EXPAND").is_some(),
        next_frame: Instant::now(),
        vsync: false,
        shape_moving: true,
        animtest: std::env::var_os("NOTCHY_ANIMTEST").map(|d| (PathBuf::from(d), Instant::now())),
        anim_n: 0,
        quit: false,
        press: None,
        seek: None,
        last_click: None,
        scroll: 0,
        todos: Todos::load(),
        editing: None,
        pomo: Pomo::new(cfg.pomo.0, cfg.pomo.1, cfg.pomo.2, cfg.pomo.3),
        cliptest: std::env::var_os("NOTCHY_CLIPTEST").is_some(),
    };
    // debug: NOTCHY_PAGE=shelf|clipboard|todo|timer|status
    if let Ok(p) = std::env::var("NOTCHY_PAGE") {
        app.page = match p.as_str() {
            "shelf" => Page::Shelf,
            "clipboard" => Page::Clipboard,
            "todo" => Page::Todo,
            "timer" => Page::Timer,
            "status" => Page::Status,
            _ => Page::Media,
        };
    }
    el.run_app(&mut app).unwrap();
}
