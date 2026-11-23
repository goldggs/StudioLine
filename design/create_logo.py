"""Draw the StudioLine SL mark as a flat, scalable logo.

Requires Pillow. Outputs an antialiased PNG and an editable SVG.
"""
from pathlib import Path
from PIL import Image, ImageDraw

OUTPUT = Path(__file__).resolve().parent
SIZE = 1024
SCALE = 4
INK = "#202b28"
GREEN = "#5b806b"

# Rectilinear letterforms share the same 88-unit stroke. The gap between the
# letters keeps the monogram readable when reduced to taskbar size.
s_shape = [
    (230, 276), (568, 276), (568, 364), (318, 364),
    (318, 468), (553, 468), (553, 748), (230, 748),
    (230, 660), (465, 660), (465, 556), (230, 556),
]
l_shape = [
    (624, 276), (712, 276), (712, 660), (794, 660),
    (794, 748), (624, 748),
]

canvas = Image.new("RGB", (SIZE * SCALE, SIZE * SCALE), "white")
draw = ImageDraw.Draw(canvas)
for points, color in ((s_shape, INK), (l_shape, GREEN)):
    draw.polygon([(x * SCALE, y * SCALE) for x, y in points], fill=color)
canvas.resize((SIZE, SIZE), Image.Resampling.LANCZOS).save(OUTPUT / "StudioLine-logo.png")

def points_str(points):
    return " ".join(f"{x},{y}" for x, y in points)

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" role="img" aria-labelledby="title">
<title id="title">StudioLine SL logo</title>
<rect width="1024" height="1024" fill="#ffffff"/>
<polygon points="{points_str(s_shape)}" fill="{INK}"/>
<polygon points="{points_str(l_shape)}" fill="{GREEN}"/>
</svg>\n'''
(OUTPUT / "StudioLine-logo.svg").write_text(svg, encoding="utf-8")
