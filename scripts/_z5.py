from PIL import Image
im=Image.open(r"D:\LingDao\data\shot_upd_row_dot7.png").convert("RGB")
im.crop((730,360,1045,430)).resize((945,210),Image.NEAREST).save(r"D:\LingDao\data\upd_row_zoom5.png")