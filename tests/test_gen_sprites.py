"""Tests for the sprite sheet generator.

These guard the manifest-is-the-contract invariant: if the sheet and the
manifest ever disagree, the QML silently renders the wrong rows.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))

import gen_sprites  # noqa: E402


@pytest.fixture(scope="module", params=sorted(gen_sprites.VARIANTS))
def built(request):
    """Every invariant is checked against every sprite variant, so a new hat
    cannot quietly clip the frame or stop animating."""
    sheet, manifest = gen_sprites.build_sheet(request.param)
    return sheet, manifest


def test_sheet_dimensions_match_manifest(built):
    sheet, manifest = built
    assert sheet.width == manifest["sheetWidth"]
    assert sheet.height == manifest["sheetHeight"]

    cols = max(a["frames"] for a in manifest["animations"])
    rows = len(manifest["animations"])
    assert sheet.width == cols * manifest["frameWidth"]
    assert sheet.height == rows * manifest["frameHeight"]


def test_rows_are_contiguous_and_unique(built):
    _, manifest = built
    rows = [a["row"] for a in manifest["animations"]]
    assert rows == list(range(len(rows)))


def test_names_are_unique(built):
    _, manifest = built
    names = [a["name"] for a in manifest["animations"]]
    assert len(names) == len(set(names))


def test_every_state_the_brain_uses_exists(built):
    """CrabBrain.qml hardcodes these names; losing one breaks the crab."""
    _, manifest = built
    names = {a["name"] for a in manifest["animations"]}
    required = {"sleep", "walk", "scuttle", "creep", "think", "wave", "celebrate", "tumble"}
    assert required <= names


def test_no_declared_frame_is_empty(built):
    """An all-transparent frame means the pose renderer silently produced
    nothing, which shows up as the crab blinking out of existence."""
    sheet, manifest = built
    fw, fh = manifest["frameWidth"], manifest["frameHeight"]
    for anim in manifest["animations"]:
        for col in range(anim["frames"]):
            box = (col * fw, anim["row"] * fh, (col + 1) * fw, (anim["row"] + 1) * fh)
            alpha = sheet.crop(box).getchannel("A")
            assert alpha.getextrema()[1] > 0, f"{anim['name']} frame {col} is empty"


def test_unused_cells_are_transparent(built):
    """Short rows must not bleed pixels into cells the manifest says don't
    exist -- QML would happily render them."""
    sheet, manifest = built
    fw, fh = manifest["frameWidth"], manifest["frameHeight"]
    cols = max(a["frames"] for a in manifest["animations"])
    for anim in manifest["animations"]:
        for col in range(anim["frames"], cols):
            box = (col * fw, anim["row"] * fh, (col + 1) * fw, (anim["row"] + 1) * fh)
            alpha = sheet.crop(box).getchannel("A")
            assert alpha.getextrema() == (0, 0), f"{anim['name']} has art past its frame count"


def test_no_frame_is_clipped_by_the_cell_edge(built):
    """Art touching the frame border means the pose is wider than the cell and
    is being silently cut off -- which is exactly how the rays and claws got
    truncated the first time the Claude mark was drawn."""
    sheet, manifest = built
    fw, fh = manifest["frameWidth"], manifest["frameHeight"]
    for anim in manifest["animations"]:
        for col in range(anim["frames"]):
            cell = sheet.crop(
                (col * fw, anim["row"] * fh, (col + 1) * fw, (anim["row"] + 1) * fh)
            ).getchannel("A")
            edges = {
                "left": cell.crop((0, 0, 1, fh)),
                "right": cell.crop((fw - 1, 0, fw, fh)),
                "top": cell.crop((0, 0, fw, 1)),
                "bottom": cell.crop((0, fh - 1, fw, fh)),
            }
            for name, strip in edges.items():
                assert strip.getextrema() == (0, 0), (
                    f"{anim['name']} frame {col} touches the {name} edge"
                )


def test_frames_within_an_animation_actually_differ(built):
    """A walk cycle whose frames are identical is a still image."""
    sheet, manifest = built
    fw, fh = manifest["frameWidth"], manifest["frameHeight"]
    for anim in manifest["animations"]:
        if anim["frames"] < 2:
            continue
        first = sheet.crop((0, anim["row"] * fh, fw, (anim["row"] + 1) * fh)).tobytes()
        others = [
            sheet.crop((c * fw, anim["row"] * fh, (c + 1) * fw, (anim["row"] + 1) * fh)).tobytes()
            for c in range(1, anim["frames"])
        ]
        assert any(o != first for o in others), f"{anim['name']} does not animate"


def test_fps_and_loop_are_sane(built):
    _, manifest = built
    for anim in manifest["animations"]:
        assert 1 <= anim["fps"] <= 60
        assert isinstance(anim["loop"], bool)
    by_name = {a["name"]: a for a in manifest["animations"]}
    # These play once and hand control back; looping them would trap the crab.
    assert by_name["celebrate"]["loop"] is False
    assert by_name["tumble"]["loop"] is False


def test_builder_frame_count_matches_declaration():
    """build_sheet() asserts this internally; make the failure explicit."""
    for anim in gen_sprites.ANIMATIONS:
        poses = anim["build"](anim["frames"])
        assert len(poses) == anim["frames"], anim["name"]


def test_writes_both_files(tmp_path):
    sheet, manifest = gen_sprites.build_sheet()
    sheet.save(tmp_path / "spritesheet.png")
    (tmp_path / "manifest.json").write_text(json.dumps(manifest))
    assert (tmp_path / "spritesheet.png").stat().st_size > 0
    assert json.loads((tmp_path / "manifest.json").read_text())["animations"]


# --- variants --------------------------------------------------------------


DRESSED = sorted(set(gen_sprites.VARIANTS) - {"default"})


def test_variants_share_one_manifest():
    """The sheets differ only in what is drawn inside a frame. If the layouts
    ever diverge, the single shipped manifest silently mis-indexes one."""
    _, base = gen_sprites.build_sheet("default")
    for variant in DRESSED:
        _, other = gen_sprites.build_sheet(variant)
        assert base == other, variant


@pytest.mark.parametrize("variant", DRESSED)
def test_each_variant_differs_from_default(variant):
    plain, _ = gen_sprites.build_sheet("default")
    dressed, _ = gen_sprites.build_sheet(variant)
    assert plain.tobytes() != dressed.tobytes()


@pytest.mark.parametrize("variant", DRESSED)
def test_each_variant_differs_in_every_frame(variant):
    """Headwear has to survive every pose, including the rotated ones."""
    plain, manifest = gen_sprites.build_sheet("default")
    dressed, _ = gen_sprites.build_sheet(variant)
    fw, fh = manifest["frameWidth"], manifest["frameHeight"]
    for anim in manifest["animations"]:
        for col in range(anim["frames"]):
            box = (col * fw, anim["row"] * fh, (col + 1) * fw, (anim["row"] + 1) * fh)
            assert plain.crop(box).tobytes() != dressed.crop(box).tobytes(), (
                f"{variant}: {anim['name']} frame {col} is identical to default"
            )


@pytest.mark.parametrize("variant", DRESSED)
def test_accessories_use_the_light_colour(variant):
    """GLASS exists so an accessory reads against both the body and the dark
    eye it may sit beside; drawn in EYE alone it would merge into a blob."""
    dressed, _ = gen_sprites.build_sheet(variant)
    assert gen_sprites.GLASS in {c for _, c in dressed.getcolors(maxcolors=1 << 16)}


def test_default_carries_no_accessory_colour():
    plain, _ = gen_sprites.build_sheet("default")
    assert gen_sprites.GLASS not in {c for _, c in plain.getcolors(maxcolors=1 << 16)}


def test_variants_are_distinct_from_each_other():
    """Two hats that render identically would make the menu a lie."""
    rendered = {v: gen_sprites.build_sheet(v)[0].tobytes() for v in gen_sprites.VARIANTS}
    assert len(set(rendered.values())) == len(rendered)


def test_variant_filenames_are_distinct():
    assert len(set(gen_sprites.VARIANTS.values())) == len(gen_sprites.VARIANTS)
    assert "default" in gen_sprites.VARIANTS
