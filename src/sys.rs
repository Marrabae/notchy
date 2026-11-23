//! Thread sistem: baterai, volume, jaringan; memicu "live activity" saat berubah.
//! Akses OS ada di backend per-platform (`platform::sys`), loop dan logika notifikasi dipakai bersama.
use crate::state::{Net, NetKind, Power, State, SysCmd};
use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

/// Jumlah sampel riwayat kecepatan jaringan (1 sampel / detik).
pub const NET_HIST: usize = 36;

/// Kondisi output audio default.
pub struct AudioInfo {
    pub vol: f32,
    pub muted: bool,
    /// nama perangkat output
    pub name: String,
}

pub trait Backend {
    /// Output audio default saat ini (None = tidak ada/tidak didukung).
    fn audio(&mut self) -> Option<AudioInfo>;
    fn set_volume(&mut self, v: f32);
    fn toggle_mute(&mut self);
    /// (persen, AC tersambung, detail) -- None = tanpa baterai.
    fn battery(&mut self) -> Option<(u8, bool, Power)>;
    /// Counter byte (masuk, keluar) per adapter fisik yang aktif.
    fn net_bytes(&mut self) -> Option<HashMap<u32, (u64, u64)>>;
    fn net_profile(&mut self) -> (NetKind, String);
}

pub fn spawn(state: State) -> Sender<SysCmd> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || run(state, rx, crate::platform::sys::backend()));
    tx
}

fn run(state: State, rx: Receiver<SysCmd>, mut be: impl Backend) {
    let mut first = true;
    let mut quiet_until = Instant::now(); // jangan flash volume yang diubah dari notch sendiri
    let mut last_net: Option<(Instant, HashMap<u32, (u64, u64)>)> = None;
    let mut prof_at: Option<Instant> = None;
    let mut pow_at: Option<Instant> = None;
    let due = |t: Option<Instant>, d: Duration| t.is_none_or(|t| t.elapsed() > d);
    loop {
        // perintah UI (ambil yang terbaru saja untuk volume)
        let mut vol_set = None;
        let mut mute = false;
        let mut take = |c: SysCmd| match c {
            SysCmd::Volume(v) => vol_set = Some(v),
            SysCmd::ToggleMute => mute = !mute,
        };
        match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(c) => {
                take(c);
                while let Ok(c) = rx.try_recv() {
                    take(c);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some(v) = vol_set {
            be.set_volume(v);
            quiet_until = Instant::now() + Duration::from_millis(1500);
        }
        if mute {
            be.toggle_mute();
            quiet_until = Instant::now() + Duration::from_millis(1500);
        }

        let au = be.audio();

        let pw = if due(pow_at, Duration::from_secs(2)) {
            pow_at = Some(Instant::now());
            Some(be.battery())
        } else {
            None
        };

        // kecepatan jaringan: sampel tiap ~1 detik
        let now = Instant::now();
        let mut net_rate = None;
        if last_net.as_ref().is_none_or(|(t, _)| now.duration_since(*t) >= Duration::from_millis(950)) {
            if let Some(cur) = be.net_bytes() {
                if let Some((t, prev)) = &last_net {
                    // delta per adapter yang ada di kedua sampel: adapter yang baru naik tidak bikin lonjakan palsu
                    let (mut di, mut do_) = (0u64, 0u64);
                    for (k, (i, o)) in &cur {
                        if let Some((i0, o0)) = prev.get(k) {
                            di += i.saturating_sub(*i0);
                            do_ += o.saturating_sub(*o0);
                        }
                    }
                    let dt = now.duration_since(*t).as_secs_f32().max(0.1);
                    net_rate = Some((di as f32 / dt, do_ as f32 / dt));
                }
                last_net = Some((now, cur));
            }
        }
        let prof = due(prof_at, Duration::from_secs(5)).then(|| {
            prof_at = Some(Instant::now());
            be.net_profile()
        });

        let mut s = state.lock().unwrap();
        if let Some(a) = &au {
            let (v, m) = (a.vol, a.muted);
            let quiet = Instant::now() < quiet_until;
            if !first && !quiet && (v - s.volume).abs() > 0.004 {
                s.flash(format!("Volume {}%", (v * 100.0).round() as i32));
            }
            if !first && !quiet && m != s.muted {
                s.flash(if m { "Muted".into() } else { format!("Volume {}%", (v * 100.0).round() as i32) });
            }
            s.volume = v;
            s.muted = m;
            if a.name != s.audio_out {
                if !first && !a.name.is_empty() {
                    s.flash(format!("Output \u{b7} {}", a.name));
                }
                s.audio_out = a.name.clone();
            }
        }
        if let Some(b) = pw {
            let b0 = b.as_ref().map(|(p, c, _)| (*p, *c));
            if !first {
                match (s.battery, b0) {
                    (Some((_, c0)), Some((p, c1))) if c0 != c1 => {
                        s.flash(if c1 { format!("Charging {p}%") } else { format!("On battery {p}%") })
                    }
                    (Some((p0, _)), Some((p, false))) if p != p0 && p == 20 => s.flash(format!("Battery low {p}%")),
                    _ => {}
                }
            }
            s.battery = b0;
            s.power = b.map(|(_, _, p)| p);
        }
        if let Some((d, u)) = net_rate {
            let n: &mut Net = &mut s.net;
            n.down = d;
            n.up = u;
            n.hist.push((d, u));
            if n.hist.len() > NET_HIST {
                n.hist.remove(0);
            }
        }
        if let Some((k, name)) = prof {
            s.net.kind = k;
            s.net.name = name;
        }
        drop(s);
        first = false;
    }
}
