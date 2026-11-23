//! Backend sistem Linux: volume (pactl, event lewat `pactl subscribe`), baterai (sysfs), jaringan (/proc, sysfs, nmcli).
use crate::{
    state::{NetKind, Power},
    sys::{AudioInfo, Backend},
};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub struct Sys {
    /// diset thread `pactl subscribe` saat ada perubahan sink/server
    dirty: Arc<AtomicBool>,
    /// `pactl subscribe` berjalan -> tidak perlu polling berkala (di-clear thread-nya saat pactl berakhir)
    subscribed: Arc<AtomicBool>,
    audio: Option<AudioInfo>,
    audio_at: Instant,
    saver: (Instant, bool),
}

/// Dipanggil dari thread sistem.
pub fn backend() -> Sys {
    let dirty = Arc::new(AtomicBool::new(true));
    let subscribed = Arc::new(AtomicBool::new(false));
    subscribe(dirty.clone(), subscribed.clone());
    Sys {
        dirty,
        subscribed,
        audio: None,
        audio_at: Instant::now(),
        saver: (Instant::now() - Duration::from_secs(60), false),
    }
}

/// Dengarkan perubahan audio dari PulseAudio/PipeWire-pulse; thread berhenti sendiri kalau pactl berakhir.
/// Kalau server audio restart, `pactl subscribe` berakhir: coba lagi berkala (sementara itu polling 1.5 detik).
fn subscribe(dirty: Arc<AtomicBool>, active: Arc<AtomicBool>) {
    thread::spawn(move || loop {
        let spawned = Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null()).spawn();
        if let Ok(mut child) = spawned {
            if let Some(out) = child.stdout.take() {
                active.store(true, Ordering::Relaxed);
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if line.contains(" sink ") || line.contains("server") {
                        dirty.store(true, Ordering::Relaxed);
                    }
                }
            }
            active.store(false, Ordering::Relaxed);
            dirty.store(true, Ordering::Relaxed);
            let _ = child.wait();
        } else {
            return; // pactl tidak terpasang
        }
        thread::sleep(Duration::from_secs(5));
    });
}

fn pactl(args: &[&str]) -> Option<String> {
    let out = Command::new("pactl").args(args).env("LC_ALL", "C").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Sink default: JSON (PulseAudio >= 16 / PipeWire), kalau tidak ada fallback ke keluaran teks.
fn read_audio() -> Option<AudioInfo> {
    read_audio_json().or_else(read_audio_text)
}

/// Rata-rata semua token `NN%` pada keluaran `pactl get-sink-volume`.
fn read_audio_text() -> Option<AudioInfo> {
    let name = pactl(&["get-default-sink"])?.trim().to_string();
    let vol = pactl(&["get-sink-volume", "@DEFAULT_SINK@"])?;
    let pcts: Vec<f32> = vol.split_whitespace().filter_map(|t| t.strip_suffix('%')?.parse().ok()).collect();
    if pcts.is_empty() {
        return None;
    }
    let muted = pactl(&["get-sink-mute", "@DEFAULT_SINK@"]).is_some_and(|m| m.contains("yes"));
    Some(AudioInfo { vol: (pcts.iter().sum::<f32>() / pcts.len() as f32 / 100.0).clamp(0.0, 1.5), muted, name })
}

/// Sink default: volume (rata-rata kanal, 0..1), mute, deskripsi.
fn read_audio_json() -> Option<AudioInfo> {
    let default = pactl(&["get-default-sink"])?.trim().to_string();
    if default.is_empty() {
        return None;
    }
    let json = pactl(&["-f", "json", "list", "sinks"])?;
    let v: serde_json::Value = serde_json::from_str(&json).ok()?;
    let sink = v.as_array()?.iter().find(|s| s["name"].as_str() == Some(default.as_str()))?;
    let pcts: Vec<f32> = sink["volume"]
        .as_object()?
        .values()
        .filter_map(|c| c["value_percent"].as_str()?.trim_end_matches('%').parse::<f32>().ok())
        .collect();
    if pcts.is_empty() {
        return None;
    }
    Some(AudioInfo {
        vol: (pcts.iter().sum::<f32>() / pcts.len() as f32 / 100.0).clamp(0.0, 1.5),
        muted: sink["mute"].as_bool().unwrap_or(false),
        name: sink["description"].as_str().unwrap_or(&default).to_string(),
    })
}

impl Backend for Sys {
    fn audio(&mut self) -> Option<AudioInfo> {
        let subscribed = self.subscribed.load(Ordering::Relaxed);
        let stale = self.audio_at.elapsed() > if subscribed { Duration::from_secs(30) } else { Duration::from_millis(1500) };
        if self.dirty.swap(false, Ordering::Relaxed) || stale || self.audio.is_none() && self.audio_at.elapsed() > Duration::from_secs(3) {
            self.audio = read_audio();
            self.audio_at = Instant::now();
        }
        self.audio.as_ref().map(|a| AudioInfo { vol: a.vol, muted: a.muted, name: a.name.clone() })
    }

    fn set_volume(&mut self, v: f32) {
        let pct = format!("{}%", (v.clamp(0.0, 1.0) * 100.0).round() as i32);
        let _ = pactl(&["set-sink-volume", "@DEFAULT_SINK@", &pct]);
        if self.audio.as_ref().is_some_and(|a| a.muted) {
            let _ = pactl(&["set-sink-mute", "@DEFAULT_SINK@", "0"]);
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn toggle_mute(&mut self) {
        let _ = pactl(&["set-sink-mute", "@DEFAULT_SINK@", "toggle"]);
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn battery(&mut self) -> Option<(u8, bool, Power)> {
        // mode hemat daya (power-profiles-daemon): proses kecil, cukup dicek tiap 30 detik
        if self.saver.0.elapsed() > Duration::from_secs(30) {
            let out = Command::new("powerprofilesctl").arg("get").stdin(Stdio::null()).stderr(Stdio::null()).output().ok();
            self.saver = (Instant::now(), out.is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "power-saver"));
        }
        battery(self.saver.1)
    }

    fn net_bytes(&mut self) -> Option<HashMap<u32, (u64, u64)>> {
        net_bytes()
    }

    fn net_profile(&mut self) -> (NetKind, String) {
        net_profile()
    }
}

fn read(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn read_num(p: impl AsRef<Path>) -> Option<i64> {
    read(p)?.parse().ok()
}

/// Baterai utama di /sys/class/power_supply (satuan µW / µWh / µA / µV / µAh).
fn battery(saver: bool) -> Option<(u8, bool, Power)> {
    let root = Path::new("/sys/class/power_supply");
    let mut bats: Vec<std::path::PathBuf> = Vec::new();
    let mut ac = false;
    for e in fs::read_dir(root).ok()?.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        match read(p.join("type")).as_deref() {
            // baterai sistem; abaikan baterai perangkat (mouse/keyboard nirkabel: scope=Device atau hid-*)
            Some("Battery") if read(p.join("scope")).as_deref() != Some("Device") && !name.starts_with("hid-") => bats.push(p),
            Some("Mains") | Some("USB") => ac |= read_num(p.join("online")) == Some(1),
            _ => {}
        }
    }
    // urutan read_dir tidak terjamin: utamakan BAT*, lalu urut nama
    bats.sort_by_key(|p| {
        let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        (!n.starts_with("BAT"), n)
    });
    let b = bats.into_iter().next()?;
    let pct = read_num(b.join("capacity"))?.clamp(0, 100) as u8;
    let status = read(b.join("status")).unwrap_or_default();
    let charging = status == "Charging";
    let volt = read_num(b.join("voltage_now")).unwrap_or(0); // µV
    let (remain_uwh, full_uwh) = match (read_num(b.join("energy_now")), read_num(b.join("energy_full"))) {
        (Some(n), Some(f)) => (n, f),
        _ => (
            read_num(b.join("charge_now")).unwrap_or(0) * volt / 1_000_000,
            read_num(b.join("charge_full")).unwrap_or(0) * volt / 1_000_000,
        ),
    };
    let rate_uw = read_num(b.join("power_now")).unwrap_or_else(|| read_num(b.join("current_now")).unwrap_or(0) * volt / 1_000_000);
    let mut pw = Power { saver, charging, ..Default::default() };
    pw.remain_mwh = (remain_uwh / 1000).max(0) as u32;
    pw.full_mwh = (full_uwh / 1000).max(0) as u32;
    // sysfs memberi daya absolut; beri tanda: mengisi positif, terpakai negatif
    let mw = (rate_uw.abs() / 1000) as i32;
    pw.rate_mw = if charging {
        mw
    } else if status == "Discharging" {
        -mw
    } else {
        0
    };
    pw.secs = if status == "Discharging" && pw.rate_mw < 0 {
        Some((pw.remain_mwh as f64 * 3600.0 / pw.rate_mw.unsigned_abs() as f64) as u32)
    } else if charging && pw.rate_mw > 0 && pw.full_mwh > pw.remain_mwh {
        Some(((pw.full_mwh - pw.remain_mwh) as f64 * 3600.0 / pw.rate_mw as f64) as u32)
    } else {
        None
    };
    Some((pct, ac || charging || status == "Full", pw))
}

/// Counter byte (masuk, keluar) per adapter fisik yang aktif (punya `device` di sysfs; bukan lo/docker/veth/vpn).
fn net_bytes() -> Option<HashMap<u32, (u64, u64)>> {
    let dev = fs::read_to_string("/proc/net/dev").ok()?;
    let mut out = HashMap::new();
    for line in dev.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let name = name.trim();
        let sys = Path::new("/sys/class/net").join(name);
        if !sys.join("device").exists() || read(sys.join("operstate")).as_deref() == Some("down") {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|n| n.parse().ok()).collect();
        if f.len() < 9 {
            continue;
        }
        if let Some(idx) = read_num(sys.join("ifindex")) {
            out.insert(idx as u32, (f[0], f[8]));
        }
    }
    Some(out)
}

/// Interface rute default dari /proc/net/route.
fn default_iface() -> Option<String> {
    let t = fs::read_to_string("/proc/net/route").ok()?;
    t.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        // Destination 00000000 + flag UP (0x1)
        (f.len() > 3 && f[1] == "00000000" && u32::from_str_radix(f[3], 16).ok()? & 1 != 0).then(|| f[0].to_string())
    })
}

fn net_profile() -> (NetKind, String) {
    let Some(iface) = default_iface() else { return (NetKind::Offline, String::new()) };
    let sys = Path::new("/sys/class/net").join(&iface);
    let nmcli = |args: &[&str]| {
        Command::new("nmcli")
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let kind = if sys.join("wireless").exists() || sys.join("phy80211").exists() {
        NetKind::Wifi
    } else if iface.starts_with("ww") {
        NetKind::Cellular
    } else {
        NetKind::Ethernet
    };
    // nama koneksi: SSID untuk Wi-Fi (nmcli, atau iwgetid), kalau tidak ada pakai nama interface
    let name = nmcli(&["-t", "-g", "GENERAL.CONNECTION", "device", "show", &iface])
        .filter(|n| n != "--")
        .or_else(|| {
            (kind == NetKind::Wifi)
                .then(|| {
                    Command::new("iwgetid")
                        .arg("-r")
                        .stderr(Stdio::null())
                        .output()
                        .ok()
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .flatten()
        })
        .unwrap_or(iface);
    (kind, name)
}
