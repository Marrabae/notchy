//! Window Windows: layered window (per-pixel alpha), always-on-top, tanpa fokus & taskbar.
#![allow(unsafe_op_in_unsafe_fn)]
use std::ffi::c_void;
use tiny_skia::Pixmap;
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

pub type WinId = HWND;

/// Tunggu frame komposisi DWM berikutnya (sinkron vsync) -> animasi rata, tanpa timer 15.6 ms.
pub fn wait_vsync() {
    unsafe {
        if windows::Win32::Graphics::Dwm::DwmFlush().is_err() {
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
    }
}

pub fn win_id(window: &winit::window::Window) -> WinId {
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
