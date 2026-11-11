#!/usr/bin/env python3
"""Regenerate every rusty-tip branding asset from the two 1x pixel sources.

Sources (edit these, then re-run):
    source/icon-24.png  24x24 icon
    source/lockup.png   icon + "rusty tip" wordmark, 24 px tall

Only integer nearest-neighbour scaling is used so pixels stay crisp. Sizes that
aren't a multiple of 24 (32, 64, 128, 256, 512) use the largest multiple that
fits, centred on a transparent canvas; the icon has an open border, so the
margin reads as intentional.

Needs Pillow:  nix shell nixpkgs#python3Packages.pillow -c python3 export.py
"""
from pathlib import Path
from PIL import Image

HERE = Path(__file__).resolve().parent
OUT = HERE
DARK = (30, 30, 36, 255)
LIGHT = (245, 240, 232, 255)


def scaled(im, size):
    """Nearest-neighbour scale to the largest integer multiple fitting `size`, centred."""
    k = max(1, size // im.width)
    big = im.resize((im.width * k, im.height * k), Image.NEAREST)
    if big.size == (size, size):
        return big
    canvas = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    canvas.alpha_composite(big, ((size - big.width) // 2, (size - big.height) // 2))
    return canvas


def master(masters, size):
    """Master that divides `size` evenly (the larger master wins on ties), else the one filling it best."""
    exact = [m for m in masters if size % m.width == 0]
    if exact:
        return min(exact, key=lambda m: size // m.width)
    return max(masters, key=lambda m: (size // m.width) * m.width)


def render(masters, size):
    return scaled(master(masters, size), size)


def svg(im, path):
    """One <rect> per horizontal run of identical pixels."""
    w, h = im.size
    px = im.load()
    rects = []
    for y in range(h):
        x = 0
        while x < w:
            r, g, b, a = px[x, y]
            if a == 0:
                x += 1
                continue
            run = 1
            while x + run < w and px[x + run, y] == (r, g, b, a):
                run += 1
            op = '' if a == 255 else f' fill-opacity="{a / 255:.3f}"'
            rects.append(f'<rect x="{x}" y="{y}" width="{run}" height="1" fill="#{r:02x}{g:02x}{b:02x}"{op}/>')
            x += run
    path.write_text(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w * 8}" height="{h * 8}" '
        f'shape-rendering="crispEdges">\n' + '\n'.join(rects) + '\n</svg>\n')


def main():
    icon24 = Image.open(HERE / 'source/icon-24.png').convert('RGBA')
    masters = [icon24]
    lockup = Image.open(HERE / 'source/lockup.png').convert('RGBA')

    # --- icon PNGs, each from its native master -------------------------------
    d = OUT / 'icon'; d.mkdir(exist_ok=True)
    for old in d.glob('rusty-tip-*'):
        old.unlink()
    for size in (24, 32, 48, 64, 96, 128, 192, 256, 512):
        render(masters, size).save(d / f'rusty-tip-{size}.png')
    svg(icon24, d / 'rusty-tip.svg')

    # --- freedesktop hicolor theme (Linux .desktop / Wayland app icon) ------
    for size in (24, 32, 48, 64, 96, 128, 256, 512):
        p = OUT / 'hicolor' / f'{size}x{size}' / 'apps'
        p.mkdir(parents=True, exist_ok=True)
        render(masters, size).save(p / 'rusty-tip.png')
    p = OUT / 'hicolor' / 'scalable' / 'apps'; p.mkdir(parents=True, exist_ok=True)
    svg(icon24, p / 'rusty-tip.svg')

    # --- Windows .ico (every entry pre-rendered, no resampling) -------------
    sizes = [24, 32, 48, 64, 96, 128, 256]
    frames = [render(masters, s) for s in sizes]
    frames[-1].save(d / 'rusty-tip.ico', format='ICO', sizes=[(s, s) for s in sizes],
                    append_images=frames[:-1])

    # --- raw RGBA for egui::IconData (include_bytes!, no decoder needed) ----
    for s in (48, 96):
        (d / f'rusty-tip-{s}.rgba').write_bytes(render(masters, s).tobytes())

    # --- wordmark lockup -----------------------------------------------------
    d = OUT / 'logo'; d.mkdir(exist_ok=True)
    for k in (1, 2, 4, 8):
        lockup.resize((lockup.width * k, lockup.height * k), Image.NEAREST).save(d / f'rusty-tip-lockup-{k}x.png')
    svg(lockup, d / 'rusty-tip-lockup.svg')

    # --- GitHub social preview (1280x640) ------------------------------------
    d = OUT / 'social'; d.mkdir(exist_ok=True)
    k = min(1000 // lockup.width, 320 // lockup.height)
    big = lockup.resize((lockup.width * k, lockup.height * k), Image.NEAREST)
    for name, bg in (('dark', DARK), ('light', LIGHT)):
        card = Image.new('RGBA', (1280, 640), bg)
        card.alpha_composite(big, ((1280 - big.width) // 2, (640 - big.height) // 2))
        card.convert('RGB').save(d / f'social-preview-{name}.png')


if __name__ == '__main__':
    main()
