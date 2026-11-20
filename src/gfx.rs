use ab_glyph::{point, Font as _, FontRef, PxScale, ScaleFont};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::OnceLock};
use std::ffi::c_void;
use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pattern, Pixmap, Point,
    SpreadMode, Transform,
};
use windows::Win32::{
    Foundation::{COLORREF, HWND, POINT, SIZE},
    Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, HBITMAP, HDC, HGDIOBJ,
        AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    },
    UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, UpdateLayeredWindow, GWL_EXSTYLE, ULW_ALPHA, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    },
};

pub type Rgba = [u8; 4];

struct Slot {
    path: &'static str,
    index: u32,
    font: OnceLock<Option<FontRef<'static>>>,
}

impl Slot {
    fn get(&self) -> Option<&FontRef<'static>> {
        self.font
            .get_or_init(|| {
                // ab_glyph parse outline secara lazy -> hanya ~ukuran file di RAM; dimuat saat dibutuhkan
                let bytes: &'static [u8] = Box::leak(std::fs::read(self.path).ok()?.into_boxed_slice());
                FontRef::try_from_slice_and_index(bytes, self.index).ok()
            })
            .as_ref()
    }
}

/// Bitmap coverage glyph yang sudah di-rasterize (offset relatif ke titik pen/baseline).
struct GlyphBmp {
    x: i32,
    y: i32,
    w: usize,
    cov: Vec<u8>,
}

pub struct Fonts {
    slots: Vec<Slot>,
    /// cache rasterisasi: (slot, glyph id, ukuran px*16) -> bitmap
    glyphs: RefCell<HashMap<(usize, u16, u32), Option<Rc<GlyphBmp>>>>,
    /// cache hasil wrap: (teks, px*16, lebar*4) -> baris
    wraps: RefCell<HashMap<(String, u32, u32), Rc<Vec<String>>>>,
}

fn scale(f: &FontRef, px: f32) -> PxScale {
    // ab_glyph: PxScale = tinggi (ascent-descent); konversi dari ukuran em
    let em = f.units_per_em().unwrap_or(1000.0);
    PxScale::from(px * f.height_unscaled() / em)
}

impl Fonts {
    pub fn load(main: &'static str) -> Self {
        let slot = |path, index| Slot { path, index, font: OnceLock::new() };
        let slots = vec![
            slot(main, 0),
            slot(r"C:\Windows\Fonts\malgun.ttf", 0),  // Korea
            slot(r"C:\Windows\Fonts\YuGothR.ttc", 0), // Jepang
            slot(r"C:\Windows\Fonts\msyh.ttc", 0),    // Mandarin
        ];
        slots[0].get().expect("font utama tidak ada/rusak");
        Self { slots, glyphs: RefCell::default(), wraps: RefCell::default() }
    }

    fn pick(&self, c: char) -> &FontRef<'static> {
        self.pick_idx(c).1
    }

    fn pick_idx(&self, c: char) -> (usize, &FontRef<'static>) {
        for (i, sl) in self.slots.iter().enumerate() {
            if let Some(f) = sl.get() {
                if f.glyph_id(c).0 != 0 {
                    return (i, f);
                }
            }
        }
        (0, self.slots[0].get().unwrap())
    }

    fn glyph(&self, c: char, px: f32) -> Option<Rc<GlyphBmp>> {
        let (si, font) = self.pick_idx(c);
        let gid = font.glyph_id(c);
        let key = (si, gid.0, (px * 16.0) as u32);
        if let Some(g) = self.glyphs.borrow().get(&key) {
            return g.clone();
        }
        let g = font.outline_glyph(gid.with_scale_and_position(scale(font, px), point(0.0, 0.0))).map(|og| {
            let bb = og.px_bounds();
            let (w, h) = (bb.width() as usize, bb.height() as usize);
            let mut cov = vec![0u8; w * h];
            og.draw(|x, y, v| {
                let (x, y) = (x as usize, y as usize);
                if x < w && y < h {
                    cov[y * w + x] = (v.clamp(0.0, 1.0) * 255.0) as u8;
                }
            });
            Rc::new(GlyphBmp { x: bb.min.x as i32, y: bb.min.y as i32, w, cov })
        });
        let mut cache = self.glyphs.borrow_mut();
        if cache.len() > 6000 {
            cache.clear();
        }
        cache.insert(key, g.clone());
        g
    }

    /// Bungkus teks per kata (atau per karakter untuk CJK) agar muat di `max_w` (px fisik). Hasil di-cache.
    pub fn wrap(&self, text: &str, px: f32, max_w: f32) -> Rc<Vec<String>> {
        let key = (text.to_string(), (px * 16.0) as u32, (max_w * 4.0) as u32);
        if let Some(v) = self.wraps.borrow().get(&key) {
            return v.clone();
        }
        let mut out = Vec::new();
        let mut cur: Vec<char> = Vec::new();
        let mut cur_w = 0.0;
        let mut last_space: Option<usize> = None;
        for ch in text.chars() {
            cur.push(ch);
            cur_w += self.advance(ch, px);
            if ch == ' ' {
                last_space = Some(cur.len() - 1);
            }
            if cur.len() > 1 && cur_w > max_w {
                let cut = match last_space {
                    Some(i) if i > 0 => i,
                    _ => cur.len() - 1,
                };
                let head: String = cur[..cut].iter().collect();
                out.push(head.trim_end().to_string());
                cur = cur[cut..].iter().copied().skip_while(|c| *c == ' ').collect();
                cur_w = cur.iter().map(|c| self.advance(*c, px)).sum();
                last_space = cur.iter().rposition(|c| *c == ' ');
            }
        }
        if !cur.is_empty() {
            out.push(cur.iter().collect());
        }
        let v = Rc::new(out);
        let mut cache = self.wraps.borrow_mut();
        if cache.len() > 2000 {
            cache.clear();
        }
        cache.insert(key, v.clone());
        v
    }

    fn advance(&self, c: char, px: f32) -> f32 {
        let f = self.pick(c);
        f.as_scaled(scale(f, px)).h_advance(f.glyph_id(c))
    }

    pub fn width(&self, text: &str, px: f32) -> f32 {
        text.chars().map(|c| self.advance(c, px)).sum()
    }

    /// Potong teks dengan "…" agar muat di `max_w`.
    pub fn fit(&self, text: &str, px: f32, max_w: f32) -> String {
        if self.width(text, px) <= max_w {
            return text.to_string();
        }
        let mut out = String::new();
        let mut w = self.width("…", px);
        for c in text.chars() {
            let cw = self.advance(c, px);
            if w + cw > max_w {
                break;
            }
            w += cw;
            out.push(c);
        }
        out.push('…');
        out
    }
}

/// Konteks gambar: koordinat logical, otomatis dikali `s` (DPI scale).
pub struct Ctx<'a> {
    pub pm: &'a mut Pixmap,
    pub fonts: &'a Fonts,
    pub bold: &'a Fonts,
    pub s: f32,
}

impl Ctx<'_> {
    fn rrect_path(&self, x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
        rrect_path_s(self.s, x, y, w, h, r)
    }

    pub fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgba) {
        if let Some(path) = self.rrect_path(x, y, w, h, r) {
            let mut p = Paint::default();
            p.set_color_rgba8(c[0], c[1], c[2], c[3]);
            p.anti_alias = true;
            self.pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
        }
    }

    /// Rounded rect dengan gradient vertikal: `stops` = (posisi 0..1 relatif tinggi, warna).
    pub fn rrect_vgrad(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, stops: &[(f32, Rgba)]) {
        let Some(path) = self.rrect_path(x, y, w, h, r) else { return };
        let st = stops
            .iter()
            .map(|(p, c)| GradientStop::new(*p, Color::from_rgba8(c[0], c[1], c[2], c[3])))
            .collect();
        let Some(shader) = LinearGradient::new(
            Point::from_xy(0.0, y * self.s),
            Point::from_xy(0.0, (y + h) * self.s),
            st,
            SpreadMode::Pad,
            Transform::identity(),
        ) else {
            return;
        };
        let mut p = Paint::default();
        p.shader = shader;
        p.anti_alias = true;
        self.pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
    }

    /// Gambar pixmap persegi `img` ke kotak (x, y, size) dengan sudut membulat.
    pub fn image(&mut self, img: &Pixmap, x: f32, y: f32, size: f32, r: f32, alpha: f32) {
        let Some(path) = self.rrect_path(x, y, size, size, r) else { return };
        let k = size * self.s / img.width() as f32;
        let tf = Transform::from_row(k, 0.0, 0.0, k, x * self.s, y * self.s);
        let mut p = Paint::default();
        p.shader = Pattern::new(img.as_ref(), SpreadMode::Pad, FilterQuality::Bilinear, alpha, tf);
        p.anti_alias = true;
        self.pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
    }

    /// Gambar pixmap ke kotak (x, y, w, h) tanpa crop (skala non-seragam jika perlu).
    pub fn image_rect(&mut self, img: &Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, alpha: f32) {
        let Some(path) = self.rrect_path(x, y, w, h, r) else { return };
        let (kx, ky) = (w * self.s / img.width() as f32, h * self.s / img.height() as f32);
        let tf = Transform::from_row(kx, 0.0, 0.0, ky, x * self.s, y * self.s);
        let mut p = Paint::default();
        p.shader = Pattern::new(img.as_ref(), SpreadMode::Pad, FilterQuality::Bilinear, alpha, tf);
        p.anti_alias = true;
        self.pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        self.rrect(cx - r, cy - r, r * 2.0, r * 2.0, r, c);
    }

    /// Busur lingkaran (stroke), mulai dari jam 12 searah jarum jam sepanjang `frac` (0..1).
    pub fn arc(&mut self, cx: f32, cy: f32, r: f32, width: f32, frac: f32, c: Rgba) {
        let frac = frac.clamp(0.0, 1.0);
        if frac <= 0.0 {
            return;
        }
        let s = self.s;
        let n = ((48.0 * frac).ceil() as usize).max(2);
        let mut pb = PathBuilder::new();
        for i in 0..=n {
            let t = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * i as f32 / n as f32;
            let (x, y) = ((cx + r * t.cos()) * s, (cy + r * t.sin()) * s);
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        if frac >= 1.0 {
            pb.close();
        }
        if let Some(path) = pb.finish() {
            let mut p = Paint::default();
            p.set_color_rgba8(c[0], c[1], c[2], c[3]);
            p.anti_alias = true;
            let st = tiny_skia::Stroke { width: width * s, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
            self.pm.stroke_path(&path, &p, &st, Transform::identity(), None);
        }
    }

    /// Segitiga (ikon play/next/prev). Titik logical.
    pub fn poly(&mut self, pts: &[(f32, f32)], c: Rgba) {
        let mut pb = PathBuilder::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(x * self.s, y * self.s);
            } else {
                pb.line_to(x * self.s, y * self.s);
            }
        }
        pb.close();
        if let Some(path) = pb.finish() {
            let mut p = Paint::default();
            p.set_color_rgba8(c[0], c[1], c[2], c[3]);
            p.anti_alias = true;
            self.pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
        }
    }

    /// Gambar teks; (x, y) = pojok kiri-atas kotak teks. Mengembalikan lebar.
    pub fn text(&mut self, text: &str, x: f32, y: f32, px: f32, c: Rgba, max_w: f32) -> f32 {
        let f = self.fonts;
        self.text_with(f, text, x, y, px, c, max_w)
    }

    pub fn text_b(&mut self, text: &str, x: f32, y: f32, px: f32, c: Rgba, max_w: f32) -> f32 {
        let f = self.bold;
        self.text_with(f, text, x, y, px, c, max_w)
    }

    fn text_with(&mut self, fonts: &Fonts, text: &str, x: f32, y: f32, px: f32, c: Rgba, max_w: f32) -> f32 {
        let fpx = px * self.s;
        let t = fonts.fit(text, fpx, max_w * self.s);
        let (pw, ph) = (self.pm.width() as i32, self.pm.height() as i32);
        let base = y * self.s + fpx * 0.82;
        let mut pen = x * self.s;
        let start = pen;
        let data = self.pm.data_mut();
        let by = base.round() as i32;
        let alpha = c[3] as f32 / 255.0;
        for ch in t.chars() {
            if let Some(g) = fonts.glyph(ch, fpx) {
                let (ox, oy) = (pen.round() as i32 + g.x, by + g.y);
                let h = g.cov.len() / g.w.max(1);
                for gy in 0..h {
                    let y_ = oy + gy as i32;
                    if y_ < 0 || y_ >= ph {
                        continue;
                    }
                    for gx in 0..g.w {
                        let v = g.cov[gy * g.w + gx];
                        if v == 0 {
                            continue;
                        }
                        let x_ = ox + gx as i32;
                        if x_ < 0 || x_ >= pw {
                            continue;
                        }
                        let cov = v as f32 / 255.0 * alpha;
                        let i = ((y_ * pw + x_) * 4) as usize;
                        let inv = 1.0 - cov;
                        for k in 0..3 {
                            data[i + k] = (c[k] as f32 * cov + data[i + k] as f32 * inv) as u8;
                        }
                        data[i + 3] = (255.0 * cov + data[i + 3] as f32 * inv) as u8;
                    }
                }
            }
            pen += fonts.advance(ch, fpx);
        }
        (pen - start) / self.s
    }
}

/// Path rounded-rect dalam koordinat fisik (logical * s).
fn rrect_path_s(s: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let (x, y, w, h) = (x * s, y * s, w * s, h * s);
    let r = (r * s).min(w / 2.0).min(h / 2.0);
    let k = r * 0.5523;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Salin isi `src` ke `dst` hanya di dalam bentuk rounded-rect (anti-alias), tanpa mask satu layar penuh.
pub fn fill_rrect_from(dst: &mut Pixmap, src: &Pixmap, s: f32, x: f32, y: f32, w: f32, h: f32, r: f32) {
    let Some(path) = rrect_path_s(s, x, y, w, h, r) else { return };
    let mut p = Paint::default();
    p.shader = Pattern::new(src.as_ref(), SpreadMode::Pad, FilterQuality::Nearest, 1.0, Transform::identity());
    p.anti_alias = true;
    dst.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
}

/// Tunggu frame komposisi DWM berikutnya (sinkron vsync) -> animasi rata, tanpa timer 15.6 ms.
pub fn wait_vsync() {
    unsafe {
        if windows::Win32::Graphics::Dwm::DwmFlush().is_err() {
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
    }
}

pub fn hwnd_of(window: &winit::window::Window) -> HWND {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().unwrap().as_raw() {
        RawWindowHandle::Win32(h) => HWND(h.hwnd.get() as *mut c_void),
        _ => unreachable!(),
    }
}

/// Jadikan window layered (per-pixel alpha), always-on-top, tanpa fokus & taskbar.
pub fn make_overlay(hwnd: HWND) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let add = (WS_EX_LAYERED.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0 | WS_EX_NOACTIVATE.0) as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | add);
    }
}

/// Izinkan/larang window menerima fokus keyboard (untuk mengetik di todo).
pub fn set_activatable(hwnd: HWND, on: bool) {
    use windows::Win32::UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::SetForegroundWindow};
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let na = WS_EX_NOACTIVATE.0 as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, if on { ex & !na } else { ex | na });
        if on {
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(Some(hwnd));
        }
    }
}

/// DIB + DC yang dipakai ulang per window (tidak alokasi tiap frame).
struct Surface {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

thread_local! {
    static SURFACES: std::cell::RefCell<std::collections::HashMap<isize, Surface>> = Default::default();
}

fn make_surface(w: i32, h: i32) -> Option<Surface> {
    unsafe {
        let dc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let old = SelectObject(dc, bmp.into());
        Some(Surface { dc, bmp, old, bits: bits as *mut u32, w, h })
    }
}

/// Lepas surface milik window yang sudah ditutup.
pub fn forget(hwnd: HWND) {
    SURFACES.with(|c| {
        c.borrow_mut().remove(&(hwnd.0 as isize));
    });
}

/// Kirim pixmap (RGBA premultiplied) ke layered window (BGRA premultiplied).
pub fn present(hwnd: HWND, pm: &Pixmap) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    SURFACES.with(|c| {
        let mut c = c.borrow_mut();
        let key = hwnd.0 as isize;
        if !c.get(&key).is_some_and(|s| s.w == w && s.h == h) {
            c.remove(&key);
            match make_surface(w, h) {
                Some(s) => {
                    c.insert(key, s);
                }
                None => return,
            }
        }
        let s = &c[&key];
        unsafe {
            // RGBA -> BGRA: tukar byte R dan B, 4 byte sekaligus
            let n = (w * h) as usize;
            let src = pm.data().as_ptr() as *const u32;
            for i in 0..n {
                let v = src.add(i).read_unaligned();
                s.bits.add(i).write((v & 0xFF00_FF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF));
            }
            let size = SIZE { cx: w, cy: h };
            let pt = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(hwnd, None, None, Some(&size), Some(s.dc), Some(&pt), COLORREF(0), Some(&blend), ULW_ALPHA);
        }
    });
}
