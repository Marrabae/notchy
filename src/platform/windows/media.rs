//! Media Windows: polling SMTC (Spotify, browser, dll) + kontrol.
use crate::{
    media::{self, Art, Track, Tracker, ART},
    state::{Cmd, State},
};
use std::{sync::mpsc::Receiver, thread, time::Duration};
use windows::{
    Graphics::Imaging::{
        BitmapAlphaMode, BitmapBounds, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
        ColorManagementMode, ExifOrientationMode,
    },
    Media::Control::{
        GlobalSystemMediaTransportControlsSessionMediaProperties as Props,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PStatus,
    },
    Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
};

/// Thread polling SMTC + terima perintah kontrol.
pub fn spawn(state: State, rx: Receiver<Cmd>) {
    thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut tracker = Tracker::default();
        loop {
            if poll_once(&state, &rx, &mut tracker).is_err() {
                tracker.clear(&state);
            }
            thread::sleep(Duration::from_millis(500));
        }
    });
}

fn poll_once(state: &State, rx: &Receiver<Cmd>, tracker: &mut Tracker) -> windows::core::Result<()> {
    let mgr = Manager::RequestAsync()?.join()?;
    let session = mgr.GetCurrentSession()?;

    while let Ok(cmd) = rx.try_recv() {
        let _ = match cmd {
            Cmd::PlayPause => session.TryTogglePlayPauseAsync()?.join(),
            Cmd::Next => session.TrySkipNextAsync()?.join(),
            Cmd::Prev => session.TrySkipPreviousAsync()?.join(),
            Cmd::Seek(ms) => session.TryChangePlaybackPositionAsync(ms.max(0) * 10_000)?.join(),
        };
    }

    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let title = props.Title()?.to_string();
    let artist = props.Artist()?.to_string();
    let tl = session.GetTimelineProperties()?;
    let playing = session.GetPlaybackInfo()?.PlaybackStatus()? == PStatus::Playing;
    // Position dari SMTC bisa basi (app hanya update saat state berubah):
    // tambahkan selisih waktu sejak LastUpdatedTime (FILETIME, 100ns sejak 1601).
    let last = tl.LastUpdatedTime()?.UniversalTime;
    let now_ft = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() / 100)
        .unwrap_or(0) as i64)
        + 116_444_736_000_000_000;
    let drift = if playing && last > 0 { ((now_ft - last) / 10_000).clamp(0, 6 * 3_600_000) } else { 0 };
    let pos_ms = tl.Position()?.Duration / 10_000 + drift;
    let dur_ms = tl.EndTime()?.Duration / 10_000;

    tracker.publish(state, &Track { title, artist, playing, pos_ms, dur_ms }, || load_art(&props).ok());
    Ok(())
}

/// Ambil thumbnail dari SMTC, crop tengah jadi kotak 128px (RGBA premultiplied) + warna rata-rata.
fn load_art(props: &Props) -> windows::core::Result<Art> {
    const S: u32 = ART;
    let stream = props.Thumbnail()?.OpenReadAsync()?.join()?;
    let dec = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let (w, h) = (dec.PixelWidth()?, dec.PixelHeight()?);
    let m = w.min(h).max(1);
    let (sw, sh) = ((w * S).div_ceil(m), (h * S).div_ceil(m));
    let tf = BitmapTransform::new()?;
    tf.SetScaledWidth(sw)?;
    tf.SetScaledHeight(sh)?;
    tf.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    tf.SetBounds(BitmapBounds { X: (sw - S) / 2, Y: (sh - S) / 2, Width: S, Height: S })?;
    let pd = dec
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Rgba8,
            BitmapAlphaMode::Premultiplied,
            &tf,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?;
    let data = pd.DetachPixelData()?;
    media::finish_art(&data).ok_or_else(windows::core::Error::empty)
}
