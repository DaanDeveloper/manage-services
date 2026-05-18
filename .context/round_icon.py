from PIL import Image, ImageDraw
import sys

src_path = sys.argv[1]
dst_path = sys.argv[2]
size = int(sys.argv[3]) if len(sys.argv) > 3 else 1024

img = Image.open(src_path).convert("RGBA").resize((size, size), Image.LANCZOS)

mask = Image.new("L", (size, size), 0)
draw = ImageDraw.Draw(mask)
radius = round(size * 0.2237)
draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=255)

out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
out.paste(img, (0, 0), mask)
out.save(dst_path)
print(f"wrote {dst_path} ({size}x{size}, r={radius})")
