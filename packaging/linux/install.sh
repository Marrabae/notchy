#!/bin/sh
# Pasang notchy untuk user saat ini (tanpa root): binary ke ~/.local/bin, ikon + launcher ke ~/.local/share.
# Pakai: install.sh [path/ke/notchy]   (default: ./notchy di tarball rilis, atau target/release/notchy di repo)
# Tambahkan --autostart untuk menjalankan notchy otomatis saat login.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
if [ -f "$here/notchy.png" ]; then
    # tarball rilis: semua file satu folder
    root=$here; icon=$here/notchy.png; default_bin=$here/notchy
else
    root=$(cd "$here/../.." && pwd); icon=$root/assets/notchy.png; default_bin=$root/target/release/notchy
fi
bin=${1:-$default_bin}
[ "$bin" = "--autostart" ] && bin=$default_bin

[ -x "$bin" ] || { echo "binary tidak ditemukan: $bin (jalankan: cargo build --release)" >&2; exit 1; }

prefix=${XDG_DATA_HOME:-$HOME/.local/share}
install -Dm755 "$bin" "$HOME/.local/bin/notchy"
install -Dm644 "$icon" "$prefix/icons/hicolor/256x256/apps/notchy.png"
# Exec pakai path absolut: ~/.local/bin sering belum ada di PATH sesi login
desktop() { sed "s|^Exec=.*|Exec=$HOME/.local/bin/notchy|" "$here/notchy.desktop" | install -Dm644 /dev/stdin "$1"; }
desktop "$prefix/applications/notchy.desktop"

for a in "$@"; do
    if [ "$a" = "--autostart" ]; then
        desktop "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/notchy.desktop"
        echo "autostart aktif"
    fi
done
echo "terpasang di ~/.local/bin/notchy (pastikan ~/.local/bin ada di PATH)"
