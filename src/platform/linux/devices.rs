//! Perangkat Bluetooth yang tersambung via BlueZ (D-Bus system bus), plus level baterainya (org.bluez.Battery1).
use crate::state::{DevKind, Device, State};
use std::{collections::HashMap, thread, time::Duration};
use zbus::{
    blocking::{connection::Builder, Connection},
    zvariant::{OwnedObjectPath, OwnedValue},
};

type Props = HashMap<String, OwnedValue>;
type Managed = HashMap<OwnedObjectPath, HashMap<String, Props>>;

fn prop<T: TryFrom<OwnedValue>>(p: &Props, k: &str) -> Option<T> {
    T::try_from(p.get(k)?.try_clone().ok()?).ok()
}

fn kind_of(icon: Option<&str>, cod: Option<u32>, name: &str) -> DevKind {
    match icon.unwrap_or_default() {
        i if i.starts_with("audio-") => DevKind::Audio,
        i if i.starts_with("input-") => DevKind::Input,
        "phone" | "multimedia-player" => DevKind::Phone,
        _ => DevKind::guess(name, cod),
    }
}

fn scan(conn: &Connection) -> Option<Vec<Device>> {
    let msg = conn
        .call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
        .ok()?;
    let objs: Managed = msg.body().deserialize().ok()?;
    let mut out: Vec<Device> = Vec::new();
    for ifaces in objs.values() {
        let Some(d) = ifaces.get("org.bluez.Device1") else { continue };
        if prop::<bool>(d, "Connected") != Some(true) {
            continue;
        }
        let name = prop::<String>(d, "Alias").or_else(|| prop::<String>(d, "Name")).unwrap_or_default();
        let id = prop::<String>(d, "Address").unwrap_or_else(|| name.clone());
        if name.trim().is_empty() || out.iter().any(|x| x.id == id) {
            continue;
        }
        let battery = ifaces.get("org.bluez.Battery1").and_then(|b| prop::<u8>(b, "Percentage")).filter(|&p| p <= 100);
        let kind = kind_of(prop::<String>(d, "Icon").as_deref(), prop::<u32>(d, "Class"), &name);
        out.push(Device { id, name, kind, battery });
    }
    Some(out)
}

pub fn spawn(state: State) {
    thread::spawn(move || {
        let mut conn: Option<Connection> = None;
        let mut first = true;
        loop {
            if conn.is_none() {
                conn = Builder::system().ok().and_then(|b| b.method_timeout(Duration::from_secs(3)).build().ok());
            }
            // bluetoothd tidak jalan / tidak ada adapter -> scan gagal; coba lagi nanti tanpa mengubah daftar
            match conn.as_ref().and_then(scan) {
                Some(list) => {
                    state.lock().unwrap().update_devices(list, first);
                    first = false;
                    thread::sleep(Duration::from_secs(6));
                }
                None => thread::sleep(Duration::from_secs(20)),
            }
        }
    });
}
