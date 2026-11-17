#!/usr/bin/env python3
"""Regenerate the bundled application icon set from the vector falcon master.

Outputs are committed, so this is a dev utility rather than a build step:
    python3 scripts/generate-app-icons.py

Source of truth is `src-tauri/icons/source/xfalcon_bird.svg`, the brand pack's
geometry master (47 straight segments, no curves, `fill-rule="evenodd"`). It is
rasterised here rather than upscaled from a PNG so the mark stays exact at 16pt.
The renderer only understands M/L/Z because that is all the master uses; it will
refuse anything else rather than silently drop a curve.

macOS and Windows want different silhouettes from the same artwork. macOS
composites icons onto the system squircle -- a full-bleed square reads as a
foreign tile in the Dock -- so `.icns` insets the mark into an 824pt superellipse
on a 1024pt canvas, Apple's macOS icon grid. Windows and Linux draw the icon
square, so those targets stay full-bleed.

macOS 26 and later ignore that `.icns`: an icon that does not fill the system
shape is shrunk into a light squircle of its own, which reads as a white border.
`xFalcon.icon` is the fix: an Icon Composer document holding only the navy fill
and the gradient mark, so the system draws the shape. It is generated here but
NOT listed in `bundle.icon` yet: the release runner's actool (Xcode 26, macOS 26
image) failed to compile it in the v0.5.50 build, while Xcode 27's compiles it
cleanly. Until that is resolved every macOS version uses the `.icns`.
"""

import json

import re
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
SOURCE = ICONS / "source" / "xfalcon_bird.svg"

NAVY = (6, 17, 34)  # #061122
# The brand pack's app-icon treatment is the gradient mark on navy, not the flat
# cyan one -- see xfalcon_brand_pack/xfalcon_bird_icon_kit/ios_icons_opaque.
# The ramp runs vertically across the mark's own bounding box, matching
# svg/xfalcon_bird_gradient.svg.
MARK_GRADIENT = ((92, 227, 245), (34, 156, 177))  # #5CE3F5 -> #229CB1

CANVAS = 1024
# Apple's macOS icon grid: an 824pt body centred on a 1024pt canvas.
BODY = 824
# Apple's continuous corner is a superellipse, not a circular-arc rounded rect;
# exponent 5 is the standard fit for the macOS/iOS squircle.
SUPERELLIPSE_N = 5.0
# Fraction of the tile's width the mark spans. The master's viewBox is a tight
# bbox, so this is the only padding the artwork gets.
MARK_WIDTH_RATIO = 0.78
# Masks are built at 4x and downsampled; that is what antialiases every edge.
SUPERSAMPLE = 4

ICNS_SIZES = [16, 32, 64, 128, 256, 512, 1024]
ICO_SIZES = [16, 32, 48, 64, 128, 256]


def parse_subpaths(svg_text: str) -> tuple[list[list[tuple[float, float]]], float, float]:
    """Return the master's closed polygons plus its viewBox extent."""
    view_box = re.search(r'viewBox="([\d.\s-]+)"', svg_text)
    if not view_box:
        raise ValueError("master has no viewBox")
    _, _, vb_w, vb_h = (float(v) for v in view_box.group(1).split())

    d = re.search(r'\bd="([^"]+)"', svg_text)
    if not d:
        raise ValueError("master has no path data")
    data = d.group(1)

    unsupported = set(re.findall(r"[A-Za-z]", data)) - {"M", "L", "Z"}
    if unsupported:
        raise ValueError(f"master uses unsupported path commands: {sorted(unsupported)}")

    subpaths: list[list[tuple[float, float]]] = []
    for chunk in (c for c in data.split("M") if c.strip()):
        points = [
            (float(x), float(y))
            for x, y in re.findall(r"(-?[\d.]+),(-?[\d.]+)", chunk)
        ]
        if len(points) >= 3:
            subpaths.append(points)
    return subpaths, vb_w, vb_h


def mark_mask(size: int, target_width: float) -> Image.Image:
    """Rasterise the falcon into a `size` canvas, honouring evenodd fill.

    Under evenodd a pixel is inside when an odd number of subpaths cover it,
    which is exactly an XOR of the per-subpath masks.
    """
    subpaths, vb_w, vb_h = parse_subpaths(SOURCE.read_text())
    hi = size * SUPERSAMPLE
    scale = (target_width * SUPERSAMPLE) / vb_w
    offset_x = (hi - vb_w * scale) / 2.0
    offset_y = (hi - vb_h * scale) / 2.0

    combined = Image.new("L", (hi, hi), 0)
    for points in subpaths:
        layer = Image.new("L", (hi, hi), 0)
        ImageDraw.Draw(layer).polygon(
            [(x * scale + offset_x, y * scale + offset_y) for x, y in points],
            fill=255,
        )
        # Masks here are strictly 0/255, so a difference is an XOR.
        combined = ImageChops.difference(combined, layer)
    return combined.resize((size, size), Image.LANCZOS)


def squircle_mask(size: int) -> Image.Image:
    """An antialiased superellipse mask, |x|^n + |y|^n <= 1."""
    hi = size * SUPERSAMPLE
    mask = Image.new("L", (hi, hi), 0)
    draw = ImageDraw.Draw(mask)
    half = hi / 2.0
    for y in range(hi):
        # Sample pixel centres so the curve stays symmetric.
        ny = abs((y + 0.5) - half) / half
        ny_n = ny**SUPERELLIPSE_N
        if ny_n > 1.0:
            continue
        x_limit = ((1.0 - ny_n) ** (1.0 / SUPERELLIPSE_N)) * half
        draw.line([(half - x_limit, y), (half + x_limit - 1, y)], fill=255)
    return mask.resize((size, size), Image.LANCZOS)


def gradient_over(box: tuple[int, int, int, int], size: int) -> Image.Image:
    """The brand ramp, spanning `box` vertically on a `size` canvas."""
    top, bottom = MARK_GRADIENT
    img = Image.new("RGB", (size, size), top)
    draw = ImageDraw.Draw(img)
    y0, y1 = box[1], box[3]
    span = max(1, y1 - y0 - 1)
    for y in range(size):
        t = min(1.0, max(0.0, (y - y0) / span))
        draw.line(
            [(0, y), (size, y)],
            fill=tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)),
        )
    return img


def tile(size: int, mask: Image.Image | None) -> Image.Image:
    """A navy tile carrying the gradient mark, clipped to `mask` when given."""
    art = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    body = Image.new("RGBA", (size, size), NAVY + (255,))
    if mask is not None:
        body.putalpha(mask)
    art.alpha_composite(body)

    alpha = mark_mask(size, size * MARK_WIDTH_RATIO)
    box = alpha.getbbox() or (0, 0, size, size)
    falcon = gradient_over(box, size).convert("RGBA")
    if mask is not None:
        alpha = ImageChops.darker(alpha, mask)
    falcon.putalpha(alpha)
    art.alpha_composite(falcon)
    return art


def macos_icon(size: int) -> Image.Image:
    """The squircle tile inset on a transparent canvas, at Apple's body ratio.

    Rendered natively at `size` rather than downsampled from the 1024pt master:
    the mark's strokes are thin, and resampling them muddies 16pt and 32pt.
    """
    body_size = round(size * BODY / CANVAS)
    body = tile(body_size, squircle_mask(body_size))
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    offset = (size - body_size) // 2
    canvas.alpha_composite(body, (offset, offset))
    return canvas


def write_icns() -> None:
    iconset = ICONS / "icon.iconset"
    if iconset.exists():
        for stale in iconset.iterdir():
            stale.unlink()
    iconset.mkdir(parents=True, exist_ok=True)
    for size in ICNS_SIZES:
        scaled = macos_icon(size)
        if size <= 512:
            scaled.save(iconset / f"icon_{size}x{size}.png")
        if size >= 32:
            scaled.save(iconset / f"icon_{size // 2}x{size // 2}@2x.png")
    subprocess.run(
        ["iconutil", "-c", "icns", str(iconset), "-o", str(ICONS / "icon.icns")],
        check=True,
    )
    for stale in iconset.iterdir():
        stale.unlink()
    iconset.rmdir()


def write_icon_composer() -> None:
    """The macOS 26+ icon: navy fill plus the mark as one vector layer.

    Icon Composer centres a layer at its own size on the 1024pt canvas, so the
    SVG's width is the mark's width on the tile.
    """
    d = re.search(r'\bd="([^"]+)"', SOURCE.read_text())
    _, vb_w, vb_h = parse_subpaths(SOURCE.read_text())
    width = CANVAS * MARK_WIDTH_RATIO
    height = width * vb_h / vb_w
    top, bottom = ("#%02X%02X%02X" % rgb for rgb in MARK_GRADIENT)
    document = ICONS / "xFalcon.icon"
    (document / "Assets").mkdir(parents=True, exist_ok=True)
    (document / "Assets" / "falcon.svg").write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:g}" height="{height:.1f}" '
        f'viewBox="0 0 {vb_w:g} {vb_h:g}" fill-rule="evenodd">\n'
        "  <defs>\n"
        '    <linearGradient id="ramp" x1="0" y1="0" x2="0" y2="1">\n'
        f'      <stop offset="0" stop-color="{top}"/>\n'
        f'      <stop offset="1" stop-color="{bottom}"/>\n'
        "    </linearGradient>\n"
        "  </defs>\n"
        f'  <path fill="url(#ramp)" d="{d.group(1)}"/>\n'
        "</svg>\n"
    )
    navy = ",".join(f"{c / 255:.5f}" for c in NAVY)
    manifest = {
        "fill": {"solid": f"srgb:{navy},1.00000"},
        "groups": [
            {
                "layers": [{"image-name": "falcon.svg", "name": "falcon"}],
                "shadow": {"kind": "neutral", "opacity": 0.5},
                "translucency": {"enabled": False, "value": 0.5},
            }
        ],
        "supported-platforms": {"squares": ["macOS"]},
    }
    (document / "icon.json").write_text(json.dumps(manifest, indent=2) + "\n")


def main() -> int:
    if not SOURCE.exists():
        print(f"missing vector master: {SOURCE}", file=sys.stderr)
        return 1

    write_icns()
    write_icon_composer()

    # Square targets keep the artwork full-bleed, each rendered at its own size.
    for name, size in [
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", CANVAS),
    ]:
        tile(size, None).save(ICONS / name)
    ico = tile(max(ICO_SIZES), None)
    ico.save(ICONS / "icon.ico", format="ICO", sizes=[(s, s) for s in ICO_SIZES])

    print(f"wrote icon set to {ICONS}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
