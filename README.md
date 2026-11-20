# notchy

Dynamic notch untuk Windows, ditulis dengan Rust (winit + tiny-skia + windows-rs). Ringan: ~8 MB RAM, CPU <1% saat idle.

## Fitur

- Now Playing (SMTC): artwork, kontrol media, seek timeline
- Lirik tersinkron (lrclib.net): inline, pill saat collapsed, atau jendela PiP yang bisa di-resize; klik baris untuk lompat
- Live activity baterai & volume
- File shelf (drag masuk, drag keluar untuk memindahkan)
- Clipboard viewer (teks & gambar), klik untuk paste ke field aktif
- Todo list
- Timer Pomodoro yang bisa diatur

## Build

```powershell
cargo build --release
```

Installer (Inno Setup):

```powershell
ISCC.exe installer\notchy.iss
```

Hasilnya di `dist\`.

## Status

Windows saja untuk sekarang. Rencana voice assistant: [docs/voice-assistant-plan.md](docs/voice-assistant-plan.md).
