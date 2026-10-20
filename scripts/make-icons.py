#!/usr/bin/env python3
"""Fit the approved transparent roll on square icon canvases (requires Pillow).

Run from the repository root: python scripts/make-icons.py [source.png]
Only empty alpha margins are trimmed. Aspect ratio and artwork are preserved.
"""
from pathlib import Path
import sys
from PIL import Image, ImageOps

root = Path(__file__).resolve().parent.parent
source = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "crates/pasteroll-core/assets/logo-source.png"
sizes = (16, 24, 32, 48, 64, 128, 256)


def clean_transparent_pixels(image):
    pixels = bytearray(image.tobytes())
    for i in range(0, len(pixels), 4):
        if pixels[i + 3] == 0:
            pixels[i:i + 3] = b"\0\0\0"
    return Image.frombytes("RGBA", image.size, bytes(pixels))


with Image.open(source) as original:
    source_image = original.convert("RGBA")
    bounds = source_image.getchannel("A").getbbox()
    if bounds is None:
        raise SystemExit("The icon source is fully transparent.")
    # Center the complete visible roll with a four-percent transparent margin.
    # Resize once at high resolution, then derive every output from this canvas.
    roll = ImageOps.contain(source_image.crop(bounds), (942, 942), Image.Resampling.LANCZOS)
    image = Image.new("RGBA", (1024, 1024))
    image.paste(roll, ((1024 - roll.width) // 2, (1024 - roll.height) // 2))
    image = clean_transparent_pixels(image)
    image.info.clear()
    assets = root / "crates/pasteroll-core/assets"
    for size in (*sizes, 512):
        output = clean_transparent_pixels(image.resize((size, size), Image.Resampling.LANCZOS))
        output.info.clear()
        output.save(assets / f"logo-{size}.png")
    image.save(root / "crates/pasteroll-win/pasteroll.ico", sizes=[(s, s) for s in sizes])
print("Generated ICO sizes 16/24/32/48/64/128/256 and Linux PNG sizes through 512.")
