# notchy

Dynamic notch untuk Windows dan Linux (X11/XWayland), ditulis dengan Rust (winit + tiny-skia). Ringan: ~8 MB RAM, CPU <1% saat idle.

## Fitur

- Now Playing (SMTC di Windows, MPRIS di Linux): artwork, kontrol media, seek timeline
- Lirik tersinkron (lrclib.net): inline, pill saat collapsed, atau jendela PiP yang bisa di-resize; klik baris untuk lompat
- Live activity baterai & volume
- File shelf (drag masuk, drag keluar untuk memindahkan)
- Clipboard viewer (teks & gambar), klik untuk paste ke field aktif
- Todo list
- Timer Pomodoro yang bisa diatur
- Halaman System info (baterai, jaringan, volume, brightness, Bluetooth)

## Tampilan

- Warna aksen per halaman; di halaman Media aksen dan glow diambil dari artwork lagu
- Tepi kaca tipis, shadow lembut, chip bergradient dengan hover berwarna
- Animasi: spring buka/tutup, geser antar halaman, item muncul berurutan, centang todo, tombol media memantul saat ditekan
- Empty state dengan drop zone untuk Shelf
- Mode peek: setelah 30 detik tidak dipakai, notch mengecil jadi tab 10px dengan penanda (garis aksen saat lagu main, progres saat timer jalan); hover untuk membuka. Tidak aktif selagi mode lirik menyala dan ada lagu. Aktifkan lewat ikon di pojok kanan bawah halaman Media

## Build

```powershell
cargo build --release
```

Installer (Inno Setup):

```powershell
ISCC.exe installer\notchy.iss
```

Hasilnya di `dist\`. Versi dan folder build bisa dioverride: `ISCC.exe /DAppVersion=1.2.3 /DBuildDir=<folder-exe> installer\notchy.iss`.

## Rilis

Dorong tag `vX.Y.Z` (harus sama dengan `version` di `Cargo.toml`) dan GitHub Actions (`.github/workflows/release.yml`)
membuat installer Windows (`notchy-setup-X.Y.Z.exe`), paket `.deb`, dan tarball Linux, lalu menerbitkannya di GitHub Releases.
`ci.yml` menjalankan `cargo check` dan `cargo test` di Windows dan Ubuntu untuk tiap push dan PR.
Paket `.deb` lokal: `cargo build --release && cargo deb --no-build` (butuh `cargo install cargo-deb`).

## Linux

Target: sesi X11 atau Wayland lewat XWayland (butuh compositor untuk transparansi: GNOME, KDE, XFCE, Sway+XWayland, dll).
Native Wayland (layer-shell) belum ada.

```sh
cargo build --release
packaging/linux/install.sh --autostart   # ke ~/.local/bin + launcher + autostart (opsional)
```

Baseline: Ubuntu 24.04 LTS (glibc 2.39) atau lebih baru; paket `.deb` menolak host dengan `libc6` lebih lama. Rilis dibuild di runner `ubuntu-24.04` supaya binary tidak butuh glibc yang lebih baru.

Build butuh compiler C (untuk `ring`, TLS lirik). Runtime butuh library X11 yang di-load dinamis (`libxkbcommon-x11-0`, `libx11-6`, `libxcursor1`, `libxi6`, `libxrandr2`; sudah ada di desktop X11 umum), tanpa paket dev. Yang dipakai:

| Fitur | Sumber di Linux | Paket |
| --- | --- | --- |
| Media, artwork, kontrol | MPRIS (D-Bus session) | player apa pun yang mendukung MPRIS |
| Volume, mute, nama output | `pactl` (+ `pactl subscribe`) | `pulseaudio-utils` / `pipewire-pulse` |
| Baterai | `/sys/class/power_supply`, `powerprofilesctl` (opsional) | - |
| Brightness | `/sys/class/backlight`; tulis via sysfs atau logind | - |
| Jaringan | `/proc/net`, `nmcli` / `iwgetid` untuk nama | `network-manager` (opsional) |
| Bluetooth + baterai device | BlueZ (D-Bus system) | `bluez` |
| Clipboard + paste | XFixes, arboard, XTest | - |
| Drag file keluar | XDND (sumber) | - |
| Font | `fc-match`, cadangan DejaVu/Liberation/Noto | `fontconfig`, `fonts-dejavu-core` |
| Buka file / pengaturan | `xdg-open`, `gnome-control-center` / `systemsettings` | - |

Config dan todo di `~/.config/notchy/`. Satu instance dijaga dengan flock di `$XDG_RUNTIME_DIR`.

Catatan: tanpa compositor, area transparan tampil hitam. Di window manager tanpa dukungan `_NET_WM_STATE_ABOVE`
untuk tipe dock, notch bisa tertutup window lain.

## Status

Windows dan Linux (X11). Rencana voice assistant: [docs/voice-assistant-plan.md](docs/voice-assistant-plan.md).
