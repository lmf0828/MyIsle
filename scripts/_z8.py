from PIL import Image
im=Image.open(r"D:\LingDao\data\shot_upd_available_state.png").convert("RGB")
im.crop((650,350,1045,430)).resize((1185,240),Image.NEAREST).save(r"D:\LingDao\data\upd_available_zoom2.png")