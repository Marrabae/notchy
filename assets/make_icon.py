"""Generate assets/notchy.ico (16-256px). Jalankan: python assets/make_icon.py"""
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024  # digambar besar lalu di-downscale -> anti-alias halus
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# latar: kotak membulat gelap dengan gradient vertikal
bg = Image.new("RGBA", (S, S))
for y in range(S):
    t = y / S
    c = (int(30 - 18 * t), int(30 - 18 * t), int(38 - 22 * t), 255)
    ImageDraw.Draw(bg).line([(0, y), (S, y)], fill=c)
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle([40, 40, S - 40, S - 40], radius=220, fill=255)
img.paste(bg, (0, 0), mask)

# notch: pill hitam menempel di atas
d.rounded_rectangle([212, 40, S - 212, 330], radius=140, fill=(0, 0, 0, 255))
d.rectangle([212, 40, S - 212, 170], fill=(0, 0, 0, 255))

# visualizer 4 bar di dalam notch
bars = [90, 150, 110, 170]
bw, gap = 46, 30
x = S / 2 - (4 * bw + 3 * gap) / 2
for h in bars:
    d.rounded_rectangle([x, 270 - h, x + bw, 270], radius=23, fill=(120, 200, 255, 255))
    x += bw + gap

# baris "lirik" di bawah
d.rounded_rectangle([250, 520, S - 250, 600], radius=40, fill=(255, 255, 255, 235))
d.rounded_rectangle([330, 660, S - 330, 720], radius=30, fill=(150, 150, 160, 200))

out = Path(__file__).with_name("notchy.ico")
img.save(out, sizes=[(256, 256), (64, 64), (48, 48), (32, 32), (24, 24), (16, 16)])
img.resize((256, 256), Image.LANCZOS).save(out.with_suffix(".png"))
print("wrote", out)
