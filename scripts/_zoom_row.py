from PIL import Image
im=Image.open(r"D:\LingDao\data\shot_upd_row_dot3.png").convert("RGB")
crop=im.crop((740,360,1040,430))
crop=crop.resize((crop.width*3,crop.height*3),Image.NEAREST)
crop.save(r"D:\LingDao\data\upd_row_zoom.png")
print(crop.size)
