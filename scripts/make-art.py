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


def contour_lines(field, levels, width):
    """Thin lines where `field` crosses each level, like an engraver's contour of a cloud"""
    out = np.zeros_like(field)
    for level in levels:
        out = np.maximum(out, np.clip(1 - np.abs(field - level) / width, 0, 1))
    return out


def fork(draw, rng, x, y, angle, length, width, depth):
    """A forked lightning bolt: hard zig-zags that keep heading one way, splitting into
    thinner branches"""
    pts = [(x, y)]
    side = 1
    base = angle
    for _ in range(int(length / 38)):
        side = -side
        step = rng.uniform(26, 54)
        a = base + side * rng.uniform(0.35, 0.75)
        x, y = x + np.sin(a) * step, y + np.cos(a) * step
        pts.append((x, y))
    draw.line(pts, fill=255, width=max(3, int(width)), joint="curve")
    if depth > 0:
        for i in range(2, len(pts) - 3, 3):
            if rng.random() < 0.65:
                fork(draw, rng, pts[i][0], pts[i][1], base + rng.choice([-1, 1]) * rng.uniform(0.5, 0.9),
                     length * 0.4, width * 0.55, depth - 1)


def amp(w=1920, h=708):
    """Storm: inked clouds with contour lines, forked lightning down each side, and a hatched
    ridge of mountains along the foot of the head"""
    paint = (124, 28, 32)
    x, y = grid(w, h)
    rng = np.random.default_rng(7)
    edge = edges(w, h, 0.42)

    # Clouds: two noise fields, a body in halftone and contour lines on top
    cloud = noise(w, h, 240, 71) * 0.55 + noise(w, h, 90, 72) * 0.3 + noise(w, h, 34, 73) * 0.15
    cloud_field = smooth(0.38, 0.72, cloud) * edge
    body = halftone(cloud_field * 0.9, 10, 45)
    lines = contour_lines(cloud, (0.44, 0.52, 0.60), 0.012) * edge

    # Lightning: two forks from the top corners, with a wide soft glow under each
    img = Image.new("L", (w, h), 0)
    draw = ImageDraw.Draw(img)
    for cx, lean in ((w * 0.07, 0.25), (w * 0.93, -0.25)):
        fork(draw, rng, cx, -10, lean, h * 0.95, 13, 1)
    mask = np.asarray(img.filter(ImageFilter.GaussianBlur(0.7)), dtype=np.float32) / 255
    glow = np.asarray(img.filter(ImageFilter.GaussianBlur(14)), dtype=np.float32) / 255
    glow = np.clip(glow * 3.0, 0, 1) * edge

    # Mountains: a ridge from layered noise, hatched, with a lighter ridge line
    peaks = np.abs(np.sin(np.arange(w) / 61.0 + 1.3)) * 0.6 + np.abs(np.sin(np.arange(w) / 23.0)) * 0.25         + noise(w, 2, 120, 74)[0] * 0.5
    ridge_y = h * (0.93 - 0.17 * peaks)
    ground = smooth(-1.5, 1.5, y - ridge_y[None, :])
    ridge = np.clip(1 - np.abs(y - ridge_y[None, :]) / 1.6, 0, 1)
    foot = hatch(ground, 6, 62, 2.4) * smooth(0.0, 0.1, (x / w) * (1 - x / w)) * 1.0

    layers = [
        ink(glow, tint(paint, 0.5), 0.35, w, h, 79),
        ink(body, tint(paint, 0.38), 0.6, w, h, 81),
        ink(np.roll(lines, (2, 3), axis=(0, 1)), shadeof(paint, 0.55), 0.7, w, h, 83),
        ink(foot, shadeof(paint, 0.6), 0.65, w, h, 85),
        ink(ridge, tint(paint, 0.55), 0.7, w, h, 87),
        ink(np.maximum(mask, 0), CREAM, 0.85, w, h, 89),
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


def sketch(draw_fn, w, h, blur=0.7):
    """Draw with PIL: returns the picture, its mask, and a wide soft glow of it"""
    img = Image.new("L", (w, h), 0)
    draw_fn(ImageDraw.Draw(img))
    mask = np.asarray(img.filter(ImageFilter.GaussianBlur(blur)), dtype=np.float32) / 255
    glow = np.asarray(img.filter(ImageFilter.GaussianBlur(12)), dtype=np.float32) / 255
    return img, mask, np.clip(glow * 2.5, 0, 1)


def rim_of(img, mask, grow=5):
    """The outline just outside a drawn shape"""
    return np.clip(np.asarray(img.filter(ImageFilter.MaxFilter(grow)), dtype=np.float32) / 255 - mask, 0, 1)


def skyline(w, seed, base, height, sharp=61.0):
    """A ridge of peaks across the width, as y values"""
    peaks = (np.abs(np.sin(np.arange(w) / sharp + seed)) * 0.6
             + np.abs(np.sin(np.arange(w) / (sharp * 0.4))) * 0.25
             + noise(w, 2, 120, int(seed) + 70)[0] * 0.5)
    return base - height * peaks


def pitch_shifter(w=800, h=1120):
    """Ascent: a staircase climbing the left edge, and rings and rays from a sun in the corner"""
    paint = (222, 86, 30)
    x, y = grid(w, h)
    edge = edges(w, h, 0.34)
    d = np.sqrt((x - w * 0.95) ** 2 + (y - h * 0.04) ** 2)
    rings = smooth(0.2, 0.8, 0.5 + 0.5 * np.cos(d / 28.0)) * smooth(h * 0.95, h * 0.2, d) * edge
    th = np.arctan2(y - h * 0.04, x - w * 0.95)
    rays = smooth(0.4, 0.95, 0.5 + 0.5 * np.cos(th * 30)) * smooth(h * 1.1, h * 0.3, d) * edge * 0.8

    def stairs(draw):
        for i in range(7):
            x0, y0 = i * 16, h * 0.97 - i * 82
            draw.rectangle([x0, y0, x0 + 110, y0 + 82], fill=255)
        draw.ellipse([w * 0.95 - 80, h * 0.04 - 80, w * 0.95 + 80, h * 0.04 + 80], fill=255)
    img, mask, glow = sketch(stairs, w, h)
    layers = [
        ink(glow * edge, tint(paint, 0.5), 0.3, w, h, 91),
        ink(halftone(rings, 11, 15), tint(paint, 0.42), 0.6, w, h, 92),
        ink(halftone(rays, 8, 60), shadeof(paint, 0.5), 0.45, w, h, 93),
        ink(np.roll(hatch(mask, 6, 35, 2.4), (3, 4), axis=(0, 1)), shadeof(paint, 0.6), 0.7, w, h, 94),
        ink(rim_of(img, mask), CREAM, 0.7, w, h, 95),
    ]
    return compose(layers, w, h)


def drums(w=1320, h=1200):
    """Stage: spotlight beams from the top corners, crossed sticks and the rims of two toms"""
    paint = (29, 130, 130)
    x, y = grid(w, h)
    edge = edges(w, h, 0.34)
    beams = np.zeros((h, w), np.float32)
    for cx, lean in ((w * 0.1, 1.0), (w * 0.9, -1.0)):
        a = np.arctan2(y + 80, (x - cx) * lean + 1e-3)
        reach = smooth(h * 1.3, 0.0, np.sqrt((x - cx) ** 2 + (y + 80) ** 2))
        beams = np.maximum(beams, smooth(0.55, 0.95, 0.5 + 0.5 * np.cos((a - 1.0) * 5.0)) * reach)

    def kit(draw):
        for cx, sign in ((w * 0.14, 1), (w * 0.86, -1)):
            for tilt in (0.5, -0.5):
                dx, dy = np.sin(tilt) * 230, np.cos(tilt) * 230
                draw.line([(cx - dx * sign, h * 0.17 - dy), (cx + dx * sign, h * 0.17 + dy)], fill=255, width=30)
                draw.ellipse([cx + dx * sign - 34, h * 0.17 + dy - 34, cx + dx * sign + 34, h * 0.17 + dy + 34],
                             fill=255)
        for cx in (w * 0.12, w * 0.88):
            for r, wd in ((190, 14), (150, 5), (120, 5)):
                draw.ellipse([cx - r, h * 1.02 - r, cx + r, h * 1.02 + r], outline=255, width=wd)
            for i in range(10):
                ang = i * 0.314
                px, py = cx + np.cos(ang) * 170, h * 1.02 - abs(np.sin(ang)) * 170
                draw.ellipse([px - 7, py - 7, px + 7, py + 7], fill=255)
    img, mask, glow = sketch(kit, w, h)
    layers = [
        ink(halftone(beams * edge * 0.9, 12, 30), tint(paint, 0.45), 0.55, w, h, 101),
        ink(glow * edge, tint(paint, 0.5), 0.3, w, h, 102),
        ink(np.roll(hatch(mask, 7, 55, 2.8), (3, 4), axis=(0, 1)), shadeof(paint, 0.6), 0.7, w, h, 103),
        ink(rim_of(img, mask), CREAM, 0.7, w, h, 104),
    ]
    return compose(layers, w, h)


def bass(w=1320, h=1200):
    """Neck: a fretboard down each side with inlays, and plucked strings blurring across top and foot"""
    paint = (30, 64, 156)
    x, y = grid(w, h)
    edge = edges(w, h, 0.3)

    def neck(draw):
        for cx in (w * 0.075, w * 0.925):
            draw.rectangle([cx - 78, 0, cx + 78, h], fill=255)
    img, mask, _ = sketch(neck, w, h, 0.5)
    fret = np.zeros((h, w), np.float32)
    for i in range(12):
        fret = np.maximum(fret, np.clip(1 - np.abs(y - h * (0.03 + 0.08 * i)) / 2.4, 0, 1))
    for cx in (w * 0.075, w * 0.925):
        for k in range(1, 5):
            line = np.clip(1 - np.abs(x - (cx - 60 + k * 24)) / 1.4, 0, 1)
            fret = np.maximum(fret, line * (np.abs(x - cx) < 80))
    inlay = np.zeros((h, w), np.float32)
    for cx in (w * 0.075, w * 0.925):
        for row, r in ((0.2, 14), (0.4, 14), (0.55, 20), (0.74, 14)):
            inlay = np.maximum(inlay, np.clip(r - np.sqrt((x - cx) ** 2 + (y - h * row) ** 2) + 0.5, 0, 1))
    strings = np.zeros((h, w), np.float32)
    swing = smooth(0.05, 0.5, np.minimum(x, w - x) / w * 2)
    for band in (0.045, 0.955):
        for k in range(4):
            yy = h * band + (k - 1.5) * 18 + np.sin(x / (90 + k * 14) + k) * (6 + k * 3) * swing
            strings = np.maximum(strings, np.clip(1 - np.abs(y - yy) / (1.3 + k * 0.5), 0, 1))
    waves = smooth(0.4, 1.0, 0.5 + 0.5 * np.cos(y / 14.0 + 2 * np.sin(x / 130.0))) * edge * 0.8 * (1 - mask)
    layers = [
        ink(mask * 0.55, shadeof(paint, 0.55), 0.5, w, h, 111),
        ink(fret * mask, tint(paint, 0.55), 0.7, w, h, 112),
        ink(inlay, CREAM, 0.8, w, h, 113),
        ink(halftone(waves, 11, 0), tint(paint, 0.4), 0.45, w, h, 114),
        ink(strings, CREAM, 0.85, w, h, 115),
    ]
    return compose(layers, w, h)


def pad(w=1320, h=1200):
    """Night: a cratered moon, a veil of aurora, drifting stars and a far ridge"""
    paint = (96, 44, 120)
    x, y = grid(w, h)
    edge = edges(w, h, 0.32)
    rng = np.random.default_rng(121)
    mx, my, mr = w * 0.88, h * 0.17, 110
    moon = np.clip(mr - np.sqrt((x - mx) ** 2 + (y - my) ** 2) + 0.5, 0, 1)
    crater = np.zeros((h, w), np.float32)
    for _ in range(9):
        a_, r_ = rng.uniform(0, 6.28), rng.uniform(0, mr * 0.7)
        cr = rng.uniform(10, 26)
        d_ = np.sqrt((x - (mx + np.cos(a_) * r_)) ** 2 + (y - (my + np.sin(a_) * r_)) ** 2)
        crater = np.maximum(crater, np.clip(1 - np.abs(d_ - cr) / 1.6, 0, 1))
    curtain = 0.5 + 0.5 * np.sin(x / 70.0 + 2.2 * np.sin(y / 260.0) + noise(w, h, 150, 122) * 3)
    aurora = smooth(0.4, 0.95, curtain) * smooth(0.45, 1.0, y / h) * edge

    def stars(draw):
        for _ in range(60):
            cx, cy = rng.random() * w, rng.random() * h * 0.7
            if abs(cx / w - 0.5) < 0.3 and abs(cy / h - 0.4) < 0.3:
                continue
            draw.polygon(star_polygon(cx, cy, 8 + rng.random() ** 2 * 30), fill=255)
    img, smask, sglow = sketch(stars, w, h, 0.5)
    ridge = skyline(w, 3.0, h * 1.0, h * 0.2, 90.0)
    ground = smooth(-1.5, 1.5, y - ridge[None, :])
    layers = [
        ink(halftone(aurora * 0.9, 13, 75), tint(paint, 0.45), 0.55, w, h, 123),
        ink(sglow, tint(paint, 0.55), 0.25, w, h, 124),
        ink(moon * 0.9, CREAM, 0.75, w, h, 125),
        ink(crater * moon, shadeof(paint, 0.5), 0.6, w, h, 126),
        ink(smask, CREAM, 0.85, w, h, 127),
        ink(hatch(ground, 6, 60, 2.4), shadeof(paint, 0.6), 0.65, w, h, 128),
    ]
    return compose(layers, w, h)


def bells(w=1320, h=1200):
    """Glass: bells on chains from the top corners, rings spreading off them, and glints"""
    paint = (28, 104, 60)
    x, y = grid(w, h)
    edge = edges(w, h, 0.34)
    rings = np.zeros((h, w), np.float32)
    for cx in (w * 0.1, w * 0.9):
        d = np.sqrt((x - cx) ** 2 + (y - h * 0.42) ** 2)
        rings = np.maximum(rings, smooth(0.3, 0.9, 0.5 + 0.5 * np.cos(d / 22.0)) * smooth(h * 0.8, 120, d))

    def shapes(draw):
        for cx in (w * 0.1, w * 0.9):
            draw.line([(cx, 0), (cx, h * 0.1)], fill=255, width=8)
            for k in range(5):
                draw.ellipse([cx - 9, k * 24, cx + 9, k * 24 + 30], outline=255, width=4)
            draw.ellipse([cx - 82, h * 0.1, cx + 82, h * 0.38], fill=255)
            draw.polygon([(cx - 80, h * 0.26), (cx + 80, h * 0.26), (cx + 120, h * 0.41), (cx - 120, h * 0.41)],
                         fill=255)
            draw.ellipse([cx - 120, h * 0.395, cx + 120, h * 0.435], fill=255)
            draw.ellipse([cx - 20, h * 0.43, cx + 20, h * 0.43 + 40], fill=255)
            for gx, gy, r in ((0.7, 0.16, 38), (0.6, 0.55, 22), (-0.6, 0.52, 28), (-0.7, 0.2, 18), (0.2, 0.7, 16)):
                draw.polygon(star_polygon(cx + gx * 170, h * gy, r), fill=255)
    img, mask, glow = sketch(shapes, w, h)
    layers = [
        ink(halftone(rings * edge * 0.9, 11, 60), tint(paint, 0.4), 0.5, w, h, 131),
        ink(glow, tint(paint, 0.55), 0.3, w, h, 132),
        ink(np.roll(hatch(mask, 6, 60, 2.3), (3, 4), axis=(0, 1)), shadeof(paint, 0.6), 0.7, w, h, 133),
        ink(rim_of(img, mask), CREAM, 0.8, w, h, 134),
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
