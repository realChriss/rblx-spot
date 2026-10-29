from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent

S = 2048
INK, PAPER = "#1E2229", "#F4F5F7"
TILT = -15
SIDE, RADIUS, EDGE = 1400, 230, 60

im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(im)
a, b, c = (S - SIDE) / 2, (S + SIDE) / 2, S / 2
d.rounded_rectangle((a - EDGE, a - EDGE, b + EDGE, b + EDGE), RADIUS + EDGE, fill=INK)
d.rounded_rectangle((a, a, b, b), RADIUS, fill=PAPER)
for r, fill in ((400, INK), (236, PAPER), (124, INK)):
    d.ellipse((c - r, c - r, c + r, c + r), fill=fill)
im = im.rotate(TILT, resample=Image.BICUBIC, center=(c, c))

# crop to the shape so it fills the tray slot like other tray icons
MARGIN = 0.02
im = im.crop(im.getbbox())
side = round(max(im.size) / (1 - 2 * MARGIN))
square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
square.paste(im, ((side - im.width) // 2, (side - im.height) // 2))
im = square

sizes = [16, 20, 24, 32, 40, 48, 64, 256]
frames = [im.resize((s, s), Image.LANCZOS) for s in sizes]
frames[-1].save(ROOT / "icon.ico", sizes=[(s, s) for s in sizes], append_images=frames)
frames[-1].save(ROOT / "icon.png")
