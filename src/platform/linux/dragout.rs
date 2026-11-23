//! Drag file dari shelf keluar ke file manager / app lain: sisi sumber protokol XDND (v5).
//! Dipanggil saat tombol kiri masih ditekan (blocking sampai drop/cancel), jadi posisi pointer dan tombol
//! dibaca dengan polling `QueryPointer` -- tidak perlu grab (yang sudah dipegang window winit).
use super::{
    window::own_windows,
    x11::{self, X, XK_ESCAPE},
};
use std::{
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{
            AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, KeyButMask, PropMode,
            SelectionNotifyEvent, SelectionRequestEvent, WindowClass, SELECTION_NOTIFY_EVENT,
        },
        Event,
    },
    wrapper::ConnectionExt as _,
    COPY_DEPTH_FROM_PARENT,
};

const VERSION: u32 = 5;

struct Atoms {
    selection: u32,
    aware: u32,
    enter: u32,
    position: u32,
    status: u32,
    leave: u32,
    drop: u32,
    finished: u32,
    copy: u32,
    mv: u32,
    uri_list: u32,
    targets: u32,
}

impl Atoms {
    fn new(x: &X) -> Self {
        Atoms {
            selection: x.atom("XdndSelection"),
            aware: x.atom("XdndAware"),
            enter: x.atom("XdndEnter"),
            position: x.atom("XdndPosition"),
            status: x.atom("XdndStatus"),
            leave: x.atom("XdndLeave"),
            drop: x.atom("XdndDrop"),
            finished: x.atom("XdndFinished"),
            copy: x.atom("XdndActionCopy"),
            mv: x.atom("XdndActionMove"),
            uri_list: x.atom("text/uri-list"),
            targets: x.atom("TARGETS"),
        }
    }
}

/// `file:///...` dengan percent-encoding, satu per baris (CRLF) sesuai RFC 2483.
fn uri_list(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = String::new();
    for p in paths {
        let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        out.push_str("file://");
        for b in abs.to_string_lossy().bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out.push_str("\r\n");
    }
    out.into_bytes()
}

/// Turun dari root mencari window (di bawah pointer) yang mengiklankan XdndAware; lewati window notchy sendiri.
fn find_target(x: &X, a: &Atoms, own: &[u32]) -> Option<(u32, u32)> {
    let mut w = x.root;
    for _ in 0..16 {
        let child = x.conn.query_pointer(w).ok()?.reply().ok()?.child;
        if child == x11rb::NONE {
            return None;
        }
        w = child;
        let prop = x.conn.get_property(false, w, a.aware, AtomEnum::ATOM, 0, 1).ok()?.reply().ok()?;
        if let Some(v) = prop.value32().and_then(|mut it| it.next()) {
            return (!own.contains(&w)).then_some((w, v.min(VERSION)));
        }
    }
    None
}

fn send(x: &X, to: u32, ty: u32, data: [u32; 5]) {
    let _ = x.conn.send_event(false, to, EventMask::NO_EVENT, ClientMessageEvent::new(32, to, ty, data));
}

/// Layani permintaan data selection dari target drop.
fn serve(x: &X, a: &Atoms, e: &SelectionRequestEvent, data: &[u8]) {
    let prop = if e.property == x11rb::NONE { e.target } else { e.property };
    let ok = if e.target == a.targets {
        x.conn.change_property32(PropMode::REPLACE, e.requestor, prop, AtomEnum::ATOM, &[a.targets, a.uri_list]).is_ok()
    } else if e.target == a.uri_list {
        x.conn.change_property8(PropMode::REPLACE, e.requestor, prop, a.uri_list, data).is_ok()
    } else {
        false
    };
    let notify = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: e.time,
        requestor: e.requestor,
        selection: e.selection,
        target: e.target,
        property: if ok { prop } else { x11rb::NONE },
    };
    let _ = x.conn.send_event(false, e.requestor, EventMask::NO_EVENT, notify);
    let _ = x.conn.flush();
}

/// Mulai drag file (blocking sampai drop/cancel). Some(()) kalau target menerima drop.
pub fn drag_files(paths: &[PathBuf]) -> Option<()> {
    let paths: Vec<PathBuf> = paths.iter().filter(|p| Path::new(p).exists()).cloned().collect();
    if paths.is_empty() {
        return None;
    }
    let x = x11::connect()?;
    let a = Atoms::new(&x);
    let data = uri_list(&paths);
    let own = own_windows();

    // window sumber (tak terlihat) yang memiliki selection XdndSelection
    let src = x.conn.generate_id().ok()?;
    x.conn
        .create_window(COPY_DEPTH_FROM_PARENT, src, x.root, -10, -10, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())
        .ok()?;
    x.conn.set_selection_owner(src, a.selection, x11rb::CURRENT_TIME).ok()?;
    x.conn.flush().ok()?;
    let esc = x.keycode(XK_ESCAPE);

    let result = run(&x, &a, src, &data, &own, esc);
    let _ = x.conn.destroy_window(src);
    let _ = x.conn.flush();
    result
}

struct Target {
    win: u32,
    version: u32,
    accepted: bool,
    /// menunggu XdndStatus untuk Position terakhir
    waiting: Option<Instant>,
    last_pos: (i16, i16),
}

fn run(x: &X, a: &Atoms, src: u32, data: &[u8], own: &[u32], esc: Option<u8>) -> Option<()> {
    let mut tgt: Option<Target> = None;
    let mut finished = false;
    loop {
        while let Ok(Some(ev)) = x.conn.poll_for_event() {
            match ev {
                Event::SelectionRequest(e) if e.selection == a.selection => serve(x, a, &e, data),
                Event::ClientMessage(e) if e.type_ == a.status => {
                    if let Some(t) = tgt.as_mut().filter(|t| t.win == e.data.as_data32()[0]) {
                        t.accepted = e.data.as_data32()[1] & 1 != 0;
                        t.waiting = None;
                    }
                }
                _ => {}
            }
        }
        let q = x.conn.query_pointer(x.root).ok()?.reply().ok()?;
        let cancelled = esc.is_some_and(|k| {
            x.conn
                .query_keymap()
                .ok()
                .and_then(|c| c.reply().ok())
                .is_some_and(|m| m.keys[(k / 8) as usize] & (1 << (k % 8)) != 0)
        });
        if cancelled {
            if let Some(t) = &tgt {
                send(x, t.win, a.leave, [src, 0, 0, 0, 0]);
            }
            let _ = x.conn.flush();
            return None;
        }
        if !q.mask.contains(KeyButMask::BUTTON1) {
            break;
        }

        // target berganti?
        let found = find_target(x, a, own);
        if tgt.as_ref().map(|t| t.win) != found.map(|f| f.0) {
            if let Some(t) = tgt.take() {
                send(x, t.win, a.leave, [src, 0, 0, 0, 0]);
            }
            if let Some((win, version)) = found {
                send(x, win, a.enter, [src, version << 24, a.uri_list, 0, 0]);
                tgt = Some(Target { win, version, accepted: false, waiting: None, last_pos: (-1, -1) });
            }
        }
        if let Some(t) = tgt.as_mut() {
            let pos = (q.root_x, q.root_y);
            let stale = t.waiting.is_some_and(|w| w.elapsed() > Duration::from_millis(150));
            if (pos != t.last_pos && (t.waiting.is_none() || stale)) || stale {
                // Ctrl = salin, selain itu pindahkan (shelf = "drag keluar untuk memindahkan")
                let act = if q.mask.contains(KeyButMask::CONTROL) { a.copy } else { a.mv };
                send(x, t.win, a.position, [src, 0, ((pos.0 as u32) << 16) | (pos.1 as u32 & 0xFFFF), 0, act]);
                t.last_pos = pos;
                t.waiting = Some(Instant::now());
            }
        }
        let _ = x.conn.flush();
        thread::sleep(Duration::from_millis(8));
    }

    // tombol dilepas; kalau XdndStatus untuk Position terakhir belum datang, tunggu sebentar sebelum memutuskan
    let mut t = tgt?;
    let until = Instant::now() + Duration::from_millis(250);
    while t.waiting.is_some() && Instant::now() < until {
        while let Ok(Some(ev)) = x.conn.poll_for_event() {
            match ev {
                Event::SelectionRequest(e) if e.selection == a.selection => serve(x, a, &e, data),
                Event::ClientMessage(e) if e.type_ == a.status && e.data.as_data32()[0] == t.win => {
                    t.accepted = e.data.as_data32()[1] & 1 != 0;
                    t.waiting = None;
                }
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    if !t.accepted {
        send(x, t.win, a.leave, [src, 0, 0, 0, 0]);
        let _ = x.conn.flush();
        return None;
    }
    send(x, t.win, a.drop, [src, 0, 0, 0, 0]);
    let _ = x.conn.flush();
    // layani permintaan data sampai target selesai (XdndFinished), atau jeda wajar untuk target lama
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut served_at: Option<Instant> = None;
    while Instant::now() < deadline && !finished {
        while let Ok(Some(ev)) = x.conn.poll_for_event() {
            match ev {
                Event::SelectionRequest(e) if e.selection == a.selection => {
                    serve(x, a, &e, data);
                    served_at = Some(Instant::now());
                }
                Event::ClientMessage(e) if e.type_ == a.finished => finished = true,
                _ => {}
            }
        }
        // XdndFinished baru ada di protokol v2+; versi lama: cukup tunggu sebentar setelah data terkirim
        if t.version < 2 && served_at.is_some_and(|s| s.elapsed() > Duration::from_millis(400)) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    Some(())
}
