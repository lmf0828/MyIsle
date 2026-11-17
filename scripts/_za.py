from PIL import Image
im=Image.open(r"D:\LingDao\data\shot_upd_available_state4.png").convert("RGB")
im.crop((600,350,1045,430)).resize((1335,240),Image.NEAREST).save(r"D:\LingDao\data\upd_available_zoom4.png")