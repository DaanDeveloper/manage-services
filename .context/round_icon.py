from PIL import Image, ImageDraw
import sys

src_path = sys.argv[1]
dst_path = sys.argv[2]
size = int(sys.argv[3]) if len(sys.argv) > 3 else 1024

inner = round(size * 824 / 1024)
offset = (size - inner) // 2

img = Image.open(src_path).convert("RGBA").resize((inner, inner), Image.LANCZOS)

mask = Image.new("L", (inner, inner), 0)
draw = ImageDraw.Draw(mask)
radius = round(inner * 0.2237)
draw.rounded_rectangle((0, 0, inner - 1, inner - 1), radius=radius, fill=255)

out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
out.paste(img, (offset, offset), mask)
out.save(dst_path)
print(f"wrote {dst_path} ({size}x{size}, r={radius})")
