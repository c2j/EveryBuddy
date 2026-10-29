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
# Measured from the artifact-card capture: the file badge is a #7A95F8 rounded
# square with a white glyph.
ARTIFACT_BLUE = (122, 149, 248, 255)
WHITE = (255, 255, 255, 245)


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


def folder():
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((12, 28, 84, 80), radius=8, outline=BLACK, width=6)
    d.line((12, 42, 44, 42), fill=BLACK, width=6)
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


def search_dim():
    """The pill's magnifier: measured #797979 over white ≈ 52% black."""
    im = canvas(); d = ImageDraw.Draw(im)
    gray = (0, 0, 0, 134)
    d.ellipse((16, 16, 62, 62), outline=gray, width=7)
    line(d, [(58, 58), (82, 82)], fill=gray, width=7)
    return im


def share():
    """The topbar share glyph (2026-10-24 capture): a curved forward arrow —
    tail at the bottom, sweeping left and over the top, head pointing right."""
    im = canvas(); d = ImageDraw.Draw(im)
    d.arc((16, 20, 64, 88), 90, 270, fill=BLACK, width=6)
    # The arc ends at the top of the box travelling right; the head is an
    # open chevron continuing in that direction.
    line(d, [(44, 6), (64, 20), (42, 36)], width=6)
    return im


def panel_right():
    """The right-panel toggle (2026-10-24 capture): a rounded rectangle whose
    right rail is a filled capsule."""
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((14, 22, 82, 74), radius=10, outline=BLACK, width=6)
    d.rounded_rectangle((60, 32, 71, 64), radius=5, fill=BLACK)
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


def pencil():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(32, 68), (70, 30)], width=12)
    line(d, [(20, 80), (32, 68)], width=5)
    line(d, [(60, 20), (78, 38)], width=6)
    return im


def regenerate():
    im = canvas(); d = ImageDraw.Draw(im)
    d.arc((20, 20, 76, 76), 75, 375, fill=BLACK, width=7)
    # Arrowhead where the arc ends (75 deg ≈ top-right of the circle).
    d.polygon([(74, 34), (86, 40), (72, 48)], fill=BLACK)
    return im


def like():
    im = canvas(); d = ImageDraw.Draw(im)
    # Wrist block, then a filled hand with the thumb raised.
    d.rounded_rectangle((16, 46, 32, 80), radius=5, fill=BLACK)
    d.polygon(
        [(38, 78), (38, 48), (50, 44), (56, 20), (66, 20), (70, 44), (82, 48), (80, 72), (70, 78)],
        fill=BLACK,
    )
    return im


def dislike():
    return like().rotate(180)


def send():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(48, 76), (48, 20)], width=7)
    line(d, [(28, 40), (48, 20), (68, 40)], width=7)
    return im


def stop():
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((28, 28, 68, 68), radius=8, fill=BLACK)
    return im


def pin():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(48, 16), (48, 46)], width=6)
    d.ellipse((30, 34, 66, 70), outline=BLACK, width=6)
    return im


def send_light():
    im = canvas(); d = ImageDraw.Draw(im)
    white = (255, 255, 255, 235)
    d.line((48, 76, 48, 20), fill=white, width=7, joint="curve")
    d.line((28, 40, 48, 20, 68, 40), fill=white, width=7, joint="curve")
    r = 3.5
    for x, y in ((48, 76), (48, 20), (28, 40), (68, 40)):
        d.ellipse((x - r, y - r, x + r, y + r), fill=white)
    return im


def stop_light():
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((28, 28, 68, 68), radius=8, fill=(255, 255, 255, 235))
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


def chevron_up():
    im = canvas(); d = ImageDraw.Draw(im)
    line(d, [(28, 58), (48, 38), (68, 58)], width=7)
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


def task_failed():
    """Unseen-failure mark for a sidebar task row.

    Traced from the product's `TaskFailedIcon`: a 16-unit box whose dark-theme
    layer (`__solid`) is a disc of radius 6.5 filled #f64041 with the
    exclamation knocked out to transparency - stem 1.2 wide from y 4.5 to 9.0,
    dot from y 10.3 to 11.5, both centred on x 8 (ui-spec §10-40). Scaled x6
    onto this 96px canvas."""
    im = canvas(); d = ImageDraw.Draw(im)
    red = (246, 64, 65, 255)
    d.ellipse((9, 9, 87, 87), fill=red)
    d.rectangle((44, 27, 50, 54), fill=(0, 0, 0, 0))
    d.rectangle((44, 62, 50, 69), fill=(0, 0, 0, 0))
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


def artifact_code():
    """Blue badge with a white </> glyph: a file whose extension is code."""
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((8, 8, 88, 88), radius=24, fill=ARTIFACT_BLUE)
    d.line((40, 34, 26, 48, 40, 62), fill=WHITE, width=6, joint="curve")
    d.line((56, 34, 70, 48, 56, 62), fill=WHITE, width=6, joint="curve")
    d.line((52, 28, 44, 68), fill=WHITE, width=5)
    return im


def artifact_unknown():
    """Blue badge with a white ? glyph: any other file."""
    im = canvas(); d = ImageDraw.Draw(im)
    d.rounded_rectangle((8, 8, 88, 88), radius=24, fill=ARTIFACT_BLUE)
    d.arc((34, 26, 62, 52), 200, 360, fill=WHITE, width=6)
    d.arc((34, 26, 62, 52), 0, 30, fill=WHITE, width=6)
    line(d, [(57, 49), (48, 57)], fill=WHITE, width=6)
    d.ellipse((43, 64, 53, 74), fill=WHITE)
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


def appicon(size=1024):
    """The application icon: assets/logo.png scaled onto a square.

    The running window, the Dock, the Linux hicolor icon and the Windows .exe
    resource all carry this mark. The in-app agent glyph stays `logo()`.
    """
    source = HERE.parents[2] / "assets" / "logo.png"
    image = Image.open(source).convert("RGBA")
    if image.size != (size, size):
        image = image.resize((size, size), Image.LANCZOS)
    return image


def main():
    (HERE / "icons").mkdir(exist_ok=True)
    icons = {
        "plus": plus(), "assistant": assistant(), "grid": grid(), "calendar": calendar(),
        "library": library(), "folder": folder(), "search": search(), "search-dim": search_dim(),
        "share": share(), "panel-right": panel_right(), "copy": copy(),
        "expand": expand(), "collapse": collapse(), "close": close(), "code": code(),
        "pin": pin(), "send": send(), "send-light": send_light(), "stop": stop(), "stop-light": stop_light(), "menu": menu(), "filter": filter(),
        "user": user(), "chevron-up": chevron_up(), "chevron-down": chevron_down(), "chevron-right": chevron_right(),
        "check": check(), "ring": ring(), "logo": logo(), "agent": agent(),
        "pencil": pencil(), "regenerate": regenerate(), "like": like(), "dislike": dislike(),
        "artifact-code": artifact_code(), "artifact-unknown": artifact_unknown(),
        "task-failed": task_failed(),
    }
    for name, image in icons.items():
        save(image, name)
    mascot().resize((120, 120), Image.LANCZOS).save(HERE / "mascot.png")
    icon = appicon(1024)
    icon.save(HERE / "appicon.png")
    packaging = HERE.parents[2] / "packaging"
    icon.save(
        packaging / "evobuddy.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    icon.resize((256, 256), Image.LANCZOS).save(packaging / "evobuddy.png")
    print(f"wrote {len(icons)} icons + app icon to {HERE}")


if __name__ == "__main__":
    main()
