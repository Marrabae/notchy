//! Perangkat Bluetooth yang tersambung (classic + LE) via Windows.Devices.Enumeration, plus level baterainya.
use crate::state::{DevKind, Device, State};
use std::{
    collections::HashMap,
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::{IInspectable, Interface, GUID, HSTRING},
    Devices::Enumeration::{DeviceInformation, DeviceInformationKind},
    Foundation::IReference,
    Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
};
use windows_collections::IIterable;

/// Devnode PnP perangkat Bluetooth (classic + LE). Query AssociationEndpoint memicu scan radio
/// (~30 detik), sedangkan devnode PnP instan dan sudah punya status tersambung.
const AQS: &str = "System.Devices.DeviceInstanceId:~<\"BTHENUM\\DEV_\" OR System.Devices.DeviceInstanceId:~<\"BTHLE\\DEV_\"";
/// DEVPKEY_Device_IsConnected
const P_CONN: &str = "{83DA6326-97A6-4088-9453-A1923F573B29} 15";
const P_CONT: &str = "System.Devices.ContainerId";
/// DEVPKEY_Bluetooth_ClassOfDevice (CoD 24-bit; major class = bit 8..12)
const P_COD: &str = "{2BD67D8B-8BEB-48D5-87E0-6CDA3428040A} 10";
/// DEVPKEY_Bluetooth_Battery (persen, dilaporkan headset HFP / perangkat BLE)
const P_BATT: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";

fn props(list: &[&str]) -> IIterable<HSTRING> {
    IIterable::from(list.iter().map(|s| HSTRING::from(*s)).collect::<Vec<_>>())
}

fn num(v: &IInspectable) -> Option<u32> {
    if let Ok(r) = v.cast::<IReference<u8>>() {
        return r.Value().ok().map(u32::from);
    }
    if let Ok(r) = v.cast::<IReference<u16>>() {
        return r.Value().ok().map(u32::from);
    }
    if let Ok(r) = v.cast::<IReference<u32>>() {
        return r.Value().ok();
    }
    None
}

fn kind_of(name: &str, cod: Option<u32>) -> DevKind {
    DevKind::guess(name, cod)
}

fn battery_of(container: GUID) -> Option<u8> {
    let aqs = HSTRING::from(format!("System.Devices.ContainerId:=\"{{{container:?}}}\""));
    let col = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(&aqs, &props(&[P_BATT]), DeviceInformationKind::Device)
        .ok()?
        .join()
        .ok()?;
    let key = HSTRING::from(P_BATT);
    (0..col.Size().ok()?)
        .filter_map(|i| col.GetAt(i).ok())
        .filter_map(|d| d.Properties().ok()?.Lookup(&key).ok())
        .filter_map(|v| num(&v))
        .filter(|&p| p <= 100)
        .max()
        .map(|p| p as u8)
}

fn scan(cache: &mut HashMap<GUID, (Option<Instant>, Option<u8>)>) -> Option<Vec<Device>> {
    let col = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
        &HSTRING::from(AQS),
        &props(&[P_CONN, P_CONT, P_COD]),
        DeviceInformationKind::Device,
    )
    .ok()?
    .join()
    .ok()?;
    let (k_conn, k_cont, k_cod) = (HSTRING::from(P_CONN), HSTRING::from(P_CONT), HSTRING::from(P_COD));
    let mut out: Vec<Device> = Vec::new();
    for i in 0..col.Size().ok()? {
        let Ok(d) = col.GetAt(i) else { continue };
        let Ok(p) = d.Properties() else { continue };
        let connected = p
            .Lookup(&k_conn)
            .ok()
            .and_then(|v| v.cast::<IReference<bool>>().ok()?.Value().ok())
            .unwrap_or(false);
        let name = d.Name().map(|s| s.to_string()).unwrap_or_default();
        let cont = p.Lookup(&k_cont).ok().and_then(|v| v.cast::<IReference<GUID>>().ok()?.Value().ok());
        // perangkat dual-mode punya devnode classic + LE dalam satu container -> tampilkan sekali
        let id = cont.map(|g| format!("{g:?}")).unwrap_or_else(|| d.Id().map(|s| s.to_string()).unwrap_or_default());
        if !connected || name.trim().is_empty() || out.iter().any(|x| x.id == id) {
            continue;
        }
        let cod = p.Lookup(&k_cod).ok().and_then(|v| num(&v));
        let battery = cont.and_then(|g| {
            let e = cache.entry(g).or_insert((None, None));
            if e.0.is_none_or(|t: Instant| t.elapsed() > Duration::from_secs(30)) {
                *e = (Some(Instant::now()), battery_of(g));
            }
            e.1
        });
        out.push(Device { id, kind: kind_of(&name, cod), name, battery });
    }
    Some(out)
}

pub fn spawn(state: State) {
    thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut cache = HashMap::new();
        let mut first = true;
        loop {
            if let Some(list) = scan(&mut cache) {
                state.lock().unwrap().update_devices(list, first);
                first = false;
            }
            thread::sleep(Duration::from_secs(6));
        }
    });
}
