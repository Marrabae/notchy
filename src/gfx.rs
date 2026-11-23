use ab_glyph::{point, Font as _, FontRef, PxScale, ScaleFont};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::OnceLock};
use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pattern, Pixmap, Point,
    RadialGradient, SpreadMode, Transform,
};
pub type Rgba = [u8; 4];

struct Slot {
    path: String,
    index: u32,
    font: OnceLock<Option<FontRef<'static>>>,
}

impl Slot {
    fn get(&self) -> Option<&FontRef<'static>> {
        self.font
            .get_or_init(|| {
                // ab_glyph parse outline secara lazy -> hanya ~ukuran file di RAM; dimuat saat dibutuhkan
                let bytes: &'static [u8] = Box::leak(std::fs::read(&self.path).ok()?.into_boxed_slice());
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
    /// `main` = font utama (wajib ada), `fallback` = cadangan untuk glyph yang tidak ada (CJK, dll).
    pub fn load(main: &(String, u32), fallback: &[(String, u32)]) -> Self {
        let slots: Vec<Slot> = std::iter::once(main)
            .chain(fallback)
            .map(|(path, index)| Slot { path: path.clone(), index: *index, font: OnceLock::new() })
            .collect();
        slots[0].get().unwrap_or_else(|| panic!("font utama tidak ada/rusak: {}", main.0));
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
        self.arc_span(cx, cy, r, width, -90.0, 360.0 * frac, c);
    }

    /// Busur dari sudut `a0` (derajat; 0 = jam 3, -90 = jam 12) sepanjang `sweep` derajat searah jarum jam.
    #[allow(clippy::too_many_arguments)]
    pub fn arc_span(&mut self, cx: f32, cy: f32, r: f32, width: f32, a0: f32, sweep: f32, c: Rgba) {
        if sweep <= 0.0 {
            return;
        }
        let s = self.s;
        let n = ((48.0 * sweep / 360.0).ceil() as usize).max(2);
        let mut pb = PathBuilder::new();
        for i in 0..=n {
            let t = (a0 + sweep * i as f32 / n as f32).to_radians();
            let (x, y) = ((cx + r * t.cos()) * s, (cy + r * t.sin()) * s);
            if i == 0 {
                pb.move_to(x, y);
            } else {
                pb.line_to(x, y);
            }
        }
        if sweep >= 360.0 {
            pb.close();
        }
        if let Some(path) = pb.finish() {
            self.stroke(&path, width, c);
        }
    }

    /// Garis bersambung (stroke, ujung bulat). Titik logical.
    pub fn polyline(&mut self, pts: &[(f32, f32)], width: f32, c: Rgba) {
        let mut pb = PathBuilder::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(x * self.s, y * self.s);
            } else {
                pb.line_to(x * self.s, y * self.s);
            }
        }
        if let Some(path) = pb.finish() {
            self.stroke(&path, width, c);
        }
    }

    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, c: Rgba) {
        self.polyline(&[(x1, y1), (x2, y2)], width, c);
    }

    /// Garis tepi rounded rect (stroke tipis).
    pub fn rrect_stroke(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: Rgba) {
        let k = width / 2.0;
        if let Some(path) = self.rrect_path(x + k, y + k, w - width, h - width, r - k) {
            self.stroke(&path, width, c);
        }
    }

    /// Tepi rounded rect putus-putus (drop zone). `dash`/`gap` dalam logical px.
    #[allow(clippy::too_many_arguments)]
    pub fn rrect_dashed(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, dash: f32, gap: f32, c: Rgba) {
        let k = width / 2.0;
        let Some(path) = self.rrect_path(x + k, y + k, w - width, h - width, r - k) else { return };
        let mut p = Paint::default();
        p.set_color_rgba8(c[0], c[1], c[2], c[3]);
        p.anti_alias = true;
        let st = tiny_skia::Stroke {
            width: width * self.s,
            dash: tiny_skia::StrokeDash::new(vec![dash * self.s, gap * self.s], 0.0),
            ..Default::default()
        };
        self.pm.stroke_path(&path, &p, &st, Transform::identity(), None);
    }

    fn stroke(&mut self, path: &Path, width: f32, c: Rgba) {
        let mut p = Paint::default();
        p.set_color_rgba8(c[0], c[1], c[2], c[3]);
        p.anti_alias = true;
        let st = tiny_skia::Stroke {
            width: width * self.s,
            line_cap: tiny_skia::LineCap::Round,
            line_join: tiny_skia::LineJoin::Round,
            ..Default::default()
        };
        self.pm.stroke_path(path, &p, &st, Transform::identity(), None);
    }

    /// Rounded rect dengan gradient horizontal (kiri -> kanan).
    #[allow(clippy::too_many_arguments)]
    pub fn rrect_hgrad(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c0: Rgba, c1: Rgba) {
        let Some(path) = self.rrect_path(x, y, w, h, r) else { return };
        let st = vec![
            GradientStop::new(0.0, Color::from_rgba8(c0[0], c0[1], c0[2], c0[3])),
            GradientStop::new(1.0, Color::from_rgba8(c1[0], c1[1], c1[2], c1[3])),
        ];
        let Some(shader) = LinearGradient::new(
            Point::from_xy(x * self.s, 0.0),
            Point::from_xy((x + w) * self.s, 0.0),
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

    /// Cahaya radial berwarna `c` (memudar ke transparan) berpusat di (cx, cy) radius `rad`, dipotong ke rounded rect.
    #[allow(clippy::too_many_arguments)]
    pub fn rrect_glow(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, cx: f32, cy: f32, rad: f32, c: Rgba) {
        let Some(path) = self.rrect_path(x, y, w, h, r) else { return };
        let ctr = Point::from_xy(cx * self.s, cy * self.s);
        let st = vec![
            GradientStop::new(0.0, Color::from_rgba8(c[0], c[1], c[2], c[3])),
            GradientStop::new(1.0, Color::from_rgba8(c[0], c[1], c[2], 0)),
        ];
        let Some(shader) = RadialGradient::new(ctr, 0.0, ctr, rad * self.s, st, SpreadMode::Pad, Transform::identity())
        else {
            return;
        };
        let mut p = Paint::default();
        p.shader = shader;
        p.anti_alias = true;
        self.pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
    }

    /// Bayangan lembut di bawah rounded rect: lapisan rrect membesar bertahap dengan alpha kecil (murah, tanpa blur).
    #[allow(clippy::too_many_arguments)]
    pub fn shadow(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, spread: f32, dy: f32, alpha: f32) {
        const N: usize = 7;
        let per = (alpha / N as f32).clamp(0.0, 255.0) as u8;
        if per == 0 {
            return;
        }
        for i in 0..N {
            let g = spread * (N - i) as f32 / N as f32;
            self.rrect(x - g, y + dy - g * 0.5, w + 2.0 * g, h + 2.0 * g * 0.7, r + g, [0, 0, 0, per]);
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
