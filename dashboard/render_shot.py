#!/usr/bin/env python3
"""Render one shot's weight curve (with HX temperature behind it) to a JPEG.

Usage: render_shot.py <out.jpg>   — shot JSON on stdin, as stored in shots.json:
    {"ts": ms, "duration": s, "weight_g": g|null, "aborted": bool,
     "curve": [[s, g], ...], "hx": [[s, °C], ...]}

Deployed to the Pi by deploy.py and called by the Node-RED state function after each shot.
Only needs Pillow (python3-pil) and the DejaVu fonts, both present on Raspberry Pi OS.
"""

import json
import math
import sys
from datetime import datetime

from PIL import Image, ImageDraw, ImageFont

W, H = 1200, 700
ML, MR, MT, MB = 90, 90, 110, 70  # plot margins: left (g axis), right (°C axis), top (title), bottom
BG = (255, 255, 255)
GRID = (228, 222, 214)
TEXT = (34, 26, 21)
MUTED = (122, 109, 99)
WEIGHT = (21, 128, 61)
HX_LINE = (37, 99, 235)
HX_FILL = (219, 230, 252)

FONT_DIR = "/usr/share/fonts/truetype/dejavu/"


def font(size, bold=False):
    try:
        return ImageFont.truetype(FONT_DIR + ("DejaVuSans-Bold.ttf" if bold else "DejaVuSans.ttf"), size)
    except OSError:
        return ImageFont.load_default()


def de1(v):
    return f"{v:.1f}".replace(".", ",")


def nice_step(span, count):
    raw = span / max(count, 1)
    mag = 10 ** math.floor(math.log10(raw)) if raw > 0 else 1
    for m in (1, 2, 5, 10):
        if raw <= m * mag:
            return m * mag
    return 10 * mag


def ticks(lo, hi, count):
    step = nice_step(hi - lo, count)
    v = math.ceil(lo / step) * step
    out = []
    while v <= hi + 1e-9:
        out.append(round(v, 6))
        v += step
    return out


def render(shot, out_path):
    curve = shot.get("curve") or []
    hx = shot.get("hx") or []
    duration = shot.get("duration") or (curve[-1][0] if curve else 0)
    weight = shot.get("weight_g")

    max_t = max([30, duration] + [p[0] for p in curve] + [p[0] for p in hx])
    max_t = math.ceil(max_t / 5) * 5
    max_g = max([20] + [p[1] for p in curve] + ([weight] if isinstance(weight, (int, float)) else []))
    max_g = math.ceil(max_g * 1.1 / 5) * 5

    x0, x1, y0, y1 = ML, W - MR, MT, H - MB
    xs = lambda t: x0 + t / max_t * (x1 - x0)
    ys = lambda g: y1 - max(0.0, g) / max_g * (y1 - y0)

    img = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(img)
    f_title, f_sub, f_axis = font(30, True), font(20), font(17)

    # Title: date/time, then the key numbers
    when = datetime.fromtimestamp(shot["ts"] / 1000).strftime("%d.%m.%Y  %H:%M:%S")
    d.text((ML, 22), "Shot " + when, font=f_title, fill=TEXT)
    parts = [f"Dauer {duration} s"]
    if isinstance(weight, (int, float)):
        parts.append(f"Gewicht {de1(weight)} g")
        if duration:
            parts.append(f"Fluss {de1(weight / duration)} g/s")
    if hx:
        temps = [p[1] for p in hx]
        parts.append(f"HX {min(temps)}–{max(temps)} °C")
    if shot.get("aborted"):
        parts.append("kurz")
    d.text((ML, 64), "  ·  ".join(parts), font=f_sub, fill=MUTED)

    # Grid + left (g) and bottom (s) axis labels
    for g in ticks(0, max_g, 5):
        y = ys(g)
        d.line([(x0, y), (x1, y)], fill=GRID, width=1)
        d.text((x0 - 10, y), f"{g:g} g", font=f_axis, fill=MUTED, anchor="rm")
    for t in ticks(0, max_t, 6):
        d.text((xs(t), y1 + 14), f"{t:g} s", font=f_axis, fill=MUTED, anchor="mt")

    # HX temperature behind the weight curve, own scale on the right
    if len(hx) >= 1:
        temps = [p[1] for p in hx]
        lo, hi = math.floor(min(temps) - 3), math.ceil(max(temps) + 3)
        yh = lambda v: y1 - (v - lo) / (hi - lo) * (y1 - y0)
        pts = [(xs(min(t, max_t)), yh(v)) for t, v in hx]
        if len(pts) == 1:
            pts.append((xs(duration), pts[0][1]))
        d.polygon(pts + [(pts[-1][0], y1), (pts[0][0], y1)], fill=HX_FILL)
        d.line(pts, fill=HX_LINE, width=2, joint="curve")
        for v in ticks(lo, hi, 4):
            d.text((x1 + 10, yh(v)), f"{v:g} °C", font=f_axis, fill=HX_LINE, anchor="lm")

    # Weight curve on top
    if len(curve) >= 2:
        pts = [(xs(t), ys(g)) for t, g in curve]
        d.line(pts, fill=WEIGHT, width=5, joint="curve")
        ex, ey = pts[-1]
        d.ellipse([ex - 7, ey - 7, ex + 7, ey + 7], fill=WEIGHT)
        if isinstance(weight, (int, float)):
            d.text((ex + 12, ey - 12), f"{de1(weight)} g", font=font(20, True), fill=WEIGHT, anchor="ls")

    d.line([(x0, y1), (x1, y1)], fill=MUTED, width=1)

    # Legend (bottom right)
    lx, ly = x1 - 360, H - 26
    d.line([(lx, ly), (lx + 26, ly)], fill=WEIGHT, width=5)
    d.text((lx + 34, ly), "Gewicht (links)", font=f_axis, fill=MUTED, anchor="lm")
    lx += 190
    d.rectangle([lx, ly - 6, lx + 26, ly + 6], fill=HX_FILL, outline=HX_LINE)
    d.text((lx + 34, ly), "HX (rechts)", font=f_axis, fill=MUTED, anchor="lm")

    img.save(out_path, "JPEG", quality=90)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    render(json.load(sys.stdin), sys.argv[1])
