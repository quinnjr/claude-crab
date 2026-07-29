#!/usr/bin/env python3
"""Generate the claude-crab sprite sheet and its manifest.

The character is Clawd, drawn the way the Claude Code mascot is drawn: flat
terracotta blocks on a coarse grid, no outline and no shading. A 12x8 cell body
-- torso, two square eyes, a nub on each side, four stubby legs -- laid out on a
16x16 cell canvas at 4px per cell, giving 64px frames.

Poses are expressed in grid cells rather than pixels, because drawing off-grid
is what makes a blocky sprite look wrong. The few places that do move by pixels
(hops, blinks, lean) say so explicitly.

Three variants are emitted: the plain character, a 'fancy' one in a top hat and
monocle, and a 'party' one in a birthday hat. They share a single manifest,
because they differ only in what is drawn inside a frame, never in the layout.

The manifest is the contract between art and code: as long as a replacement PNG
keeps the same row/frame layout, no QML has to change.
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path

from PIL import Image, ImageDraw

# --- palette (sampled from the mascot) -------------------------------------

BODY = (208, 106, 75, 255)
EYE = (43, 42, 38, 255)
GLASS = (240, 238, 230, 255)  # monocle rim, party hat: reads against BODY and EYE
TRANSPARENT = (0, 0, 0, 0)

# --- geometry --------------------------------------------------------------

GRID = 16  # canvas, in cells
CELL = 4  # pixels per cell
FRAME = GRID * CELL

# The body's own bounding box, in cells.
BODY_W = 12
BODY_H = 8

# Columns, relative to the body box.
TORSO_L, TORSO_R = 2, 9  # inclusive
NUB_L, NUB_R = (0, 1), (10, 11)
LEG_COLS = (2, 4, 7, 9)
EYE_COLS = (3, 8)

# Rows, relative to the body box.
TORSO_TOP, TORSO_BOTTOM = 0, 5
NUB_ROWS = (2, 3)
LEG_ROWS = (6, 7)
EYE_ROW = 1

# Where the body sits when standing on the ground.
REST_X = (GRID - BODY_W) // 2
REST_Y = GRID - BODY_H - 1  # leaves the bottom row of the frame clear


def _pose(**overrides) -> dict:
    """A neutral pose, with any field overridden.

    dx, dy       whole-body offset, in cells
    hop          extra vertical offset, in pixels, for sub-cell arcs
    legs         per-leg lift in cells, one entry per LEG_COLS
    nub_l/nub_r  vertical lift of each side nub, in cells
    eye_dy       eye offset, in pixels
    eye_open     1 fully open, 0 a closed slit
    lean         horizontal shear of the torso top, in pixels
    rot          rotation, in degrees, about the frame centre
    airborne     centre the body vertically before rotating, so a spin fits
    """
    pose = {
        "dx": 0,
        "dy": 0,
        "hop": 0,
        "legs": (0, 0, 0, 0),
        "nub_l": 0,
        "nub_r": 0,
        "eye_dy": 0,
        "eye_open": 1.0,
        "lean": 0,
        "rot": 0.0,
        "airborne": False,
    }
    pose.update(overrides)
    return pose


def _rect(d: ImageDraw.ImageDraw, col: float, row: float, w: float, h: float,
          fill, px_dx: float = 0, px_dy: float = 0) -> None:
    """Fill a cell-aligned rectangle, with an optional pixel-level nudge."""
    x0 = col * CELL + px_dx
    y0 = row * CELL + px_dy
    d.rectangle([x0, y0, x0 + w * CELL - 1, y0 + h * CELL - 1], fill=fill)


def _draw_party_hat(d: ImageDraw.ImageDraw, bx: int, by: int, p: dict) -> None:
    """A birthday cone, for the 'party' variant.

    A 6/4/2 stepped cone, banded light/dark/light. The taper has to come from
    the tier widths: at four pixels per cell there is no room to draw a smooth
    slope, and a near-cylinder reads as a chef's hat instead of a party one.

    Tier widths stay even so every tier centres on the same column pair, and the
    cone occupies the same three rows above the torso as the top hat, leaving
    the frame-clipping budget unchanged.
    """
    hop = p["hop"]
    lean = p["lean"]

    _rect(d, bx + 3, by - 1, 6, 1, GLASS, px_dx=lean, px_dy=hop)
    _rect(d, bx + 4, by - 2, 4, 1, GLASS, px_dx=lean, px_dy=hop)
    # The top tier is the pom, not another tier of cone: a dark band across the
    # middle instead just reads as a beanie.
    _rect(d, bx + 5, by - 3, 2, 1, EYE, px_dx=lean, px_dy=hop)

    # A single body-coloured stripe, at pixel resolution because a whole cell
    # would swallow the tier it sits on.
    stripe_y = (by - 1) * CELL + hop + 1
    stripe_x = (bx + 3) * CELL + lean
    d.rectangle([stripe_x, stripe_y, stripe_x + 6 * CELL - 1, stripe_y + 1], fill=BODY)


def _draw_finery(d: ImageDraw.ImageDraw, bx: int, by: int, p: dict) -> None:
    """Top hat and monocle, for the 'fancy' variant.

    Drawn after the body so it sits on top, and before rotation so the hat spins
    with the wearer. The hat occupies the three cell rows above the torso, which
    is why the resting body leaves that much headroom.
    """
    hop = p["hop"]
    lean = p["lean"]

    # Brim, then crown, then a body-coloured band so the hat is not a slab.
    _rect(d, bx + 3, by - 1, 6, 1, EYE, px_dx=lean, px_dy=hop)
    _rect(d, bx + 4, by - 3, 4, 2, EYE, px_dx=lean, px_dy=hop)
    band_y = (by - 2) * CELL + hop + CELL - 2
    band_x = (bx + 4) * CELL + lean
    d.rectangle([band_x, band_y, band_x + 4 * CELL - 1, band_y + 1], fill=BODY)

    # Monocle on the leading eye. The sprite is drawn facing right; QML mirrors
    # it for leftward travel, so the monocle always stays on the front eye.
    ex = (bx + EYE_COLS[1]) * CELL + lean
    ey = (by + EYE_ROW) * CELL + hop + p["eye_dy"]
    d.rectangle([ex - 1, ey - 1, ex + CELL, ey + CELL], outline=GLASS)
    d.line([(ex + CELL, ey + CELL + 1), (ex + CELL, ey + CELL + 3)], fill=GLASS)


def _draw_crab(p: dict, variant: str = "default") -> Image.Image:
    """Render one pose into a FRAME x FRAME RGBA image, facing right."""
    img = Image.new("RGBA", (FRAME, FRAME), TRANSPARENT)
    d = ImageDraw.Draw(img)

    bx = REST_X + p["dx"]
    by = ((GRID - BODY_H) // 2 if p["airborne"] else REST_Y) + p["dy"]
    hop = p["hop"]

    # Torso. Only the top two rows carry the lean, so a fast gait reads as the
    # body tipping forward rather than the whole sprite sliding sideways.
    torso_w = TORSO_R - TORSO_L + 1
    for row in range(TORSO_TOP, TORSO_BOTTOM + 1):
        lean = p["lean"] if row <= TORSO_TOP + 1 else 0
        _rect(d, bx + TORSO_L, by + row, torso_w, 1, BODY, px_dx=lean, px_dy=hop)

    # Side nubs.
    for cols, lift in ((NUB_L, p["nub_l"]), (NUB_R, p["nub_r"])):
        for row in NUB_ROWS:
            _rect(d, bx + cols[0], by + row - lift, len(cols), 1, BODY, px_dy=hop)

    # Legs, each shortened from the top when lifted.
    for col, lift in zip(LEG_COLS, p["legs"]):
        height = len(LEG_ROWS) - lift
        if height <= 0:
            continue
        _rect(d, bx + col, by + LEG_ROWS[0] + lift, 1, height, BODY, px_dy=hop)

    # Eyes. A closed eye is a slit rather than an omitted cell: at four pixels
    # per cell, leaving it out just reads as a blank body.
    for col in EYE_COLS:
        x0 = (bx + col) * CELL + p["lean"]
        height = max(1, round(CELL * p["eye_open"]))
        y0 = (by + EYE_ROW) * CELL + hop + p["eye_dy"] + (CELL - height) // 2
        d.rectangle([x0, y0, x0 + CELL - 1, y0 + height - 1], fill=EYE)

    if variant == "fancy":
        _draw_finery(d, bx, by, p)
    elif variant == "party":
        _draw_party_hat(d, bx, by, p)

    if p["rot"]:
        img = img.rotate(p["rot"], resample=Image.NEAREST, center=(FRAME / 2, FRAME / 2))
    return img


# --- animations ------------------------------------------------------------
#
# Leg patterns are written out rather than computed: with four legs and one cell
# of travel there is nothing worth computing, and a literal table is easier to
# check against the rendered sheet.

STEP_A = (0, 1, 0, 1)
STEP_B = (1, 0, 1, 0)
STAND = (0, 0, 0, 0)


def anim_sleep(n: int) -> list[dict]:
    return [_pose(eye_open=0.0, legs=STAND, hop=1 if i in (1, 2) else 0) for i in range(n)]


def _gait(n: int, bob: int, lean: int) -> list[dict]:
    frames = []
    for i in range(n):
        half = (i * 2) // n  # 0 through the first half of the cycle, 1 after
        frames.append(
            _pose(
                legs=STEP_A if half == 0 else STEP_B,
                hop=-bob if i % 2 else 0,
                lean=lean,
                dx=1 if (lean and half) else 0,
            )
        )
    return frames


def anim_walk(n: int) -> list[dict]:
    return _gait(n, bob=1, lean=0)


def anim_scuttle(n: int) -> list[dict]:
    return _gait(n, bob=2, lean=2)


def anim_creep(n: int) -> list[dict]:
    # Legs shuffle, but the body never leaves the ground.
    return _gait(n, bob=0, lean=0)


def anim_think(n: int) -> list[dict]:
    """Planted, eyes drifting upward and back."""
    frames = []
    for i in range(n):
        drift = math.sin(i / n * 2 * math.pi)
        frames.append(_pose(legs=STAND, eye_dy=-round(abs(drift) * 2)))
    return frames


def anim_wave(n: int) -> list[dict]:
    """Turned to the viewer, right nub raised and waving."""
    return [
        _pose(legs=STAND, nub_r=2 if i % 2 else 3, hop=-1 if i % 2 else 0)
        for i in range(n)
    ]


def anim_celebrate(n: int) -> list[dict]:
    """Squash, leap with both nubs up, land."""
    frames = []
    for i in range(n):
        t = i / (n - 1)
        if t < 0.25:  # anticipation
            frames.append(_pose(legs=STAND, hop=2))
            continue
        arc = math.sin((t - 0.25) / 0.75 * math.pi)
        frames.append(
            _pose(
                legs=(1, 1, 1, 1) if arc > 0.3 else STAND,
                hop=-round(arc * 10),
                nub_l=round(arc * 3),
                nub_r=round(arc * 3),
            )
        )
    return frames


# Rotation snaps to quarter turns. Rotating a blocky sprite by an arbitrary
# angle with nearest-neighbour sampling shreds it into loose pixels; quarter
# turns are lossless and keep the character legible mid-flip.
TUMBLE_ROTATION = (0, -90, -180, -180, -180, -180, -270, -270, -360, -360)


def anim_tumble(n: int) -> list[dict]:
    """Flip onto the back, flail, and right itself."""
    if n != len(TUMBLE_ROTATION):
        raise AssertionError(
            f"tumble declares {n} frames but TUMBLE_ROTATION has {len(TUMBLE_ROTATION)}"
        )
    frames = []
    for i in range(n):
        rot = TUMBLE_ROTATION[i]
        frames.append(
            _pose(
                # Centred before rotating: spun 90 degrees the body is taller
                # than the gap between its resting position and the frame edge.
                airborne=True,
                rot=rot,
                legs=STEP_B if i % 2 else STEP_A,
                eye_open=0.5,
            )
        )
    return frames


ANIMATIONS = [
    {"name": "sleep", "frames": 4, "fps": 3, "loop": True, "build": anim_sleep},
    {"name": "walk", "frames": 8, "fps": 10, "loop": True, "build": anim_walk},
    {"name": "scuttle", "frames": 8, "fps": 18, "loop": True, "build": anim_scuttle},
    {"name": "creep", "frames": 8, "fps": 5, "loop": True, "build": anim_creep},
    {"name": "think", "frames": 6, "fps": 5, "loop": True, "build": anim_think},
    {"name": "wave", "frames": 6, "fps": 8, "loop": True, "build": anim_wave},
    {"name": "celebrate", "frames": 8, "fps": 12, "loop": False, "build": anim_celebrate},
    {"name": "tumble", "frames": 10, "fps": 12, "loop": False, "build": anim_tumble},
]


# Mirrors CrabConfig::spriteVariants() and spriteFileName(); adding one here
# means adding it there and to the CMake resource list.
VARIANTS = {
    "default": "spritesheet.png",
    "fancy": "spritesheet-fancy.png",
    "party": "spritesheet-party.png",
}


def build_sheet(variant: str = "default") -> tuple[Image.Image, dict]:
    cols = max(a["frames"] for a in ANIMATIONS)
    rows = len(ANIMATIONS)
    sheet = Image.new("RGBA", (cols * FRAME, rows * FRAME), TRANSPARENT)

    manifest_anims = []
    for row, anim in enumerate(ANIMATIONS):
        poses = anim["build"](anim["frames"])
        if len(poses) != anim["frames"]:
            raise AssertionError(
                f"{anim['name']}: builder returned {len(poses)} poses, "
                f"manifest declares {anim['frames']}"
            )
        for col, pose in enumerate(poses):
            sheet.alpha_composite(_draw_crab(pose, variant), (col * FRAME, row * FRAME))
        manifest_anims.append(
            {
                "name": anim["name"],
                "row": row,
                "frames": anim["frames"],
                "fps": anim["fps"],
                "loop": anim["loop"],
            }
        )

    manifest = {
        "frameWidth": FRAME,
        "frameHeight": FRAME,
        "sheetWidth": cols * FRAME,
        "sheetHeight": rows * FRAME,
        "animations": manifest_anims,
    }
    return sheet, manifest


# --- application icon ------------------------------------------------------

ICON_SIZES = (32, 48, 64, 128, 256)


def build_icon(size: int, variant: str = "default") -> Image.Image:
    """Render the standing pose as a square app icon at @p size.

    Reuses the sprite renderer rather than keeping a separate icon asset, so the
    icon cannot drift from the character. Scaling is nearest-neighbour and the
    source is cropped tight then padded to a square, which keeps the blocks
    square at every size.
    """
    frame = _draw_crab(_pose(legs=STAND), variant)

    box = frame.getbbox()
    if box is None:
        raise AssertionError("icon pose rendered nothing")
    art = frame.crop(box)

    # Pad to square around the art, leaving one cell of margin.
    side = max(art.width, art.height) + 2 * CELL
    square = Image.new("RGBA", (side, side), TRANSPARENT)
    square.alpha_composite(art, ((side - art.width) // 2, (side - art.height) // 2))

    # Scale by whole pixels where possible so blocks stay crisp.
    return square.resize((size, size), Image.NEAREST)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "assets",
        help="directory to write spritesheet.png and manifest.json into",
    )
    parser.add_argument(
        "--icons",
        type=Path,
        help="also write hicolor-style app icons into this directory",
    )
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)

    # Both variants share one manifest: they differ only in what is drawn inside
    # a frame, never in the row and frame layout.
    manifest = None
    for name, filename in VARIANTS.items():
        sheet, manifest = build_sheet(name)
        sheet.save(args.out / filename)
        print(f"wrote {args.out / filename} ({sheet.width}x{sheet.height})")

    (args.out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {args.out / 'manifest.json'}")

    if args.icons:
        args.icons.mkdir(parents=True, exist_ok=True)
        for size in ICON_SIZES:
            path = args.icons / f"{size}.png"
            build_icon(size).save(path)
            print(f"wrote {path}")


if __name__ == "__main__":
    main()
