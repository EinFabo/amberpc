"""Renders the plugin's PNGs from Amber's app icon (the amber disc with the flame).

    python3 tools/make-icons.py ../src-tauri/app-icon.png com.einfabo.amber.sdPlugin/imgs
"""
import sys
from pathlib import Path

from PIL import Image

src, out = Path(sys.argv[1]), Path(sys.argv[2])
icon = Image.open(src).convert("RGBA")

def save(rel, size, background=None):
    img = icon.resize((size, size), Image.LANCZOS)
    if background:
        canvas = Image.new("RGBA", (size, size), background)
        # The key image: the disc a little smaller than the key, on Amber's ink.
        inner = icon.resize((int(size * 0.8), int(size * 0.8)), Image.LANCZOS)
        canvas.alpha_composite(inner, ((size - inner.width) // 2, (size - inner.height) // 2))
        img = canvas
    path = out / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)

for name, size, bg in [
    ("plugin/marketplace", 144, None),
    ("plugin/category", 28, None),
    ("actions/dictate/icon", 20, None),
    ("actions/dictate/key", 72, (10, 10, 10, 255)),
]:
    save(f"{name}.png", size, bg)
    save(f"{name}@2x.png", size * 2, bg)
print("icons written to", out)
