#!/usr/bin/env python3
"""Convert the approved square source to Windows ICO and Linux PNG sizes (requires Pillow).

Run from the repository root: python scripts/make-icons.py [source.webp]
No drawing, cropping or metadata from the source is added to the generated assets.
"""
from pathlib import Path
import sys
from PIL import Image

root = Path(__file__).resolve().parent.parent
source = Path(sys.argv[1]) if len(sys.argv) > 1 else root / "docs/icon-source-1254.webp"
sizes = (16, 24, 32, 48, 64, 128, 256)
with Image.open(source) as original:
    if original.width != original.height:
        raise SystemExit("The icon source must be square.")
    image = original.convert("RGBA")
    image.info.clear()
    assets = root / "crates/pasteroll-core/assets"
    for size in (*sizes, 512):
        output = image.resize((size, size), Image.Resampling.LANCZOS)
        output.info.clear()
        output.save(assets / f"logo-{size}.png")
    image.save(root / "crates/pasteroll-win/pasteroll.ico", sizes=[(s, s) for s in sizes])
print("Generated ICO sizes 16/24/32/48/64/128/256 and Linux PNG sizes through 512.")
