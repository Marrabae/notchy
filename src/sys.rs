use crate::state::{Net, NetKind, Power, State, SysCmd};
use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};
use windows::{
    Networking::Connectivity::NetworkInformation,
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator},
        NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2},
        System::{
            Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ},
            Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
            Power::{CallNtPowerInformation, GetSystemPowerStatus, SystemBatteryState, SYSTEM_BATTERY_STATE, SYSTEM_POWER_STATUS},
        },
    },
};

/// Jumlah sampel riwayat kecepatan jaringan (1 sampel / detik).
pub const NET_HIST: usize = 36;

struct Audio {
    vol: IAudioEndpointVolume,
    name: String,
}

/// PKEY_Device_FriendlyName
const PKEY_FRIENDLY: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

fn device_name(dev: &IMMDevice) -> Option<String> {
    unsafe {
        let store = dev.OpenPropertyStore(STGM_READ).ok()?;
        let mut pv = store.GetValue(&PKEY_FRIENDLY).ok()?;
        let s = PropVariantToStringAlloc(&pv).ok();
        let _ = PropVariantClear(&mut pv);
        let s = s?;
        let out = s.to_string().ok();
        CoTaskMemFree(Some(s.0 as _));
        out
    }
}

fn audio() -> Option<Audio> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
        let name = device_name(&dev).unwrap_or_default();
        Some(Audio { vol: dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?, name })
    }
}

fn battery() -> Option<(u8, bool, Power)> {
    let mut p = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut p).ok()? };
    if p.BatteryLifePercent == 255 || p.BatteryFlag & 128 != 0 {
        return None; // desktop tanpa baterai
    }
    let mut b = SYSTEM_BATTERY_STATE::default();
    let ok = unsafe {
        CallNtPowerInformation(
            SystemBatteryState,
            None,
            0,
            Some(&mut b as *mut _ as _),
            std::mem::size_of::<SYSTEM_BATTERY_STATE>() as u32,
        )
    }
    .is_ok();
    let ac = p.ACLineStatus == 1;
    let mut pw = Power { saver: p.SystemStatusFlag & 1 != 0, charging: p.BatteryFlag & 8 != 0, ..Default::default() };
    if ok {
        // Rate: signed (negatif saat discharge); 0x80000000 = BATTERY_UNKNOWN_RATE
        pw.rate_mw = if b.Rate == 0x8000_0000 { 0 } else { b.Rate as i32 };
        pw.remain_mwh = b.RemainingCapacity;
        pw.full_mwh = b.MaxCapacity;
        pw.charging = b.Charging;
    }
    pw.secs = if !ac {
        (p.BatteryLifeTime != u32::MAX && p.BatteryLifeTime > 0).then_some(p.BatteryLifeTime).or_else(|| {
            (pw.rate_mw < 0).then(|| (pw.remain_mwh as f64 * 3600.0 / pw.rate_mw.unsigned_abs() as f64) as u32)
        })
    } else if pw.charging && pw.rate_mw > 0 && pw.full_mwh > pw.remain_mwh {
        Some(((pw.full_mwh - pw.remain_mwh) as f64 * 3600.0 / pw.rate_mw as f64) as u32)
    } else {
        None
    };
    Some((p.BatteryLifePercent, ac, pw))
}

/// Counter byte (masuk, keluar) per adapter fisik yang aktif, key = InterfaceIndex.
fn net_bytes() -> Option<HashMap<u32, (u64, u64)>> {
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t).is_err() || t.is_null() {
            return None;
        }
        let n = (*t).NumEntries as usize;
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), n);
        let mut out = HashMap::new();
        for r in rows {
            // bit0 = HardwareInterface; OperStatus 1 = up; Type 24 = loopback
            if r.InterfaceAndOperStatusFlags._bitfield & 1 != 0 && r.OperStatus.0 == 1 && r.Type != 24 {
                out.insert(r.InterfaceIndex, (r.InOctets, r.OutOctets));
            }
        }
        FreeMibTable(t as _);
        Some(out)
    }
}

fn net_profile() -> (NetKind, String) {
    let Ok(p) = NetworkInformation::GetInternetConnectionProfile() else { return (NetKind::Offline, String::new()) };
    let name = p.ProfileName().map(|s| s.to_string()).unwrap_or_default();
    let kind = if p.IsWlanConnectionProfile().unwrap_or(false) {
        NetKind::Wifi
    } else if p.IsWwanConnectionProfile().unwrap_or(false) {
        NetKind::Cellular
    } else {
        NetKind::Ethernet
    };
    (kind, name)
}

/// Thread kecil: baterai, volume, jaringan; memicu "live activity" saat berubah.
pub fn spawn(state: State) -> Sender<SysCmd> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || run(state, rx));
    tx
}

fn run(state: State, rx: Receiver<SysCmd>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut au = audio();
    let mut au_at = Instant::now();
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
        if let Some(a) = &au {
            unsafe {
                if let Some(v) = vol_set {
                    let _ = a.vol.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null());
                    if a.vol.GetMute().is_ok_and(|m| m.as_bool()) {
                        let _ = a.vol.SetMute(false, std::ptr::null());
                    }
                    quiet_until = Instant::now() + Duration::from_millis(1500);
                }
                if mute {
                    let m = a.vol.GetMute().map(|m| m.as_bool()).unwrap_or(false);
                    let _ = a.vol.SetMute(!m, std::ptr::null());
                    quiet_until = Instant::now() + Duration::from_millis(1500);
                }
            }
        }

        // device default bisa berganti (mis. headset BT tersambung) -> cek ulang berkala
        if au.is_none() || au_at.elapsed() > Duration::from_secs(3) {
            let fresh = audio();
            if fresh.is_some() || au.is_none() {
                au = fresh;
            }
            au_at = Instant::now();
        }
        let v = au.as_ref().and_then(|a| unsafe { a.vol.GetMasterVolumeLevelScalar().ok() });
        let m = au.as_ref().and_then(|a| unsafe { a.vol.GetMute().ok() }).map(|m| m.as_bool());
        if v.is_none() {
            au = None;
        }

        let pw = if due(pow_at, Duration::from_secs(2)) {
            pow_at = Some(Instant::now());
            Some(battery())
        } else {
            None
        };

        // kecepatan jaringan: sampel tiap ~1 detik
        let now = Instant::now();
        let mut net_rate = None;
        if last_net.as_ref().is_none_or(|(t, _)| now.duration_since(*t) >= Duration::from_millis(950)) {
            if let Some(cur) = net_bytes() {
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
            net_profile()
        });

        let mut s = state.lock().unwrap();
        if let Some(v) = v {
            let quiet = Instant::now() < quiet_until;
            if !first && !quiet && (v - s.volume).abs() > 0.004 {
                s.flash(format!("Volume {}%", (v * 100.0).round() as i32));
            }
            if !first && !quiet && m.is_some_and(|m| m != s.muted) {
                s.flash(if m == Some(true) { "Muted".into() } else { format!("Volume {}%", (v * 100.0).round() as i32) });
            }
            s.volume = v;
            s.muted = m.unwrap_or(false);
        }
        if let Some(a) = &au {
            if a.name != s.audio_out {
                if !first && !a.name.is_empty() {
                    s.flash(format!("Output · {}", a.name));
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
