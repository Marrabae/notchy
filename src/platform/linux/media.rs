//! Media Linux: MPRIS lewat D-Bus session bus (Spotify, mpv, Firefox/Chromium, VLC, dll) + kontrol.
use crate::{
    media::{finish_art, Art, Track, Tracker, ART},
    state::{Cmd, State},
};
use std::{collections::HashMap, io::Read, sync::mpsc::Receiver, thread, time::Duration};
use zbus::{
    blocking::{connection::Builder, Connection},
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue},
};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PROPS: &str = "org.freedesktop.DBus.Properties";

type Meta = HashMap<String, OwnedValue>;

/// Thread polling MPRIS + terima perintah kontrol.
pub fn spawn(state: State, rx: Receiver<Cmd>) {
    thread::spawn(move || {
        let mut tracker = Tracker::default();
        let mut conn: Option<Connection> = None;
        let mut chosen: Option<String> = None;
        loop {
            if conn.is_none() {
                conn = connect();
            }
            let ok = conn.as_ref().is_some_and(|c| poll_once(c, &state, &rx, &mut tracker, &mut chosen).is_some());
            if !ok {
                // tidak ada player / bus putus: kosongkan; perintah yang menumpuk dibuang
                while rx.try_recv().is_ok() {}
                tracker.clear(&state);
                if conn.as_ref().is_some_and(|c| c.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "GetId", &()).is_err()) {
                    conn = None;
                }
            }
            thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Session bus dengan batas waktu panggilan, supaya player yang hang tidak membekukan thread ini.
fn connect() -> Option<Connection> {
    Builder::session().ok()?.method_timeout(Duration::from_secs(2)).build().ok()
}

fn call<B: serde::Serialize + zbus::zvariant::DynamicType>(c: &Connection, dest: &str, path: &str, iface: &str, method: &str, body: &B) -> Option<zbus::Message> {
    c.call_method(Some(dest), path, Some(iface), method, body).ok()
}

fn list_players(c: &Connection) -> Vec<String> {
    let Some(m) = call(c, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListNames", &()) else {
        return Vec::new();
    };
    let names: Vec<String> = m.body().deserialize().unwrap_or_default();
    names.into_iter().filter(|n| n.starts_with(PREFIX)).collect()
}

/// Semua properti interface Player dalam satu panggilan (Position ikut, dan selalu segar).
fn player_props(c: &Connection, dest: &str) -> Option<HashMap<String, OwnedValue>> {
    let m = call(c, dest, PATH, PROPS, "GetAll", &(PLAYER,))?;
    m.body().deserialize().ok()
}

fn status(p: &HashMap<String, OwnedValue>) -> String {
    p.get("PlaybackStatus").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default()
}

/// Pilih player: yang sedang Playing, kalau tidak ada tetap pakai yang terakhir dipilih, kalau tidak ya yang pertama.
fn pick(c: &Connection, chosen: &mut Option<String>) -> Option<(String, HashMap<String, OwnedValue>)> {
    let names = list_players(c);
    let mut all: Vec<(String, HashMap<String, OwnedValue>)> =
        names.into_iter().filter_map(|n| player_props(c, &n).map(|p| (n, p))).collect();
    if all.is_empty() {
        *chosen = None;
        return None;
    }
    let i = all
        .iter()
        .position(|(_, p)| status(p) == "Playing")
        .or_else(|| chosen.as_ref().and_then(|ch| all.iter().position(|(n, _)| n == ch)))
        .unwrap_or(0);
    let (name, props) = all.swap_remove(i);
    *chosen = Some(name.clone());
    Some((name, props))
}

fn poll_once(c: &Connection, state: &State, rx: &Receiver<Cmd>, tracker: &mut Tracker, chosen: &mut Option<String>) -> Option<()> {
    let (name, mut props) = pick(c, chosen)?;

    // perintah UI -> player terpilih
    let mut sent = false;
    while let Ok(cmd) = rx.try_recv() {
        sent = true;
        match cmd {
            Cmd::PlayPause => drop(call(c, &name, PATH, PLAYER, "PlayPause", &())),
            Cmd::Next => drop(call(c, &name, PATH, PLAYER, "Next", &())),
            Cmd::Prev => drop(call(c, &name, PATH, PLAYER, "Previous", &())),
            Cmd::Seek(ms) => seek(c, &name, &props, ms),
        }
    }
    if sent {
        props = player_props(c, &name)?;
    }

    let meta: Meta = props.get("Metadata").and_then(|v| Meta::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
    let get_str = |k: &str| meta.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok());
    let title = get_str("xesam:title").unwrap_or_default();
    let artist = meta
        .get("xesam:artist")
        .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
        .map(|a| a.join(", "))
        .or_else(|| get_str("xesam:artist"))
        .unwrap_or_default();
    if title.is_empty() && artist.is_empty() {
        return None; // player ada tapi belum memuat apa pun
    }
    let micros = |v: Option<&OwnedValue>| -> i64 {
        v.and_then(|v| {
            let v = v.try_clone().ok()?;
            i64::try_from(v.try_clone().ok()?).ok().or_else(|| u64::try_from(v).ok().map(|n| n as i64))
        })
        .unwrap_or(0)
    };
    let dur_ms = micros(meta.get("mpris:length")) / 1000;
    let pos_ms = micros(props.get("Position")) / 1000;
    let playing = status(&props) == "Playing";
    let art_url = get_str("mpris:artUrl");

    tracker.publish(state, &Track { title, artist, playing, pos_ms, dur_ms }, || art_url.as_deref().and_then(load_art));
    Some(())
}

/// Lompat ke posisi absolut: SetPosition(trackid, µs); kalau player tidak punya trackid, pakai Seek relatif.
fn seek(c: &Connection, name: &str, props: &HashMap<String, OwnedValue>, ms: i64) {
    let target = ms.max(0) * 1000;
    let track: Option<OwnedObjectPath> = props
        .get("Metadata")
        .and_then(|v| Meta::try_from(v.try_clone().ok()?).ok())
        .and_then(|m| m.get("mpris:trackid").and_then(|v| OwnedObjectPath::try_from(v.try_clone().ok()?).ok()));
    match track {
        Some(t) if t.as_str() != "/org/mpris/MediaPlayer2/TrackList/NoTrack" => {
            let path: ObjectPath = t.as_ref().clone();
            let _ = call(c, name, PATH, PLAYER, "SetPosition", &(path, target));
        }
        _ => {
            let cur = props
                .get("Position")
                .and_then(|v| i64::try_from(v.try_clone().ok()?).ok())
                .unwrap_or(0);
            let _ = call(c, name, PATH, PLAYER, "Seek", &(target - cur,));
        }
    }
}

/// Muat artwork dari mpris:artUrl (file:// atau http(s)://), crop tengah jadi kotak 128px.
fn load_art(url: &str) -> Option<Art> {
    let bytes = if let Some(p) = url.strip_prefix("file://") {
        std::fs::read(percent_decode(p)).ok()?
    } else if url.starts_with("http://") || url.starts_with("https://") {
        let mut buf = Vec::new();
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(6))).build().into();
        agent.get(url).call().ok()?.into_body().into_reader().take(8 * 1024 * 1024).read_to_end(&mut buf).ok()?;
        buf
    } else {
        return None;
    };
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let m = w.min(h);
    if m == 0 {
        return None;
    }
    let crop = image::imageops::crop_imm(&img, (w - m) / 2, (h - m) / 2, m, m).to_image();
    let sq = image::imageops::resize(&crop, ART, ART, image::imageops::FilterType::Lanczos3);
    // RGBA lurus -> premultiplied
    let mut data = sq.into_raw();
    for p in data.chunks_exact_mut(4) {
        let a = p[3] as u32;
        for c in &mut p[..3] {
            *c = (*c as u32 * a / 255) as u8;
        }
    }
    finish_art(&data)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = |c: Option<&u8>| c.and_then(|c| (*c as char).to_digit(16));
            if let (Some(h), Some(l)) = (hex(b.get(i + 1)), hex(b.get(i + 2))) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
