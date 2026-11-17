from PIL import Image
im=Image.open(r"D:\LingDao\data\shot_update_pill_big.png").convert("RGB")
minx,miny,maxx,maxy=10**9,10**9,-1,-1
for y in range(10,60):
    for x in range(600,900):
        r,g,b=im.getpixel((x,y))
        if g>r+8 and b>r+4 and g<150:
            minx=min(minx,x);maxx=max(maxx,x);miny=min(miny,y);maxy=max(maxy,y)
print("pill",(minx,miny,maxx,maxy),(maxx-minx+1,maxy-miny+1))
