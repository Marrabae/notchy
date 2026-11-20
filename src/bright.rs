//! Kecerahan layar internal (laptop) lewat WMI root\WMI: WmiMonitorBrightness / WmiSetBrightness.
use crate::state::State;
use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::{w, BSTR, PCWSTR},
    Win32::System::{
        Com::{
            CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE,
            RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
        },
        Variant::{VariantClear, VARIANT, VT_I4, VT_UI1},
        Wmi::{
            IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
            WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE,
        },
    },
};

struct Wmi {
    svc: IWbemServices,
    /// __PATH instance WmiMonitorBrightnessMethods + parameter input siap pakai
    setter: Option<(BSTR, IWbemClassObject)>,
}

fn connect() -> Option<Wmi> {
    unsafe {
        let loc: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?;
        let e = BSTR::new();
        let svc = loc.ConnectServer(&BSTR::from(r"ROOT\WMI"), &e, &e, &e, 0, &e, None).ok()?;
        // RPC_C_AUTHN_WINNT = 10, RPC_C_AUTHZ_NONE = 0
        let _ = CoSetProxyBlanket(&svc, 10, 0, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE);
        let mut w = Wmi { svc, setter: None };
        w.setter = w.make_setter();
        Some(w)
    }
}

impl Wmi {
    fn first(&self, q: &str) -> Option<IWbemClassObject> {
        unsafe {
            let flags = WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
            let en = self.svc.ExecQuery(&BSTR::from("WQL"), &BSTR::from(q), flags, None).ok()?;
            let mut objs = [None];
            let mut n = 0u32;
            let _ = en.Next(WBEM_INFINITE, &mut objs, &mut n);
            objs[0].take()
        }
    }

    fn make_setter(&self) -> Option<(BSTR, IWbemClassObject)> {
        unsafe {
            let inst = self.first("SELECT * FROM WmiMonitorBrightnessMethods WHERE Active=TRUE")?;
            let mut v = VARIANT::default();
            inst.Get(w!("__PATH"), 0, &mut v, None, None).ok()?;
            let path: BSTR = (*v.Anonymous.Anonymous.Anonymous.bstrVal).clone();
            let _ = VariantClear(&mut v);

            let mut cls = None;
            self.svc
                .GetObject(&BSTR::from("WmiMonitorBrightnessMethods"), WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut cls), None)
                .ok()?;
            let mut sig = None;
            cls?.GetMethod(w!("WmiSetBrightness"), 0, &mut sig, std::ptr::null_mut()).ok()?;
            let params = sig?.SpawnInstance(0).ok()?;
            let mut t = VARIANT::default();
            (*t.Anonymous.Anonymous).vt = VT_I4;
            (*t.Anonymous.Anonymous).Anonymous.lVal = 0;
            params.Put(w!("Timeout"), 0, &t, 0).ok()?;
            Some((path, params))
        }
    }

    fn get(&self) -> Option<f32> {
        unsafe {
            let o = self.first("SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE")?;
            let mut v = VARIANT::default();
            o.Get(w!("CurrentBrightness"), 0, &mut v, None, None).ok()?;
            let inner = &v.Anonymous.Anonymous;
            let pct = match inner.vt {
                VT_UI1 => inner.Anonymous.bVal as i32,
                VT_I4 => inner.Anonymous.lVal,
                _ => -1,
            };
            let _ = VariantClear(&mut v);
            (pct >= 0).then(|| (pct.min(100)) as f32 / 100.0)
        }
    }

    fn set(&self, f: f32) -> bool {
        let Some((path, params)) = &self.setter else { return false };
        unsafe {
            let mut b = VARIANT::default();
            (*b.Anonymous.Anonymous).vt = VT_UI1;
            (*b.Anonymous.Anonymous).Anonymous.bVal = (f.clamp(0.0, 1.0) * 100.0).round() as u8;
            if params.Put(w!("Brightness"), 0, &b, 0).is_err() {
                return false;
            }
            self.svc
                .ExecMethod(path, &BSTR::from("WmiSetBrightness"), WBEM_GENERIC_FLAG_TYPE(0), None, params, None, None)
                .is_ok()
        }
    }
}

/// Thread kecerahan: baca berkala, set dari UI (nilai 0..1). Flash "Brightness X%" kalau diubah dari luar.
pub fn spawn(state: State) -> Sender<f32> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || run(state, rx));
    tx
}

fn run(state: State, rx: Receiver<f32>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut wmi = connect();
    let mut retry_at = Instant::now();
    let mut fails = 0u32;
    let mut first = true;
    let mut flash_lock: Option<Instant> = None;
    loop {
        let mut want = None;
        // tanpa panel internal (desktop) / WMI gagal: polling jarang
        let period = if wmi.is_some() { 3000 } else { 30_000 };
        match rx.recv_timeout(Duration::from_millis(period)) {
            Ok(v) => {
                want = Some(v);
                while let Ok(v) = rx.try_recv() {
                    want = Some(v);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if wmi.is_none() {
            if Instant::now() < retry_at {
                continue;
            }
            wmi = connect();
            retry_at = Instant::now() + Duration::from_secs(60);
        }
        let Some(w) = wmi.as_mut() else { continue };
        if let Some(v) = want {
            if w.setter.is_none() {
                w.setter = w.make_setter();
            }
            w.set(v);
            continue; // UI sudah menampilkan nilai baru; baca ulang di putaran berikut
        }
        let cur = w.get();
        // koneksi basi (resume dari sleep / service restart) atau memang tidak didukung -> sambung ulang nanti
        fails = if cur.is_none() { fails + 1 } else { 0 };
        if fails >= 3 {
            fails = 0;
            wmi = None;
            retry_at = Instant::now() + Duration::from_secs(60);
        }
        let mut s = state.lock().unwrap();
        if s.brightness_hold.is_some_and(|t| Instant::now() < t) {
            continue; // UI baru mengubah nilai; jangan timpa dengan bacaan lama
        }
        if let (Some(c), Some(old)) = (cur, s.brightness) {
            // adaptive brightness meredup bertahap: abaikan perubahan kecil & batasi flash
            if !first && (c - old).abs() >= 0.03 && flash_lock.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
                s.flash(format!("Brightness {}%", (c * 100.0).round() as i32));
                flash_lock = Some(Instant::now());
            }
        }
        if cur.is_some() || fails == 0 {
            s.brightness = cur;
        }
        first = false;
    }
}
