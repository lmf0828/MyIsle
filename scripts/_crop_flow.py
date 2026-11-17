from PIL import Image
im=Image.open(r"D:\LingDao\data\update_flow_v4.png").convert("RGB")
w,h=im.size
last=0
for y in range(h):
    row=im.crop((0,y,w,y+1))
    # count pixels brighter than background
    mx=max(row.getdata(), key=lambda p:p[0]+p[1]+p[2])
    if mx[0]+mx[1]+mx[2] > 150: last=y
print(w,h,last)
im.crop((0,0,w,min(h,last+60))).save(r"D:\LingDao\data\update_flow_v4_crop.png")