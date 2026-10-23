"""Draws WinDict's icon in the Windows 11 (Fluent) style: a book, no tile."""
import sys
from PIL import Image, ImageDraw, ImageFilter, ImageFont, ImageChops

FONT = r"C:\Windows\Fonts\seguisb.ttf"  # Segoe UI Semibold


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def gradient(size, top, bottom, diagonal=True):
    """A gradient from the top left (or top) to the bottom right (or bottom)."""
    w, h = size
    g = Image.new("RGBA", size)
    px = g.load()
    for y in range(h):
        for x in range(w):
            t = ((x / w + y / h) / 2) if diagonal else y / h
            px[x, y] = lerp(top, bottom, t) + (255,)
    return g


def gradient_fast(size, top, bottom, diagonal=True):
    # Build small then upscale: smooth gradients don't need full resolution.
    small = gradient((64, 64), top, bottom, diagonal)
    return small.resize(size, Image.BICUBIC)


def shape(size, box, radius):
    m = Image.new("L", size, 0)
    ImageDraw.Draw(m).rounded_rectangle(box, radius, fill=255)
    return m


def paint(canvas, fill, mask):
    canvas.alpha_composite(Image.composite(fill, Image.new("RGBA", canvas.size), mask))


def draw(S, letters=True):
    """The icon at S×S (draw it large and downscale for antialiasing)."""
    u = S / 256  # design units: a 256 grid
    size = (S, S)
    img = Image.new("RGBA", size)

    left, right, top, bottom = 40 * u, 216 * u, 22 * u, 232 * u
    r = 18 * u
    pages_h = 26 * u

    # Shadow under the book.
    sh = shape(size, (left + 2 * u, top + 6 * u, right + 2 * u, bottom + 6 * u), r)
    sh = sh.filter(ImageFilter.GaussianBlur(8 * u)).point(lambda v: v * 0.35)
    img.alpha_composite(Image.merge("RGBA", [Image.new("L", size, 0)] * 3 + [sh]))
    sh2 = shape(size, (left, top + 2 * u, right, bottom + 2 * u), r)
    sh2 = sh2.filter(ImageFilter.GaussianBlur(2 * u)).point(lambda v: v * 0.25)
    img.alpha_composite(Image.merge("RGBA", [Image.new("L", size, 0)] * 3 + [sh2]))

    # Back cover: the whole book, darker.
    paint(img, gradient_fast(size, (0x0A, 0x4F, 0xA6), (0x06, 0x36, 0x7A)),
          shape(size, (left, top, right, bottom), r))

    # Pages, showing at the bottom between the covers.
    pl, pr = left + 14 * u, right - 8 * u
    pt, pb = bottom - pages_h - 6 * u, bottom - 8 * u
    paint(img, gradient_fast(size, (0xFF, 0xFF, 0xFF), (0xD6, 0xDE, 0xE8), diagonal=False),
          shape(size, (pl, pt, pr, pb), 4 * u))
    # Lines along the page edges.
    d = ImageDraw.Draw(img)
    for i in (1, 2):
        y = pt + (pb - pt) * i / 3
        d.line((pl + 2 * u, y, pr - 2 * u, y), fill=(0xC2, 0xCC, 0xD8, 255), width=max(1, round(1.5 * u)))

    # Front cover.
    fb = bottom - pages_h - 2 * u
    paint(img, gradient_fast(size, (0x4C, 0xB4, 0xFF), (0x00, 0x67, 0xC0)),
          shape(size, (left, top, right, fb), r))
    # Spine: a darker band on the left.
    spine = shape(size, (left, top, right, fb), r)
    spine = ImageChops.multiply(spine, shape(size, (0, 0, left + 22 * u, S), 0))
    paint(img, Image.new("RGBA", size, (0x00, 0x4E, 0x9A, 140)), spine)
    # A soft highlight along the top edge.
    hl = ImageChops.subtract(shape(size, (left, top, right, fb), r),
                             shape(size, (left, top + 3 * u, right, fb + 3 * u), r))
    paint(img, Image.new("RGBA", size, (255, 255, 255, 90)), hl)

    if letters:
        font = ImageFont.truetype(FONT, round(96 * u))
        text = "Aa"
        bbox = font.getbbox(text)
        cx = (left + 22 * u + right) / 2
        cy = (top + fb) / 2
        x = cx - (bbox[0] + bbox[2]) / 2
        y = cy - (bbox[1] + bbox[3]) / 2
        glow = Image.new("L", size, 0)
        ImageDraw.Draw(glow).text((x, y + 2 * u), text, font=font, fill=255)
        glow = glow.filter(ImageFilter.GaussianBlur(3 * u)).point(lambda v: v * 0.45)
        paint(img, Image.new("RGBA", size, (0, 0x2A, 0x60, 255)), glow)
        ImageDraw.Draw(img).text((x, y), text, font=font, fill=(255, 255, 255, 255))
    return img


def render(px, letters=True):
    return draw(px * 8, letters).resize((px, px), Image.LANCZOS)


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "assets"
    sizes = [16, 20, 24, 32, 40, 48, 64, 256]
    imgs = {s: render(s, letters=s >= 20) for s in sizes}
    imgs[256].save(out + "/windict.ico", sizes=[(s, s) for s in sizes],
                   append_images=[imgs[s] for s in sizes if s != 256])
    draw(1024 * 2).resize((1024, 1024), Image.LANCZOS).save(out + "/windict.png")
