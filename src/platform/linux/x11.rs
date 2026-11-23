//! Helper koneksi X11 (x11rb). Tiap thread/pemakai membuat koneksi sendiri supaya antrean event tidak saling curi.
use std::{cell::RefCell, collections::HashMap};
use x11rb::{
    connection::Connection,
    protocol::xproto::{ConnectionExt as _, Window},
    rust_connection::RustConnection,
};

pub struct X {
    pub conn: RustConnection,
    pub root: Window,
    atoms: RefCell<HashMap<String, u32>>,
}

pub fn connect() -> Option<X> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    Some(X { conn, root, atoms: RefCell::default() })
}

impl X {
    /// Atom berdasarkan nama (di-cache per koneksi). 0 (NONE) kalau gagal.
    pub fn atom(&self, name: &str) -> u32 {
        if let Some(a) = self.atoms.borrow().get(name) {
            return *a;
        }
        let a = self
            .conn
            .intern_atom(false, name.as_bytes())
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom)
            .unwrap_or(0);
        self.atoms.borrow_mut().insert(name.to_string(), a);
        a
    }

    /// Keycode pertama untuk sebuah keysym (None kalau tidak ada di layout aktif).
    pub fn keycode(&self, keysym: u32) -> Option<u8> {
        let setup = self.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = self.conn.get_keyboard_mapping(min, max - min + 1).ok()?.reply().ok()?;
        let per = map.keysyms_per_keycode as usize;
        map.keysyms.chunks(per).position(|syms| syms.contains(&keysym)).map(|i| min + i as u8)
    }
}

pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_ESCAPE: u32 = 0xff1b;
pub const XK_V: u32 = 0x0076;
