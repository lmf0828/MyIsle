"""Generate the MyIsle app icon: an 'M' monogram rising from a mint island.

Outputs:
  assets/myisle_preview_1024.png
  assets/myisle_256.png
  assets/myisle.ico  (16/24/32/48/64/128/256)
"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

OUT = Path(__file__).resolve().parent.parent / "assets"
S = 2048  # supersampled canvas (2x of 1024)
K = S / 1024.0


def sc(*v):
    return tuple(int(round(x * K)) for x in v)


def lerp(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(len(a)))


# ---------- rounded-square background with vertical gradient ----------
bg = Image.new("RGBA", (S, S), (0, 0, 0, 0))
grad = Image.new("RGBA", (S, S))
gd = ImageDraw.Draw(grad)
top, bot = (44, 49, 63, 255), (18, 20, 26, 255)
for y in range(S):
    gd.line([(0, y), (S, y)], fill=lerp(top, bot, y / S))
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle(sc(24, 24, 1000, 1000), radius=int(200 * K), fill=255)
bg.paste(grad, (0, 0), mask)

# soft mint glow behind the island
glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(glow).ellipse(sc(170, 520, 854, 980), fill=(70, 220, 175, 70))
glow = glow.filter(ImageFilter.GaussianBlur(int(90 * K)))
bg = Image.alpha_composite(bg, Image.composite(glow, Image.new("RGBA", (S, S), (0, 0, 0, 0)), mask))

# ---------- M monogram (custom geometric ribbon) ----------
M = [(330, 742), (330, 300), (412, 300), (512, 486), (612, 300), (694, 300), (694, 742),
     (608, 742), (608, 494), (512, 652), (416, 494), (416, 742)]
Mp = [sc(*p) for p in M]

# drop shadow
sh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(sh).polygon([(x, y + int(18 * K)) for x, y in Mp], fill=(0, 0, 0, 130))
sh = sh.filter(ImageFilter.GaussianBlur(int(22 * K)))
bg = Image.alpha_composite(bg, sh)

# white-to-mint-white gradient clipped to M
mg = Image.new("RGBA", (S, S))
md = ImageDraw.Draw(mg)
for y in range(S):
    md.line([(0, y), (S, y)], fill=lerp((255, 255, 255, 255), (226, 255, 244, 255), y / S))
mmask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mmask).polygon(Mp, fill=255)
bg.paste(mg, (0, 0), mmask)

# ---------- island (drawn on top so M appears planted in it) ----------
island_box = sc(158, 632, 866, 902)
im = Image.new("L", (S, S), 0)
ImageDraw.Draw(im).ellipse(island_box, fill=255)
ig = Image.new("RGBA", (S, S))
igd = ImageDraw.Draw(ig)
y0, y1 = island_box[1], island_box[3]
for y in range(y0, y1 + 1):
    igd.line([(0, y), (S, y)], fill=lerp((126, 233, 197, 255), (42, 155, 120, 255), (y - y0) / max(1, y1 - y0)))
# glossy highlight on the top-left of the island
hl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(hl).ellipse(sc(250, 655, 640, 745), fill=(255, 255, 255, 90))
hl = hl.filter(ImageFilter.GaussianBlur(int(26 * K)))
ig = Image.alpha_composite(ig, Image.composite(hl, Image.new("RGBA", (S, S), (0, 0, 0, 0)), im))
bg.paste(ig, (0, 0), im)

# thin rim light around the rounded square
rim = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(rim).rounded_rectangle(sc(24, 24, 1000, 1000), radius=int(200 * K),
                                     outline=(255, 255, 255, 60), width=int(5 * K))
bg = Image.alpha_composite(bg, rim)

final = bg.resize((1024, 1024), Image.LANCZOS)
final.save(OUT / "myisle_preview_1024.png")
final.resize((256, 256), Image.LANCZOS).save(OUT / "myisle_256.png")
final.save(OUT / "myisle.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
icon32 = final.resize((32, 32), Image.LANCZOS)
icon32.save(OUT / "myisle_32.png")
(OUT / "myisle_32.rgba").write_bytes(icon32.tobytes())
print("written:", OUT / "myisle.ico")
