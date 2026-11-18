# -*- coding: utf-8 -*-
"""Generate the WinIsle brand icon assets from the selected design
(gradient tile + black island + Otto 'confident' mint eyes + WinIsle wordmark).

Outputs (into ..\\assets):
  winisle.ico            16/24/32/48/64/128/256  (multi-frame; wordmark only >=64)
  winisle_256.png        wordmark @256
  winisle_preview_1024.png  wordmark @1024
  winisle_32.png         mark(no text) @32
  winisle_32.rgba        mark @32 raw RGBA (for tray-icon)
"""
import os, struct
import numpy as np
from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets")
CONF = os.path.join(ROOT, "design", "confident_mint.png")
FONT = r"C:\Windows\Fonts\segoeuib.ttf"

R = 16                      # supersample: 128 viewBox * 16 = 2048 canvas
S = 128 * R
C1 = np.array([0x34, 0xd3, 0x99], float)   # #34d399 top-left
C2 = np.array([0x0e, 0xa5, 0xe9], float)   # #0ea5e9 bottom-right
ISLAND = (0x0b, 0x0f, 0x14, 255)
TEXTCOL = (0x06, 0x23, 0x1a, 255)


def u(v):
    return v * R


def gradient_tile():
    yy, xx = np.mgrid[0:S, 0:S].astype(np.float64)
    t = np.clip((xx / (S - 1) + yy / (S - 1)) / 2.0, 0, 1)[..., None]
    arr = C1[None, None, :] * (1 - t) + C2[None, None, :] * t
    img = Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8), "RGB").convert("RGBA")
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([u(4), u(4), u(124), u(124)], radius=int(u(28)), fill=255)
    tile = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    tile.paste(img, (0, 0), mask)
    return tile


def island_layer(rect, eye_w, eye_h, eye_x, eye_y, rot_center, angle=8):
    """Draw the black island + mint confident eyes, then rotate about rot_center (SVG -angle)."""
    layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    x, y, w, h, r = rect
    d.rounded_rectangle([u(x), u(y), u(x + w), u(y + h)], radius=int(u(r)), fill=ISLAND)
    eyes = Image.open(CONF).convert("RGBA").resize((int(u(eye_w)), int(u(eye_h))), Image.LANCZOS)
    layer.alpha_composite(eyes, (int(u(eye_x)), int(u(eye_y))))
    # PIL rotate positive = counter-clockwise == SVG rotate(-angle)
    return layer.rotate(angle, center=(u(rot_center[0]), u(rot_center[1])), resample=Image.BICUBIC, expand=False)


def text_layer(txt):
    layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    # letter-spacing 0.3 units
    font = ImageFont.truetype(FONT, int(u(18)))
    sp = u(0.3)
    widths = [d.textlength(ch, font=font) for ch in txt]
    total = sum(widths) + sp * (len(txt) - 1)
    cx = u(64)
    x = cx - total / 2.0
    # vertical: center of cap height around y=96 unit
    asc, desc = font.getmetrics()
    cy = u(96)
    baseline = cy + (asc - desc) / 2.0 - u(1.2)
    for ch, w in zip(txt, widths):
        d.text((x, baseline), ch, font=font, fill=TEXTCOL, anchor="ls")
        x += w + sp
    return layer


def compose(with_text=True):
    tile = gradient_tile()
    if with_text:
        isl = island_layer((24, 26, 80, 40, 20), 70, 21.6, 29, 33.2, (64, 46))
        tile = Image.alpha_composite(tile, isl)
        tile = Image.alpha_composite(tile, text_layer("WinIsle"))
    else:
        isl = island_layer((27, 46, 74, 36, 18), 64, 19.75, 32, 54.125, (64, 64))
        tile = Image.alpha_composite(tile, isl)
    return tile


def to_size(img, s):
    return img.resize((s, s), Image.LANCZOS)


def write_ico(path, frames):
    n = len(frames)
    entries = b""
    blobs = []
    offset = 6 + 16 * n
    for size, im in frames:
        im = im.convert("RGBA")
        w = h = im.size[0]
        bih = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, w * h * 4, 0, 0, 0, 0)
        px = im.load()
        rows = []
        for yy in range(h - 1, -1, -1):
            row = bytearray()
            for xx in range(w):
                r_, g_, b_, a_ = px[xx, yy]
                row += bytes((b_, g_, r_, a_))
            rows.append(bytes(row))
        xor = b"".join(rows)
        rowbytes = ((w + 31) // 32) * 4
        andmask = b"\x00" * (rowbytes * h)
        blob = bih + xor + andmask
        blobs.append((size, blob))
    for size, blob in blobs:
        bw = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", bw, bw, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
    with open(path, "wb") as f:
        f.write(struct.pack("<HHH", 0, 1, n) + entries + b"".join(b for _, b in blobs))


master_name = compose(True)
master_mark = compose(False)

master_name.resize((1024, 1024), Image.LANCZOS).save(os.path.join(ASSETS, "winisle_preview_1024.png"))
master_name.resize((256, 256), Image.LANCZOS).save(os.path.join(ASSETS, "winisle_256.png"))

sizes = [16, 24, 32, 48, 64, 128, 256]
frames = []
for s in sizes:
    src = master_name if s >= 64 else master_mark
    frames.append((s, to_size(src, s)))
write_ico(os.path.join(ASSETS, "winisle.ico"), frames)

icon32 = to_size(master_mark, 32)
icon32.save(os.path.join(ASSETS, "winisle_32.png"))
open(os.path.join(ASSETS, "winisle_32.rgba"), "wb").write(icon32.tobytes())

print("icon assets written; ico sizes:", sizes)
