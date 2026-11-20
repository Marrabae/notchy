use crate::state::State;
use std::{thread, time::Duration};
use windows::Win32::{
    Media::Audio::{
        eMultimedia, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator,
    },
    System::{
        Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED},
        Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
    },
};

fn volume_ctl() -> Option<IAudioEndpointVolume> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia).ok()?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

fn battery() -> Option<(u8, bool)> {
    let mut p = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut p).ok()? };
    if p.BatteryLifePercent == 255 {
        return None; // desktop tanpa baterai
    }
    Some((p.BatteryLifePercent, p.ACLineStatus == 1))
}

/// Thread kecil: baterai + volume, memicu "live activity" saat berubah.
pub fn spawn(state: State) {
    thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut vol = volume_ctl();
        let mut first = true;
        loop {
            let v = vol
                .as_ref()
                .and_then(|c| unsafe { c.GetMasterVolumeLevelScalar().ok() });
            if v.is_none() {
                vol = volume_ctl(); // device default berubah / belum ada
            }
            let b = battery();
            {
                let mut s = state.lock().unwrap();
                if let Some(v) = v {
                    if !first && (v - s.volume).abs() > 0.004 {
                        s.flash(format!("Volume {}%", (v * 100.0).round() as i32));
                    }
                    s.volume = v;
                }
                if !first {
                    match (s.battery, b) {
                        (Some((_, c0)), Some((p, c1))) if c0 != c1 => {
                            s.flash(if c1 { format!("Charging {p}%") } else { format!("On battery {p}%") })
                        }
                        (Some((p0, _)), Some((p, false))) if p != p0 && p == 20 => {
                            s.flash(format!("Battery low {p}%"))
                        }
                        _ => {}
                    }
                }
                s.battery = b;
            }
            first = false;
            thread::sleep(Duration::from_millis(300));
        }
    });
}
