//! Backend sistem Windows: volume (WASAPI), baterai, byte jaringan (IpHelper), profil jaringan (WinRT).
use crate::{
    state::{NetKind, Power},
    sys::{AudioInfo, Backend},
};
use std::{
    collections::HashMap,
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

pub struct Sys {
    au: Option<Audio>,
    au_at: Instant,
}

/// Dipanggil dari thread sistem (COM per-thread).
pub fn backend() -> Sys {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    Sys { au: audio(), au_at: Instant::now() }
}

impl Backend for Sys {
    fn audio(&mut self) -> Option<AudioInfo> {
        // device default bisa berganti (mis. headset BT tersambung) -> cek ulang berkala
        if self.au.is_none() || self.au_at.elapsed() > Duration::from_secs(3) {
            let fresh = audio();
            if fresh.is_some() || self.au.is_none() {
                self.au = fresh;
            }
            self.au_at = Instant::now();
        }
        let a = self.au.as_ref()?;
        let Some(vol) = (unsafe { a.vol.GetMasterVolumeLevelScalar().ok() }) else {
            self.au = None;
            return None;
        };
        let muted = unsafe { a.vol.GetMute().ok() }.map(|m| m.as_bool()).unwrap_or(false);
        Some(AudioInfo { vol, muted, name: a.name.clone() })
    }

    fn set_volume(&mut self, v: f32) {
        if let Some(a) = &self.au {
            unsafe {
                let _ = a.vol.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null());
                if a.vol.GetMute().is_ok_and(|m| m.as_bool()) {
                    let _ = a.vol.SetMute(false, std::ptr::null());
                }
            }
        }
    }

    fn toggle_mute(&mut self) {
        if let Some(a) = &self.au {
            unsafe {
                let m = a.vol.GetMute().map(|m| m.as_bool()).unwrap_or(false);
                let _ = a.vol.SetMute(!m, std::ptr::null());
            }
        }
    }

    fn battery(&mut self) -> Option<(u8, bool, Power)> {
        battery()
    }

    fn net_bytes(&mut self) -> Option<HashMap<u32, (u64, u64)>> {
        net_bytes()
    }

    fn net_profile(&mut self) -> (NetKind, String) {
        net_profile()
    }
}
