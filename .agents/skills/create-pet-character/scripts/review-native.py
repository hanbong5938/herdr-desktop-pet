#!/usr/bin/env python3
"""Capture the real native v5 catalog for human review (no simulated rendering).

Usage: review-native.py --native BIN --path PACK --output NEW_DIR

Captures every own-pose model (neutral, hit-region overlay and fixed-time clip
samples: entry/middle/near-expiry plus low-blink, mouth and head-angle keys),
every unforced phase and every unforced reaction through the native
``pack preview`` command, then arranges the untouched native PNGs into
light/dark contact sheets. ``NEW_DIR`` must not exist; partial evidence
(PNGs, native stdout/stderr) is kept in ``review.json`` when a step fails.

Requires Pillow only to arrange already-captured native PNGs into contact sheets.
The native pack validator and preview executable remain the sole render authorities.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import traceback

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_config import POSES  # noqa: E402  (the ten herdr v5 semantic IDs)

# --pose proves the own model; independent, unforced semantic captures below
# prove the production phase/effect selection paths separately.
INTENDED = {
    "waiting": ("idle", None), "writing": ("running", None),
    "failed": ("unknown", None), "cancelled": ("unknown", None),
    "disconnected": ("unknown", None), "bored": ("idle", None),
    "happy": ("idle", "completion_observed"),
    "head-tap": ("idle", "head_tap"),
    "torso-tap": ("idle", "body_tap"), "head-pet": ("idle", "pet"),
}
# The host's effect lifetime, not the duration of the asset's reaction clip.
# An effect at precisely this age is already absent from native preview.
EFFECT_LIFETIME_MS = {"head_tap": 460, "body_tap": 520,
                      "pet": 900, "completion_observed": 900}
PHASES = ("idle", "running", "waiting", "unknown")
REACTIONS = tuple(EFFECT_LIFETIME_MS)
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def load_catalog(pack: Path) -> dict:
    manifest = json.loads((pack / "manifest.json").read_text())
    require(manifest.get("version") == 5 and manifest.get("render_mode") == "rig",
            "review requires a native v5 independent-model rig pack")
    entries = [p for p in manifest["payloads"] if p["kind"] == "entry"]
    require(len(entries) == 1, "expected one catalog entry")
    entry = json.loads((pack / entries[0]["path"]).read_text())
    models = {item["id"]: item for item in entry["models"]}
    require(set(models) == set(POSES), "catalog must contain all ten independent pose IDs")
    require(entry["bindings"] == {pose: pose for pose in POSES},
            "catalog bindings must route all ten semantics to their own models")
    motion = {pose: json.loads((pack / models[pose]["motion"]).read_text()) for pose in POSES}
    for pose, (phase, reaction) in INTENDED.items():
        clip = motion[pose]["reactions" if reaction else "phases"][reaction or phase]
        require(clip["pose"] == pose, f"{pose}: intended clip does not select own pose")
        require(isinstance(clip["duration_ms"], int) and clip["duration_ms"] > 0,
                f"{pose}: invalid intended clip duration")
    return motion


def timeline(clip: dict, effect: str | None) -> tuple[dict[str, int], dict[str, str]]:
    duration = clip["duration_ms"]
    last = min(duration - 1, EFFECT_LIFETIME_MS[effect] - 1 if effect else duration - 1)
    transition = clip["transition_ms"]
    times = {"entry": 0, "middle": min(last, max(transition + 1, duration // 2)),
             "near_expiry": last}
    unavailable = {}
    tracks = clip.get("tracks", {})
    candidates = {
        "low_blink": ("eyeOpenL", "eyeOpenR"),
        "mouth": ("mouthOpen", "mouthForm"),
        "head_angle": ("angleZ", "angleX", "angleY"),
    }
    for label, names in candidates.items():
        choices = []
        for name in names:
            for key in tracks.get(name, {}).get("keys", []):
                at, value = key["at_ms"], key["value"]
                if 0 < at <= last and (label != "low_blink" or 0.1 <= value <= 0.3):
                    score = -abs(value - 0.2) if label == "low_blink" else (
                        value if name == "mouthOpen" else abs(value))
                    choices.append((score, at, name, value))
        if choices:
            score, at, name, value = max(choices)
            if label != "low_blink" and score == 0:
                unavailable[label] = "no nonzero key in the active clip window"
            else:
                times[label] = at
        else:
            unavailable[label] = "no matching key before native effect expiry"
    return times, unavailable


def run_native(native: Path, args: list[str], record: dict) -> dict:
    command = [str(native), *args]
    record["command"] = command
    result = subprocess.run(command, capture_output=True, text=True, check=False)
    record.update(returncode=result.returncode, stdout=result.stdout, stderr=result.stderr)
    require(result.returncode == 0, f"native command failed ({result.returncode}): {' '.join(command)}: {result.stderr}")
    try:
        response = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"native command returned invalid JSON: {' '.join(command)}") from error
    require(isinstance(response, dict), "native command JSON must be an object")
    record["native_result"] = response
    return response


def png_info(path: Path, width: int, height: int) -> dict:
    data = path.read_bytes()
    require(len(data) >= 24 and data.startswith(PNG_SIGNATURE), f"not a PNG: {path}")
    size = struct.unpack(">II", data[16:24])
    require(size == (width, height), f"native PNG dimensions {size} differ from validation {(width, height)}")
    return {"path": str(path), "bytes": len(data), "sha256": digest(data),
            "width": size[0], "height": size[1]}


def make_boards(output: Path, captures: list[dict], width: int, height: int) -> list[dict]:
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError as error:
        raise RuntimeError("Pillow is required only for contact sheets; native captures remain intact") from error
    by_name = {capture["name"]: capture for capture in captures}
    font = ImageFont.load_default()
    boards = []
    # Input artwork is always the raw native preview, composited unchanged against
    # light/dark backgrounds. No source screenshot or synthetic face renderer.
    for background, color in (("light", (242, 243, 248)), ("dark", (29, 32, 43))):
        thumb_w, thumb_h = 260, 260
        board = Image.new("RGB", (5 * thumb_w, 2 * (thumb_h + 34)), color)
        painter = ImageDraw.Draw(board)
        for index, pose in enumerate(POSES):
            capture = by_name[f"{pose}-neutral"]
            native = Image.open(capture["output"]["path"]).convert("RGBA")
            native.thumbnail((thumb_w - 12, thumb_h - 12), Image.Resampling.LANCZOS)
            column, row = index % 5, index // 5
            panel = Image.new("RGBA", native.size, (*color, 255))
            panel.alpha_composite(native)
            board.paste(panel.convert("RGB"), (column * thumb_w + (thumb_w - native.width) // 2,
                                               row * (thumb_h + 34) + (thumb_h - native.height) // 2))
            painter.text((column * thumb_w + 6, row * (thumb_h + 34) + thumb_h + 4),
                         f"{pose} | native neutral", fill=(20, 20, 20) if background == "light" else (240, 240, 240), font=font)
        path = output / f"catalog-{background}.png"
        board.save(path)
        boards.append({"kind": "whole_catalog", "background": background,
                       "source_captures": [f"{pose}-neutral" for pose in POSES],
                       "derived": True, "output": png_info(path, board.width, board.height)})
    # Crops are from actual native PNGs, NOT projected geometry/claimed face boxes.
    # Upper half of the visible alpha silhouette is explicitly a detail proxy.
    for background, color in (("light", (242, 243, 248)), ("dark", (29, 32, 43))):
        tile_w, tile_h = 280, 240
        board = Image.new("RGB", (5 * tile_w, 2 * (tile_h + 46)), color)
        painter = ImageDraw.Draw(board)
        details = []
        for index, pose in enumerate(POSES):
            name = next((f"{pose}-{kind}" for kind in ("low_blink", "mouth", "middle")
                         if f"{pose}-{kind}" in by_name))
            native = Image.open(by_name[name]["output"]["path"]).convert("RGBA")
            visible = native.getchannel("A").getbbox()
            require(visible is not None, f"native capture has no visible pixels: {name}")
            left, top, right, bottom = visible
            crop = (left, top, right, top + max(1, (bottom - top) // 2))
            detail = native.crop(crop)
            detail.thumbnail((tile_w - 12, tile_h - 12), Image.Resampling.LANCZOS)
            column, row = index % 5, index // 5
            panel = Image.new("RGBA", detail.size, (*color, 255))
            panel.alpha_composite(detail)
            board.paste(panel.convert("RGB"), (column * tile_w + (tile_w - detail.width) // 2,
                                               row * (tile_h + 46) + (tile_h - detail.height) // 2))
            ink = (20, 20, 20) if background == "light" else (240, 240, 240)
            painter.text((column * tile_w + 5, row * (tile_h + 46) + tile_h + 3),
                         f"{pose} | {name.rsplit('-', 1)[-1]}", fill=ink, font=font)
            painter.text((column * tile_w + 5, row * (tile_h + 46) + tile_h + 20),
                         "upper alpha silhouette (not face bbox)", fill=ink, font=font)
            details.append({"capture": name, "crop_xyxy": crop})
        path = output / f"detail-{background}.png"
        board.save(path)
        boards.append({"kind": "upper_silhouette_detail", "background": background,
                       "crop_method": "top half of captured PNG nontransparent bounding box; not face geometry",
                       "source_crops": details, "derived": True,
                       "output": png_info(path, board.width, board.height)})
    return boards


def review(native: Path, pack: Path, output: Path) -> None:
    require(native.is_file() and pack.is_dir(), "native executable and real pack directory must exist")
    require(not output.exists() and not output.is_symlink(), "output directory must be new")
    output.mkdir(parents=True)
    report = {"version": 1, "status": "incomplete", "pack": str(pack), "native": str(native),
              "native_validation": {}, "captures": [], "boards": [], "errors": [],
              "artistic_approval": "not assessed; requires direct human image inspection"}
    manifest = output / "review.json"
    def persist() -> None:
        temporary = output / "review.json.tmp"
        temporary.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        temporary.replace(manifest)
    persist()
    try:
        validated = run_native(native, ["pack", "validate", "--path", str(pack)], report["native_validation"])
        require(validated.get("command") == "validate" and validated.get("backend") == "rig"
                and validated.get("prepared") is True, "native did not validate a prepared rig")
        width, height = validated["width"], validated["height"]
        require(width > 0 and height > 0, "native validation reported invalid canvas size")
        report["source_pack_digest"] = validated["content_digest"]
        report["viewport_caption"] = (f"Native frozen viewport {width}x{height}, backing scale 1 "
                                      "(from native pack preview; not assumed Retina)")
        persist()
        motion = load_catalog(pack)
        def capture(name: str, phase: str, at: int, *, pose: str | None = None,
                    reaction: str | None = None, age: int | None = None, overlay: bool = False,
                    clip: str | None = None) -> None:
            filename = "waiting-native.png" if name == "waiting-neutral" else f"{name}.png"
            destination = output / filename
            controls = {"phase": phase, "time_ms": at, "pose": pose, "reaction": reaction,
                        "reaction_age_ms": age, "hit_overlay": overlay}
            record = {"name": name, "controls": controls, "clip": clip}
            report["captures"].append(record)
            persist()
            args = ["pack", "preview", "--path", str(pack), "--output", str(destination),
                    "--phase", phase, "--time-ms", str(at)]
            if pose:
                args.extend(("--pose", pose))
            if reaction:
                require(age is not None and age < EFFECT_LIFETIME_MS[reaction],
                        f"{name}: effect would already be expired")
                args.extend(("--reaction", reaction, "--reaction-age-ms", str(age)))
            if overlay:
                args.append("--hit-overlay")
            result = run_native(native, args, record)
            expected = {"command": "preview", "backend": "rig", "path": str(pack),
                        "output": str(destination), "content_digest": report["source_pack_digest"],
                        "width": width, "height": height, **controls}
            for key, value in expected.items():
                require(result.get(key) == value, f"{name}: native response {key} differs: {result.get(key)!r} != {value!r}")
            record["output"] = png_info(destination, width, height)
            require(result["bytes"] == record["output"]["bytes"], f"{name}: native byte count mismatch")
            if name == "waiting-neutral":
                (output / "waiting-native.json").write_text(record["stdout"])
            persist()

        # Own-source independent poses, one paired neutral overlay per model
        # containing both sampled head and torso hit regions (not ten repeated
        # unforced phase/reaction captures).
        for pose in POSES:
            phase, effect = INTENDED[pose]
            capture(f"{pose}-neutral", phase, 0, pose=pose, clip="own_pose_neutral")
            capture(f"{pose}-hit-regions", phase, 0, pose=pose, overlay=True,
                    clip="native_head_and_torso_hit_regions")
            clip = motion[pose]["reactions" if effect else "phases"][effect or phase]
            times, unavailable = timeline(clip, effect)
            report.setdefault("timeline", {})[pose] = {
                "clip": f"{effect or phase}", "duration_ms": clip["duration_ms"],
                "transition_ms": clip["transition_ms"], "native_effect_lifetime_ms": EFFECT_LIFETIME_MS.get(effect),
                "sample_times_ms": times, "unavailable_samples": unavailable,
            }
            persist()
            for label, at in times.items():
                # Preserve labels of coincident key moments without repeating a
                # costly native render of exactly the same controls.
                prior = next((f"{pose}-{existing}" for existing, time in times.items()
                              if existing != label and time == at and f"{pose}-{existing}" in
                              {row["name"] for row in report["captures"]}), None)
                if prior:
                    report["timeline"][pose].setdefault("aliases", {})[label] = prior
                    persist()
                    continue
                capture(f"{pose}-{label}", phase, at, pose=pose, reaction=effect,
                        age=at if effect else None, clip=f"own_{effect or phase}")

        # Separate unforced semantic path coverage. Phase 'waiting' is distinct
        # from idle; failed/cancelled/disconnected/bored are direct --pose because
        # CLI preview has no outcome/disconnected/long-idle controls.
        for phase in PHASES:
            if phase == "idle":
                at = min(1300, motion["waiting"]["phases"]["idle"]["duration_ms"] - 1)
            else:
                at = 0
            capture(f"semantic-phase-{phase}", phase, at, clip=f"unforced_phase_{phase}")
        for reaction in REACTIONS:
            at = min(200, EFFECT_LIFETIME_MS[reaction] - 1)
            capture(f"semantic-reaction-{reaction}", "idle", at,
                    reaction=reaction, age=at, clip=f"unforced_reaction_{reaction}")
        report["boards"] = make_boards(output, report["captures"], width, height)
        report["status"] = "captured_for_human_review"
        persist()
    except Exception as error:
        report["errors"].append({"message": str(error), "traceback": traceback.format_exc()})
        persist()  # Keep partial native evidence, including PNGs and failed CLI stdout/stderr.
        raise


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", required=True, type=Path)
    parser.add_argument("--path", required=True, type=Path, help="actual finished v5 pack directory")
    parser.add_argument("--output", required=True, type=Path, help="new directory (never overwritten)")
    args = parser.parse_args()
    review(args.native.absolute(), args.path.absolute(), args.output.absolute())
    print(args.output.absolute() / "review.json")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"native catalog review failed: {error}", file=sys.stderr)
        sys.exit(1)
