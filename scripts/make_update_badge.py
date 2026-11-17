from PIL import Image, ImageDraw
import numpy as np

src = r"D:\dsh\home\attachments\v1\objects\5e\5e664f013981729479249512131f10abc31ef41430ae41c923f910610614160e"
im = Image.open(src).convert("RGBA")
a = np.array(im)
r, g = a[:, :, 0].astype(int), a[:, :, 1].astype(int)
mask = (g - r > 10) & (g > 60)
ys, xs = np.where(mask)
x0, x1, y0, y1 = xs.min() - 1, xs.max() + 1, ys.min() - 1, ys.max() + 1
print("bbox", x0, x1, y0, y1, "size", x1 - x0 + 1, y1 - y0 + 1)
crop = im.crop((x0, y0, x1 + 1, y1 + 1))
w, h = crop.size
S = 4
big = Image.new("L", (w * S, h * S), 0)
d = ImageDraw.Draw(big)
d.rounded_rectangle([0, 0, w * S - 1, h * S - 1], radius=(h * S) // 2, fill=255)
m = big.resize((w, h), Image.LANCZOS)
out = crop.copy()
out.putalpha(m)
out.save(r"D:\LingDao\assets\update_badge.png")
print("saved", w, h)
prev = Image.new("RGBA", (w * 8, h * 8), (28, 28, 34, 255))
prev.alpha_composite(out.resize((w * 8, h * 8), Image.NEAREST))
prev.convert("RGB").save(r"D:\LingDao\data\update_badge_preview.png")
