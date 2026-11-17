from PIL import Image
im=Image.open(r"D:\LingDao\data\shot_upd_latest_state.png").convert("RGB")
im.crop((730,350,1045,430)).resize((945,240),Image.NEAREST).save(r"D:\LingDao\data\upd_latest_zoom.png")