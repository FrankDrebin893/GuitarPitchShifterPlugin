#!/usr/bin/env python3
"""Makes the artwork under crates/suite_common/assets/art/: a two-ink screen print for each
pedal that has art. Run it again after changing a motif; the PNGs are committed.

Each file is RGBA and mostly transparent. The art is strongest at the edges and fades
out towards the middle, where the controls are. The loader scales it to cover the top
face of the enclosure, so the size here only sets how sharp it is.

To use a painted or generated picture instead, put a PNG at the same path and leave this
script alone: nothing else reads it.
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

OUT = Path(__file__).resolve().parent.parent / "crates/suite_common/assets/art"
CREAM = np.array([234, 223, 200], dtype=np.float32)


def smooth(a, b, x):
    t = np.clip((x - a) / (b - a), 0, 1)
    return t * t * (3 - 2 * t)


def grid(w, h):
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    return x, y


def halftone(field, cell, angle_deg, offset=(0.0, 0.0)):
    """Round dots on a rotated grid; a dot's size follows `field` (0..1). Returns coverage."""
    h, w = field.shape
    x, y = grid(w, h)
    a = np.radians(angle_deg)
    u = (x - offset[0]) * np.cos(a) + (y - offset[1]) * np.sin(a)
    v = -(x - offset[0]) * np.sin(a) + (y - offset[1]) * np.cos(a)
    fu = (u / cell) - np.floor(u / cell) - 0.5
    fv = (v / cell) - np.floor(v / cell) - 0.5
    dist = np.sqrt(fu * fu + fv * fv) * cell
    radius = np.sqrt(np.clip(field, 0, 1)) * cell * 0.72
    return np.clip(radius - dist + 0.5, 0, 1)


def noise(w, h, scale, seed):
    rng = np.random.default_rng(seed)
    small = rng.random((max(2, int(h / scale)), max(2, int(w / scale)))).astype(np.float32)
    img = Image.fromarray((small * 255).astype(np.uint8)).resize((w, h), Image.BICUBIC)
    return np.asarray(img, dtype=np.float32) / 255


def edges(w, h, inner=0.55):
    """1 at the border, 0 in the middle, where the controls are"""
    x, y = grid(w, h)
    dx = np.abs(x / w - 0.5) * 2
    dy = np.abs(y / h - 0.5) * 2
    d = np.maximum(dx, dy ** 1.2)
    return smooth(inner, 1.0, d)


def ink(layer, color, strength, w, h, seed):
    """An ink's coverage laid down on paper: uneven, with specks missing"""
    paper = noise(w, h, 3, seed) * 0.5 + noise(w, h, 40, seed + 1) * 0.5
    dropout = 1 - 0.55 * smooth(0.62, 0.8, paper)
    alpha = np.clip(layer * strength * dropout, 0, 1)
    rgb = np.broadcast_to(color, (h, w, 3))
    return rgb, alpha


def compose(layers, w, h):
    out_rgb = np.zeros((h, w, 3), np.float32)
    out_a = np.zeros((h, w), np.float32)
    for rgb, alpha in layers:
        a = alpha[..., None]
        out_rgb = rgb * a + out_rgb * (1 - a)
        out_a = alpha + out_a * (1 - alpha)
    return out_rgb, out_a


def bolt_polygon(cx, cy, size, tilt):
    pts = [(0.15, -1.0), (-0.55, 0.12), (-0.08, 0.12), (-0.3, 1.0), (0.6, -0.2), (0.1, -0.2), (0.45, -1.0)]
    c, s = np.cos(tilt), np.sin(tilt)
    return [(cx + (x * c - y * s) * size, cy + (x * s + y * c) * size) for x, y in pts]


def hatch(mask, spacing, angle_deg, width):
    h, w = mask.shape
    x, y = grid(w, h)
    a = np.radians(angle_deg)
    t = (x * np.cos(a) + y * np.sin(a)) / spacing
    line = np.abs(t - np.floor(t) - 0.5) * spacing
    return np.clip((width / 2 - (spacing / 2 - line)) + 0.5, 0, 1) * mask


def shadeof(paint, depth):
    return np.array(paint, np.float32) * (1 - depth)


def tint(paint, share):
    return np.array(paint, np.float32) * (1 - share) + CREAM * share


def star_polygon(cx, cy, r, squash=0.28):
    """A four-pointed glint"""
    return [(cx, cy - r), (cx + r * squash, cy - r * squash), (cx + r, cy), (cx + r * squash, cy + r * squash),
            (cx, cy + r), (cx - r * squash, cy + r * squash), (cx - r, cy), (cx - r * squash, cy - r * squash)]


def mask_of(draw_fn, w, h, blur=0.8):
    img = Image.new("L", (w, h), 0)
    draw_fn(ImageDraw.Draw(img))
    return img, np.asarray(img.filter(ImageFilter.GaussianBlur(blur)), dtype=np.float32) / 255


def outline_of(img, mask, grow=7):
    return np.clip(np.asarray(img.filter(ImageFilter.MaxFilter(grow)), dtype=np.float32) / 255 - mask, 0, 1)


def pitch_shifter(w=800, h=1120):
    """Ascent: rings spreading from the lower corner, and rising chevrons up the left side"""
    paint = (222, 86, 30)
    x, y = grid(w, h)
    d = np.sqrt((x - w * 0.95) ** 2 + (y - h * 1.04) ** 2)
    rings = smooth(0.25, 0.75, 0.5 + 0.5 * np.cos(d / 30.0))
    field_a = rings * smooth(h * 1.15, h * 0.3, d) * edges(w, h, 0.3)
    a = halftone(field_a, 12, 15)

    def chevrons(draw):
        for i in range(7):
            top = h * (0.80 - i * 0.105)
            for cx in (w * 0.1, w * 0.9):
                draw.line([(cx - 52, top + 40), (cx, top), (cx + 52, top + 40)], fill=255, width=20)
    img, mask = mask_of(chevrons, w, h)
    fade = np.clip(1.0 - np.linspace(0, 1, h)[:, None], 0.25, 1.0) ** 1.2
    layers = [
        ink(a, tint(paint, 0.42), 0.62, w, h, 1),
        ink(np.roll(hatch(mask, 5, 40, 2.2) * fade, (3, 4), axis=(0, 1)), shadeof(paint, 0.55), 0.7, w, h, 7),
        ink(mask * 0.45 * fade, tint(paint, 0.5), 0.6, w, h, 9),
    ]
    return compose(layers, w, h)


def drums(w=1320, h=1200):
    """Skin: a drum head's rings from below, and crossed sticks in each upper corner"""
    paint = (29, 130, 130)
    x, y = grid(w, h)
    d = np.sqrt((x - w * 0.5) ** 2 + (y - h * 1.15) ** 2)
    rings = smooth(0.2, 0.8, 0.5 + 0.5 * np.cos(d / 28.0))
    a = halftone(rings * smooth(h * 1.3, h * 0.5, d) * edges(w, h, 0.35), 12, 30)

    def sticks(draw):
        for cx, sign in ((w * 0.13, 1), (w * 0.87, -1)):
            for tilt in (0.55, -0.55):
                dx, dy = np.sin(tilt) * 200, np.cos(tilt) * 200
                draw.line([(cx - dx * sign, h * 0.2 - dy), (cx + dx * sign, h * 0.2 + dy)], fill=255, width=28)
                draw.ellipse([cx + dx * sign - 30, h * 0.2 + dy - 30, cx + dx * sign + 30, h * 0.2 + dy + 30], fill=255)
    img, mask = mask_of(sticks, w, h)
    layers = [
        ink(a, tint(paint, 0.4), 0.6, w, h, 21),
        ink(np.roll(np.maximum(hatch(mask, 7, 55, 2.8), outline_of(img, mask)), (3, 4), axis=(0, 1)),
            shadeof(paint, 0.6), 0.7, w, h, 23),
        ink(mask * 0.5, tint(paint, 0.5), 0.55, w, h, 25),
    ]
    return compose(layers, w, h)


def bass(w=1320, h=1200):
    """Strings: slow standing waves across the face, and inlay dots down each side"""
    paint = (30, 64, 156)
    x, y = grid(w, h)
    wave = 0.5 + 0.5 * np.cos(y / 15.0 + 2.6 * np.sin(x / 170.0) + 1.3 * np.sin(x / 61.0 + y / 400.0))
    a = halftone(smooth(0.3, 0.9, wave) * edges(w, h, 0.3) * 1.1, 11, 0)

    def inlays(draw):
        for cx in (w * 0.07, w * 0.93):
            for i, row in enumerate((0.22, 0.38, 0.54, 0.70)):
                r = 26 if i != 2 else 38
                draw.ellipse([cx - r, h * row - r, cx + r, h * row + r], fill=255)
    img, mask = mask_of(inlays, w, h)
    layers = [
        ink(a, tint(paint, 0.4), 0.6, w, h, 31),
        ink(np.roll(np.maximum(hatch(mask, 5, 20, 2.0), outline_of(img, mask, 5)), (3, 4), axis=(0, 1)),
            shadeof(paint, 0.6), 0.7, w, h, 33),
        ink(mask * 0.5, tint(paint, 0.55), 0.6, w, h, 35),
    ]
    return compose(layers, w, h)


def pad(w=1320, h=1200):
    """Drift: slow clouds in big dots, and a scatter of small stars"""
    paint = (96, 44, 120)
    cloud = noise(w, h, 260, 41) * 0.55 + noise(w, h, 110, 42) * 0.3 + noise(w, h, 45, 43) * 0.15
    a = halftone(smooth(0.35, 0.75, cloud) * edges(w, h, 0.3), 15, 45)
    rng = np.random.default_rng(44)

    def stars(draw):
        for _ in range(46):
            cx, cy = rng.random() * w, rng.random() * h
            if abs(cx / w - 0.5) < 0.3 and abs(cy / h - 0.5) < 0.35:
                continue
            draw.polygon(star_polygon(cx, cy, 10 + rng.random() * 22), fill=255)
    img, mask = mask_of(stars, w, h, 0.6)
    layers = [
        ink(a, tint(paint, 0.4), 0.6, w, h, 45),
        ink(np.roll(mask, (2, 3), axis=(0, 1)), shadeof(paint, 0.55), 0.65, w, h, 47),
        ink(mask * 0.8, tint(paint, 0.65), 0.7, w, h, 49),
    ]
    return compose(layers, w, h)


def bells(w=1320, h=1200):
    """Glass: bell outlines hung from the top corners, and glints around them"""
    paint = (28, 104, 60)
    x, y = grid(w, h)
    d = np.sqrt((x - w * 0.5) ** 2 + (y - h * 1.2) ** 2)
    a = halftone(smooth(0.3, 0.9, 0.5 + 0.5 * np.cos(d / 36.0)) * smooth(h * 1.4, h * 0.4, d)
                 * edges(w, h, 0.35), 13, 60)

    def shapes(draw):
        for cx in (w * 0.1, w * 0.9):
            draw.ellipse([cx - 78, h * 0.10, cx + 78, h * 0.36], fill=255)
            draw.polygon([(cx - 76, h * 0.25), (cx + 76, h * 0.25), (cx + 112, h * 0.40), (cx - 112, h * 0.40)], fill=255)
            draw.ellipse([cx - 112, h * 0.385, cx + 112, h * 0.425], fill=255)
            draw.ellipse([cx - 14, h * 0.065, cx + 14, h * 0.115], fill=255)
            draw.ellipse([cx - 18, h * 0.425, cx + 18, h * 0.425 + 36], fill=255)
            for gx, gy, r in ((0.65, 0.18, 34), (0.55, 0.52, 20), (-0.55, 0.50, 26), (-0.62, 0.2, 16)):
                draw.polygon(star_polygon(cx + gx * 160, h * gy, r), fill=255)
    img, mask = mask_of(shapes, w, h)
    layers = [
        ink(a, tint(paint, 0.4), 0.6, w, h, 51),
        ink(np.roll(np.maximum(hatch(mask, 6, 60, 2.2), outline_of(img, mask)), (3, 4), axis=(0, 1)),
            shadeof(paint, 0.6), 0.7, w, h, 53),
        ink(mask * 0.5, tint(paint, 0.55), 0.55, w, h, 55),
    ]
    return compose(layers, w, h)


def amp(w=1920, h=708):
    """Storm: rays from below the head, and a bolt struck on each side"""
    paint = (124, 28, 32)
    x, y = grid(w, h)
    theta = np.arctan2(x - w * 0.5, (h * 1.9) - y)
    rays = 0.5 + 0.5 * np.cos(theta * 46)
    rays = smooth(0.35, 0.95, rays)
    dist = np.sqrt((x - w * 0.5) ** 2 + ((h * 1.9) - y) ** 2)
    field_a = rays * smooth(h * 2.25, h * 1.1, dist) * edges(w, h, 0.4)
    a = halftone(field_a, 10, 45)

    img = Image.new("L", (w, h), 0)
    draw = ImageDraw.Draw(img)
    for cx, tilt in ((w * 0.075, 0.18), (w * 0.925, -0.18)):
        draw.polygon(bolt_polygon(cx, h * 0.5, h * 0.42, tilt), fill=255)
    mask = np.asarray(img.filter(ImageFilter.GaussianBlur(0.8)), dtype=np.float32) / 255
    bolts = hatch(mask, 6, 30, 2.4) * 0.9
    outline = np.clip(
        np.asarray(img.filter(ImageFilter.MaxFilter(7)), dtype=np.float32) / 255 - mask, 0, 1
    )
    layers = [
        ink(a, tint(paint, 0.38), 0.6, w, h, 11),
        ink(np.roll(np.maximum(bolts, outline), (3, 4), axis=(0, 1)), shadeof(paint, 0.6), 0.7, w, h, 13),
        ink(mask * 0.5, tint(paint, 0.55), 0.55, w, h, 17),
    ]
    return compose(layers, w, h)


def mini(w=440, h=1000):
    """One print for every mini pedal, in neutral inks (cream and shadow) so any paint takes it:
    halftone rays from the foot of the pedal, and a hatched band along each side"""
    x, y = grid(w, h)
    theta = np.arctan2(x - w * 0.5, h * 1.25 - y)
    rays = smooth(0.4, 0.95, 0.5 + 0.5 * np.cos(theta * 22))
    dist = np.sqrt((x - w * 0.5) ** 2 + (h * 1.25 - y) ** 2)
    a = halftone(rays * smooth(h * 1.5, h * 0.6, dist) * edges(w, h, 0.25), 9, 45)
    side = smooth(0.16, 0.0, np.minimum(x, w - x) / w) * smooth(0.0, 0.2, y / h)
    b = hatch(side, 5, 35, 1.8)
    layers = [
        ink(a, CREAM, 0.30, w, h, 61),
        ink(np.roll(b, (2, 3), axis=(0, 1)), np.array([10, 8, 8], np.float32), 0.30, w, h, 63),
    ]
    return compose(layers, w, h)


ART = {"mini": mini, "pitch_shifter": pitch_shifter, "amp": amp, "drums": drums, "bass": bass, "pad": pad, "bells": bells}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name in sys.argv[1:] or ART:
        rgb, alpha = ART[name]()
        # PNG holds straight (not premultiplied) colour
        straight = np.where(alpha[..., None] > 1e-4, rgb / np.maximum(alpha[..., None], 1e-4), 0)
        data = np.dstack([np.clip(straight, 0, 255), np.clip(alpha * 255, 0, 255)]).astype(np.uint8)
        Image.fromarray(data, "RGBA").save(OUT / f"{name}.png", optimize=True)
        print(name, data.shape)


if __name__ == "__main__":
    main()
