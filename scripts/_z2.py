from PIL import Image
im=Image.open(r"D:\LingDao\data\shot_upd_row_dot4.png").convert("RGB")
c=im.crop((730,360,1045,430)).resize((945,210),Image.NEAREST)
c.save(r"D:\LingDao\data\upd_row_zoom2.png")
print(c.size)