# Rencana: Voice Assistant di notchy

Status: **direncanakan, belum dikerjakan** (dibuat 2026-11-20).

## Tujuan

Bicara ke notchy untuk mengontrol app (media, todo, timer, clipboard) dan bertanya hal umum,
dengan provider AI yang bisa dipilih user, tanpa mengorbankan sifat ringan notchy saat idle.

## Alur

```
Hotkey / klik mic → rekam audio → speech-to-text (lokal) → LLM + tool use → aksi app + jawaban → text-to-speech
```

LLM tidak menerima audio, jadi suara diubah ke teks dulu secara lokal.

## Komponen

| Bagian | Pilihan | Catatan |
|---|---|---|
| Pemicu | Hotkey global (mis. Alt+Space) + tombol mic di notch | Wake word ("hey notchy") ditunda: butuh mic menyala terus + engine tambahan |
| Rekam audio | crate `cpal` | Tampilkan gelombang suara di notch saat mendengarkan |
| Speech-to-text | `whisper.cpp` via `whisper-rs`, lokal | Model `ggml-base.bin` (~142 MB), Bahasa Indonesia OK. Dimuat hanya saat dipakai. Speech recognition bawaan Windows kemungkinan tidak mendukung Bahasa Indonesia |
| LLM | Multi-provider (lihat bawah) | Tool use untuk aksi app |
| Text-to-speech | WinRT `SpeechSynthesizer` | Gratis, offline, ada suara Bahasa Indonesia |
| HTTP | `ureq` (sudah ada), streaming SSE | Tidak ada SDK resmi untuk Rust |

Model whisper **tidak** dimasukkan ke installer; notchy mengunduhnya saat fitur suara pertama kali diaktifkan
(perlu persetujuan user, dari repo resmi `huggingface.co/ggerganov/whisper.cpp`).

## Provider (dua adapter)

| Adapter | Provider | Catatan |
|---|---|---|
| Claude (Messages API, native) | Anthropic | Default model `claude-opus-5-5`, effort `low` untuk respons cepat. Jangan lewat shim OpenAI-compatible. Forced `tool_choice` (`any`/`tool`) ditolak model ini, pakai `auto` + `strict: true` |
| OpenAI-compatible (Chat Completions) | OpenRouter, Gemini, Groq, Ollama/LM Studio, OpenAI, custom URL | Satu implementasi; beda base URL, model, key |

Preset base URL (**cek ulang di dokumentasi resmi saat implementasi**):

- OpenRouter: `https://openrouter.ai/api/v1` (model gratis berakhiran `:free`)
- Gemini: `https://generativelanguage.googleapis.com/v1beta/openai/` (free tier)
- Groq: `https://api.groq.com/openai/v1` (free tier, cepat)
- Ollama: `http://localhost:11434/v1` (lokal, tanpa key)
- Custom: base URL bebas

Catatan provider gratis:

- Model kecil/gratis kadang salah memanggil tool → validasi argumen tool sebelum dijalankan; jangan eksekusi tool call yang rusak.
- Free tier sebagian provider boleh memakai prompt untuk training → tampilkan peringatan kecil di Settings saat preset itu dipilih.
- Ollama gratis & privat, tapi install terpisah dan model makan RAM 2–8 GB (notchy hanya terhubung ke sana).
- Rate limit free tier → tampilkan pesan error di notch, jangan diam.

## Autentikasi & Settings

- **API key**, bukan OAuth. Key dimasukkan user sendiri dari notchy; langganan Claude.ai (Pro/Max) tidak bisa dipakai, key dari Claude Console.
- Jangan pernah menanam key developer di exe (bisa diekstrak, biaya teman tertagih ke kita). Opsi berbagi key nanti lewat server proxy.
- Halaman **Settings** baru di notch:
  - pilih provider/preset, field model (default per preset), field API key tersamar (`sk-ant-…••••`), tombol **Test** (cek key ke API), tombol **Remove**
  - key disimpan **per provider** di Windows Credential Manager (`CredWriteW`, feature `Win32_Security_Credentials`), bukan file teks
  - tanpa key: fitur suara mati + petunjuk singkat; fitur lain normal
- Input teks pakai mekanisme yang sama dengan todo (`gfx::set_activatable`).

## Tool yang diekspos ke LLM (versi pertama)

- `media_control` (play_pause / next / prev), `seek` (detik)
- `add_todo`, `list_todos`, `complete_todo`
- `timer_start` / `timer_pause` / `timer_reset`, `timer_set` (fokus/istirahat menit)
- `clipboard_paste` (item ke-n), `now_playing`, `system_status` (baterai/volume)

Semua tool dijalankan lokal di notchy (loop tool sendiri di Messages API / Chat Completions — bukan Managed Agents, karena aksi lokal & butuh latensi rendah).
Gunakan prompt caching untuk system prompt + daftar tool (Claude).

## UI di notch

- Mendengarkan: gelombang suara + "Listening…"
- Memproses: transkrip ucapan user
- Menjawab: teks jawaban (streaming) + TTS; aksi yang dijalankan muncul sebagai live activity singkat

## Perkiraan

- Latensi: ~2–3 detik dari selesai bicara ke jawaban (STT lokal <1 dtk, token pertama ~1 dtk, TTS langsung mulai).
- Biaya Claude: ~$0.005–0.01 per perintah (Opus 5.5), lebih murah dengan caching. Provider gratis: $0 dengan rate limit.
- RAM idle tidak berubah; naik hanya saat whisper/LLM dipakai.

## Privasi

- Mic hanya menyala saat hotkey/tombol ditekan.
- Audio tidak keluar dari PC (STT lokal); yang dikirim ke provider hanya teks perintah + konteks yang dibutuhkan (mis. judul lagu).

## Urutan implementasi

1. Halaman Settings + penyimpanan key di Credential Manager + tombol Test (per adapter)
2. Adapter LLM (Claude native, OpenAI-compatible) + loop tool + validasi
3. Rekam audio (`cpal`) + whisper lokal (unduh model saat pertama dipakai)
4. TTS WinRT + UI listening/answer di notch
5. Hotkey global + tombol mic
6. Uji per provider (Claude, Gemini, OpenRouter `:free`, Groq, Ollama), ukur latensi & RAM, rebuild installer
