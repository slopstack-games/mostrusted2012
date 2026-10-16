#!/usr/bin/env python3
"""Stack screenshots into one downscaled image, for looking at an A/B in one read.

Usage: stack.py out.png a.png b.png [c.png ...] [--width 900] [--crop x0,y0,x1,y1] [--side]

Images are stacked top to bottom (--side: left to right), each resized to the first one's size, after an optional
crop (the same box for all), then the result is shrunk to fit --width (default 900 px).
"""
import argparse

from PIL import Image


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("images", nargs="+")
    ap.add_argument("--width", type=int, default=900)
    ap.add_argument("--crop")
    ap.add_argument("--side", action="store_true")
    a = ap.parse_args()
    ims = [Image.open(p).convert("RGB") for p in a.images]
    if a.crop:
        box = tuple(int(v) for v in a.crop.split(","))
        ims = [im.crop(box) for im in ims]
    w, h = ims[0].size
    ims = [im.resize((w, h)) for im in ims]
    n = len(ims)
    out = Image.new("RGB", (w * n, h) if a.side else (w, h * n))
    for i, im in enumerate(ims):
        out.paste(im, (w * i, 0) if a.side else (0, h * i))
    out.thumbnail((a.width, a.width * 4))
    out.save(a.out)


if __name__ == "__main__":
    main()
