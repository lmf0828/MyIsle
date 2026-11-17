# -*- coding: utf-8 -*-
"""Generate the 'MyIsle' brand icon: lowercase 'i' (sun-dot + stem) standing on a
small island mound with a teal water reflection, inside a rounded dark app tile."""
import os
import numpy as np
from PIL import Image, ImageDraw

OUT_DIR = r"D:\LingDao\assets"
S = 4
BASE = 1024
B = BASE * S

TOP = np.array([40, 45, 57], dtype=np.float64)
BOT = np.array([15, 17, 23], dtype=np.float64)
TEAL_LIGHT = np.array([120, 246, 198], dtype=np.float64)   # rim
TEAL_MID   = np.array([32, 208, 152], dtype=np.float64)
TEAL_DARK  = np.array([9, 122, 90], dtype=np.float64)
WHITE      = (245, 255, 251)
TL = tuple(int(v) for v in TEAL_LIGHT)   # int tuple for PIL fills


def vgrad(size, top, bot):
    ys = np.arange(size).reshape(size, 1).astype(np.float64) / (size - 1)
    row = top[None, :] * (1 - ys) + bot[None, :] * ys
    return np.repeat(row[:, None, :], size, axis=1)


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


# ---------- background tile ----------
bg_arr = vgrad(B, TOP, BOT)
yy, xx = np.mgrid[0:B, 0:B].astype(np.float64)
d0 = np.sqrt((xx - B * 0.30) ** 2 + (yy - B * 0.14) ** 2) / B
bg_arr = np.clip(bg_arr + np.clip(1.0 - d0 * 2.1, 0, 1)[..., None] * 16.0, 0, 255)
bg = Image.fromarray(bg_arr.astype(np.uint8), "RGB").convert("RGBA")

radius = int(B * 0.225)
mask = Image.new("L", (B, B), 0)
ImageDraw.Draw(mask).rounded_rectangle([0, 0, B - 1, B - 1], radius=radius, fill=255)
tile = Image.new("RGBA", (B, B), (0, 0, 0, 0))
tile.paste(bg, (0, 0), mask)

cx = B * 0.5

# ---------- beacon glow behind dot ----------
dot_cy = B * 0.275
dot_r = B * 0.068
glow = Image.new("RGBA", (B, B), (0, 0, 0, 0))
dg = ImageDraw.Draw(glow)
for k in range(20, 0, -1):
    rr = dot_r * (1.0 + k * 0.40)
    a = int(30 * (1 - k / 20.0) ** 1.7)
    dg.ellipse([cx - rr, dot_cy - rr, cx + rr, dot_cy + rr],
               fill=(TL[0], TL[1], TL[2], a))
tile = Image.alpha_composite(tile, glow)

# ---------- island mound (numpy, crisp + lit rim) ----------
isl_cx = B * 0.5
isl_cy = B * 0.790
isl_rx = B * 0.305
isl_ry = B * 0.140
rn = np.sqrt(((xx - isl_cx) / isl_rx) ** 2 + ((yy - isl_cy) / isl_ry) ** 2)
inside = rn <= 1.0
ynorm = np.clip((yy - (isl_cy - isl_ry)) / (2 * isl_ry), 0, 1)

# base vertical gradient across the mound
t = (ynorm ** 0.75)[..., None]
col = TEAL_LIGHT[None, None, :] * (1 - t) + TEAL_DARK[None, None, :] * t

# bright lit rim near the upper boundary
rim = smoothstep(0.80, 1.0, rn) * (yy < isl_cy)
col = col + (np.array([235, 255, 248])[None, None, :] - col) * (rim[..., None] * 0.75)

# gentle darkening toward the very bottom (depth)
depth = smoothstep(0.55, 1.0, ynorm)[..., None] * 0.22
col = col * (1 - depth)

isl_arr = np.zeros((B, B, 4), dtype=np.uint8)
isl_arr[..., :3] = np.clip(col, 0, 255).astype(np.uint8)
isl_arr[..., 3] = (inside * 255).astype(np.uint8)
island = Image.fromarray(isl_arr, "RGBA")
tile = Image.alpha_composite(tile, island)

# ---------- water reflection under island ----------
refl = Image.new("RGBA", (B, B), (0, 0, 0, 0))
dr = ImageDraw.Draw(refl)
ry0 = isl_cy + isl_ry + B * 0.006
for idx, (wf, al) in enumerate(zip([0.26, 0.17, 0.095], [95, 62, 34])):
    yy2 = ry0 + idx * B * 0.026
    hw = B * wf * 0.5
    dr.rounded_rectangle([cx - hw, yy2, cx + hw, yy2 + B * 0.009],
                         radius=B * 0.0045,
                         fill=(TL[0], TL[1], TL[2], al))
tile = Image.alpha_composite(tile, refl)

# ---------- the lowercase 'i' ----------
i_layer = Image.new("RGBA", (B, B), (0, 0, 0, 0))
dd = ImageDraw.Draw(i_layer)
stem_top = B * 0.405
stem_bot = B * 0.735
stem_hw = B * 0.055
dd.rounded_rectangle([cx - stem_hw, stem_top, cx + stem_hw, stem_bot],
                     radius=stem_hw, fill=WHITE + (255,))
# dot: teal ring + white core
dd.ellipse([cx - dot_r, dot_cy - dot_r, cx + dot_r, dot_cy + dot_r],
           fill=(TL[0], TL[1], TL[2], 255))
dd.ellipse([cx - dot_r * 0.72, dot_cy - dot_r * 0.78,
            cx + dot_r * 0.72, dot_cy + dot_r * 0.66],
           fill=WHITE + (255,))
tile = Image.alpha_composite(tile, i_layer)

# ---------- outer subtle border ----------
bd = ImageDraw.Draw(tile)
ins = int(B * 0.006)
bd.rounded_rectangle([ins, ins, B - 1 - ins, B - 1 - ins],
                     radius=radius - ins,
                     outline=(255, 255, 255, 26), width=max(1, int(B * 0.0035)))

# ---------- export ----------
os.makedirs(OUT_DIR, exist_ok=True)
master = tile.resize((BASE, BASE), Image.LANCZOS)
master.save(os.path.join(OUT_DIR, "island_preview_1024.png"))
sizes = [16, 24, 32, 48, 64, 128, 256]
imgs = [master.resize((s, s), Image.LANCZOS) for s in sizes]
imgs[-1].save(os.path.join(OUT_DIR, "island.ico"), format="ICO",
              sizes=[(s, s) for s in sizes])
master.save(os.path.join(OUT_DIR, "island_256.png"))
print("ok")
