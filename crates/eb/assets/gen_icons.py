#!/usr/bin/env python3
"""Generate EvoBuddy's own monochrome icon set.

Source of truth for the PNGs under assets/icons/. Everything is drawn here from
simple geometric primitives; no third-party icon set is copied.

Run from the repository root:
    python3 crates/eb/assets/gen_icons.py
"""
from pathlib import Path
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
SIZE = 96
OUT = 48
BLACK = (0, 0, 0, 205)
GREEN = (0, 194, 154, 255)
DARK = (22, 43, 38, 255)


def canvas():
    return Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))


def save(image, name, size=OUT):
    image.resize((size, size), Image.LANCZOS).save(HERE / "icons" / f"{name}.png")


def line(draw, points, fill=BLACK, width=7):
    draw.line(points, fill=fill, width=width, joint="curve")
    r = width / 2
    for x, y in (points[0], points[-1]):
        draw.ellipse((x - r, y - r, x + r, y + r), fill=fill)


def plus():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(48, 20), (48, 76)]); line(d, [(20, 48), (76, 48)])
    return im


def assistant():
    im = canvas(); d = ImageDraw.Draw(im)
    d.polygon([(48, 12), (58, 38), (84, 48), (58, 58), (48, 84), (38, 58), (12, 48), (38, 38)], fill=BLACK)
    return im


def grid():
    im = canvas(); d = ImageDraw.Draw(im)
    for x in (16, 52):
        for y in (16, 52):
            d.rounded_rectangle((x, y, x + 28, y + 28), radius=6, outline=BLACK, width=6)
    return im


def calendar():
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((16, 24, 80, 80), radius=8, outline=BLACK, width=6)
    line(d, [(16, 40), (80, 40)], width=6)
    line(d, [(32, 14), (32, 30)], width=6); line(d, [(64, 14), (64, 30)], width=6)
    return im


def library():
    im = canvas(); d = ImageDraw.Draw(im)
    for x in (20, 38, 56):
        d.rounded_rectangle((x, 22, x + 12, 76), radius=3, outline=BLACK, width=5)
    d.line((70, 24, 82, 22), fill=BLACK, width=6)
    d.line((72, 26, 84, 74), fill=BLACK, width=6)
    return im


def search():
    im = canvas(); d = ImageDraw.Draw(im)
    d.ellipse((16, 16, 62, 62), outline=BLACK, width=7)
    line(d, [(58, 58), (82, 82)], width=7)
    return im


def share():
    im = canvas(); d = ImageDraw.Draw(im)
    d.arc((18, 40, 78, 88), 180, 360, fill=BLACK, width=6)
    line(d, [(52, 58), (52, 14)], width=6)
    line(d, [(34, 30), (52, 12), (70, 30)], width=6)
    return im


def copy():
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((20, 20, 64, 64), radius=6, outline=BLACK, width=6)
    d.rounded_rectangle((36, 36, 78, 78), radius=6, outline=BLACK, width=6)
    return im


def expand():
    im = canvas(); d = ImageDraw.Draw(im)
    for sx, sy, dx, dy in ((18, 18, 38, 38), (78, 18, 58, 38), (18, 78, 38, 58), (78, 78, 58, 58)):
        line(d, [(sx, sy), (dx, dy)], width=6)
        line(d, [(sx, sy), (sx + (8 if sx < 48 else -8), sy)], width=6)
        line(d, [(sx, sy), (sx, sy + (8 if sy < 48 else -8))], width=6)
    return im


def collapse():
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((14, 18, 82, 78), radius=8, outline=BLACK, width=6)
    line(d, [(36, 18), (36, 78)], width=6)
    line(d, [(54, 48), (70, 48)], width=6)
    line(d, [(62, 40), (54, 48), (62, 56)], width=6)
    return im


def close():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(24, 24), (72, 72)]); line(d, [(72, 24), (24, 72)])
    return im


def code():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(38, 26), (18, 48), (38, 70)], width=6)
    line(d, [(58, 26), (78, 48), (58, 70)], width=6)
    line(d, [(52, 18), (44, 78)], width=6)
    return im


def send():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(48, 76), (48, 20)], width=7)
    line(d, [(28, 40), (48, 20), (68, 40)], width=7)
    return im


def stop():
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((28, 28, 68, 68), radius=8, fill=BLACK)
    return im


def menu():
    im = canvas(); d = ImageDraw.Draw(im)
    for y in (28, 48, 68):
        line(d, [(22, y), (74, y)], width=6)
    return im


def filter():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(18, 24), (78, 24)], width=6)
    line(d, [(30, 46), (66, 46)], width=6)
    line(d, [(42, 68), (54, 68)], width=6)
    return im


def user():
    im = canvas(); d = ImageDraw.Draw(im)
    d.ellipse((34, 18, 62, 46), outline=BLACK, width=6)
    d.arc((20, 48, 76, 92), 180, 360, fill=BLACK, width=6)
    return im


def chevron_down():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(28, 38), (48, 58), (68, 38)], width=7)
    return im


def chevron_right():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(38, 28), (58, 48), (38, 68)], width=7)
    return im


def check():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(20, 50), (40, 70), (78, 28)], width=8)
    return im


def ring():
    im = canvas(); d = ImageDraw.Draw(im)
    d.arc((18, 18, 78, 78), 0, 360, fill=BLACK, width=6)
    d.arc((18, 18, 78, 78), 300, 360, fill=GREEN, width=6)
    return im


def logo():
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((22, 22, 74, 74), radius=18, fill=GREEN)
    d.ellipse((40, 40, 56, 56), fill=(255, 255, 255, 235))
    return im


def agent():
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((16, 24, 80, 76), radius=16, fill=GREEN)
    d.ellipse((32, 42, 44, 54), fill=DARK)
    d.ellipse((52, 42, 64, 54), fill=DARK)
    d.arc((38, 52, 58, 66), 0, 180, fill=DARK, width=4)
    line(d, [(48, 24), (48, 12)], fill=GREEN, width=6)
    d.ellipse((42, 6, 54, 18), fill=GREEN)
    return im


def mascot():
    im = Image.new("RGBA", (240, 240), (0, 0, 0, 0)); d = ImageDraw.Draw(im)
    d.rounded_rectangle((36, 52, 204, 208), radius=48, fill=GREEN)
    d.ellipse((78, 102, 108, 132), fill=DARK)
    d.ellipse((132, 102, 162, 132), fill=DARK)
    d.arc((96, 124, 144, 168), 0, 180, fill=DARK, width=8)
    d.line((120, 52, 120, 24), fill=GREEN, width=12)
    d.ellipse((104, 8, 136, 40), fill=GREEN)
    d.ellipse((112, 16, 128, 32), fill=(255, 255, 255, 230))
    return im


def main():
    (HERE / "icons").mkdir(exist_ok=True)
    icons = {
        "plus": plus(), "assistant": assistant(), "grid": grid(), "calendar": calendar(),
        "library": library(), "search": search(), "share": share(), "copy": copy(),
        "expand": expand(), "collapse": collapse(), "close": close(), "code": code(),
        "send": send(), "stop": stop(), "menu": menu(), "filter": filter(),
        "user": user(), "chevron-down": chevron_down(), "chevron-right": chevron_right(),
        "check": check(), "ring": ring(), "logo": logo(), "agent": agent(),
    }
    for name, image in icons.items():
        save(image, name)
    mascot().resize((120, 120), Image.LANCZOS).save(HERE / "mascot.png")
    print(f"wrote {len(icons)} icons + mascot to {HERE}")


if __name__ == "__main__":
    main()
