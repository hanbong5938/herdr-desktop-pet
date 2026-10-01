#!/usr/bin/env python3
"""Adapt a canonical 1280px independent rig round to native 1024px assets.

The canonical source round is immutable input. Geometry-only export must be
requested explicitly with ``--raster-only`` and is not a complete adaptation.
For motion, pass ``--motion-profile profile.json``. The profile is an exact
object with ``version: 1``, the selected artistic ``pose`` ID, and complete
``phases``/``reactions`` maps. Every native slot maps explicitly to
``{"model": "path/from/upstream/root/model.json"|"selected",
"transition_ms": integer}``; no slot or artistic intent is inferred.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
from typing import Any

try:
    from PIL import Image
except ImportError:  # pragma: no cover - the creator environment supplies Pillow.
    Image = None  # type: ignore[assignment]


INPUT_SIZE = 1280
OUTPUT_SIZE = 1024
SCALE = OUTPUT_SIZE / INPUT_SIZE
MAX_REFERENCE_ENTRIES = 64
MAX_PROFILE_SAMPLES = 64
MAX_POLYGON_POINTS = 64
MAX_NUMBER = 1e9
PHASES = ("idle", "running", "waiting", "unknown")
REACTIONS = ("head_tap", "body_tap", "pet", "completion_observed")
GLOBAL_PARAMETER_BOUNDS = {
    **{name: (-1.0, 1.0) for name in (
        "angleX", "angleY", "angleZ", "eyeX", "eyeY", "brow", "browAngL",
        "browAngR", "browAngSym", "mouthForm", "mouthCY", "body", "armY",
        "armPos", "bangL", "bangC", "bangR", "eyeCY", "eyeCAng", "mouthCAng",
    )},
    **{name: (0.0, 1.0) for name in (
        "eyeOpenL", "eyeOpenR", "mouthOpen", "eyeEase", "mouthEase",
    )},
    "irisScale": (0.5, 1.3),
    "physAmp": (0.0, 3.0),
    "soft": (0.0, 3.0),
    "fhAmp": (0.0, 3.0),
    "bust": (0.0, 4.0),
    "bustY": (-3.0, 3.0),
    "fhSoft": (0.0, 2.0),
    "eyeScaleL": (0.5, 1.5),
    "eyeScaleR": (0.5, 1.5),
    "mouthScale": (0.5, 1.5),
}
LOCAL_TRACK_BOUNDS = {
    "translateX": (-float(OUTPUT_SIZE), float(OUTPUT_SIZE)),
    "translateY": (-float(OUTPUT_SIZE), float(OUTPUT_SIZE)),
    "rotationDeg": (-360.0, 360.0),
    "scale": (0.0, 4.0),
}
RESERVED_OUTPUT_FILES = {
    "layers.json",
    "rig-overrides.json",
    "provenance.json",
    "model.psd",
    "model-authoring.psd",
    "neutral-authored.png",
    "PSD-order-audit.json",
    "motion.json",
}

ROOT_FIELDS = {
    "layerAliases",
    "layerOrder",
    "layerOrderConstraints",
    "maskedLayerOverlays",
    "interpolatedPatchRepairs",
    "depthOverrides",
    "groupOverrides",
    "deformationSources",
    "meshSources",
    "hairAttachments",
    "headFollow",
    "cleanupThresholds",
    "hiddenLayers",
    "excludeAfterMeshResolution",
    "anchorOverrides",
    "interactionAreas",
    "mouthExpressions",
    "blinkRepair",
    "hairSplit",
    "physics",
}

LAYER_META_FIELDS = {"prefix", "timestamp", "width", "height", "layers"}
LAYER_ROW_FIELDS = {
    "name",
    "filename",
    "left",
    "top",
    "right",
    "bottom",
    "clipTo",
    "editorHidden",
}
ADAPTER_COMMAND = [
    "python3",
    "tools/adapt-rig-source.py",
    "--model",
    "<round>",
    "--output",
    "<newdir>",
    "--upstream-root",
    "<upstream-root>",
    "--size1024",
    "--motion-profile",
    "<profile.json>",
]

SCALED_FIELDS = [
    "layers.json.width",
    "layers.json.height",
    "layers[].left",
    "layers[].top",
    "layers[].right",
    "layers[].bottom",
    "rig-overrides.maskedLayerOverlays[].featherPx",
    "rig-overrides.hairAttachments.*.rootY",
    "rig-overrides.hairAttachments.*.bodyY",
    "rig-overrides.headFollow.*.center.cx",
    "rig-overrides.headFollow.*.center.cy",
    "rig-overrides.headFollow.*.radius",
    "rig-overrides.headFollow.*.falloffRadius",
    "rig-overrides.anchorOverrides.face.x0/y0/x1/y1/cx/cy",
    "rig-overrides.anchorOverrides.eyeL/eyeR.x0/y0/x1/y1/icx/icy/closeY",
    "rig-overrides.anchorOverrides.eyeL/eyeR.blink.center.cx/cy",
    "rig-overrides.anchorOverrides.eyeL/eyeR.blink.u0/u1",
    "rig-overrides.anchorOverrides.eyeL/eyeR.blink.upper/lower/closed[]",
    "rig-overrides.anchorOverrides.eyeL/eyeR.blink.closedSource.cx/cy",
    "rig-overrides.anchorOverrides.eyeL/eyeR.blink.closedTarget.cx/cy",
    "rig-overrides.anchorOverrides.mouth.x0/y0/x1/y1/cx/cy",
    "rig-overrides.anchorOverrides.mouth.morph.center.cx/cy",
    "rig-overrides.anchorOverrides.mouth.morph.neutral/open/smile.u0/u1",
    "rig-overrides.anchorOverrides.mouth.morph.neutral/open/smile.upper/lower[]",
    "rig-overrides.anchorOverrides.neckPivot/bodyPivot.cx/cy",
    "rig-overrides.anchorOverrides.neckTop/neckBottom/hairRootY",
    "rig-overrides.interactionAreas.*.x0/y0/x1/y1",
    "rig-overrides.blinkRepair.paddingX/paddingY",
    "rig-overrides.hairSplit.centerX",
    "rig-overrides.physics.frontHair/backHair/layers.*.maxOffset",
]


class AdapterError(Exception):
    """An expected, user-facing adapter failure."""


def fail(message: str) -> None:
    raise AdapterError(message)


def reject_constant(value: str) -> Any:
    fail(f"JSON contains non-finite number {value}")


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            fail(f"JSON contains duplicate key {key!r}")
        result[key] = value
    return result


def load_json(path: Path, label: str) -> Any:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        fail(f"cannot read {label} {path.name}: {error}")
    try:
        value = json.loads(
            text,
            object_pairs_hook=reject_duplicate_keys,
            parse_constant=reject_constant,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, AdapterError) as error:
        fail(f"{label} is invalid JSON: {error}")
    reject_nulls(value, label)
    return value


def reject_nulls(value: Any, label: str) -> None:
    if value is None:
        fail(f"{label} contains null; omit optional fields instead")
    if isinstance(value, list):
        for index, child in enumerate(value):
            reject_nulls(child, f"{label}[{index}]")
    elif isinstance(value, dict):
        for key, child in value.items():
            reject_nulls(child, f"{label}.{key}")


def write_json(path: Path, value: Any) -> None:
    try:
        path.write_text(
            json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + "\n",
            encoding="utf-8",
        )
    except (OSError, TypeError, ValueError) as error:
        fail(f"cannot write {path.name}: {error}")


def expect_dict(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{path} must be an object")
    return value


def expect_list(value: Any, path: str, minimum: int = 0, maximum: int | None = None) -> list[Any]:
    if not isinstance(value, list):
        fail(f"{path} must be an array")
    if len(value) < minimum or (maximum is not None and len(value) > maximum):
        fail(f"{path} array length is outside [{minimum}, {maximum if maximum is not None else 'unbounded'}]")
    return value


def expect_string(value: Any, path: str) -> str:
    if not isinstance(value, str) or not value or "\x00" in value:
        fail(f"{path} must be a non-empty string")
    return value


def expect_bool(value: Any, path: str) -> bool:
    if not isinstance(value, bool):
        fail(f"{path} must be boolean")
    return value


def expect_number(value: Any, path: str) -> int | float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        fail(f"{path} must be a finite number")
    try:
        finite = math.isfinite(value)
    except (OverflowError, TypeError):
        finite = False
    if not finite or abs(value) > MAX_NUMBER:
        fail(f"{path} must be finite and bounded")
    return value


def expect_integer(value: Any, path: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        fail(f"{path} must be an integer")
    return value


def exact_object(
    value: Any,
    path: str,
    allowed: set[str],
    *,
    required: set[str] | frozenset[str] = frozenset(),
) -> dict[str, Any]:
    result = expect_dict(value, path)
    unknown = sorted(set(result) - allowed)
    if unknown:
        fail(f"{path}.{unknown[0]} is an unsupported field")
    missing = sorted(required - set(result))
    if missing:
        fail(f"{path}.{missing[0]} is required")
    return result


def bounded_map(value: Any, path: str) -> dict[str, Any]:
    result = expect_dict(value, path)
    if len(result) > MAX_REFERENCE_ENTRIES:
        fail(f"{path} contains too many references")
    for key in result:
        expect_string(key, f"{path} key")
    return result


def bounded_name_array(value: Any, path: str) -> list[str]:
    values = expect_list(value, path, 0, MAX_REFERENCE_ENTRIES)
    return [expect_string(item, f"{path}[{index}]") for index, item in enumerate(values)]



def normalized(value: str) -> str:
    # Python has no str.normalize method; keep this separate for readability.
    import unicodedata

    return unicodedata.normalize("NFKC", value).strip().lower()


def base_name(value: str) -> str:
    import re

    result = normalized(value)
    return re.sub(r"[-_]([lr])$", "", re.sub(r"_\d+$", "", result))


def loader_canonical_name(value: str) -> str:
    import re

    result = normalized(value)
    match = re.match(r"^(eyewhite|irides|eyelash|eyebrow|eye_close)[-_](l|r)$", result)
    if match:
        return match.group(1)
    match = re.match(r"^(handwear|ears)[-_](l|r)$", result)
    if match:
        return f"{match.group(1)}_{1 if match.group(2) == 'l' else 2}"
    return result


def compact_number(value: float | int) -> int | float:
    if abs(float(value)) < 1e-12:
        return 0
    rounded = round(float(value), 12)
    if rounded.is_integer():
        return int(rounded)
    return rounded


def scale_coordinate(value: Any, path: str) -> int | float:
    number = expect_number(value, path)
    if number < 0 or number > INPUT_SIZE:
        fail(f"{path} coordinate {number!r} is outside the 1280 canvas")
    output = float(number) * SCALE
    if output < 0 or output > OUTPUT_SIZE:
        fail(f"{path} coordinate {number!r} does not fit the 1024 canvas")
    return compact_number(output)
def scale_profile_pixel(value: Any, path: str) -> int | float:
    """Scale a signed local pixel offset (u/v profile samples)."""
    number = expect_number(value, path)
    if number < -INPUT_SIZE or number > INPUT_SIZE:
        fail(f"{path} local pixel value {number!r} exceeds the 1280 bound")
    return compact_number(float(number) * SCALE)


def scale_nonnegative_pixel(value: Any, path: str) -> int | float:
    number = expect_number(value, path)
    if number < 0 or number > INPUT_SIZE:
        fail(f"{path} pixel value {number!r} is outside the 1280 canvas bound")
    return compact_number(float(number) * SCALE)


def keep_number(value: Any, path: str) -> int | float:
    return expect_number(value, path)


def point(value: Any, path: str, *, scale: bool = True) -> dict[str, Any]:
    fields = {"cx", "cy"}
    data = exact_object(value, path, fields, required=fields)
    transform = scale_coordinate if scale else keep_number
    return {
        "cx": transform(data["cx"], f"{path}.cx"),
        "cy": transform(data["cy"], f"{path}.cy"),
    }


def bounds(value: Any, path: str) -> dict[str, Any]:
    fields = {"x0", "y0", "x1", "y1"}
    data = exact_object(value, path, fields, required=fields)
    result = {
        "x0": scale_coordinate(data["x0"], f"{path}.x0"),
        "y0": scale_coordinate(data["y0"], f"{path}.y0"),
        "x1": scale_coordinate(data["x1"], f"{path}.x1"),
        "y1": scale_coordinate(data["y1"], f"{path}.y1"),
    }
    if result["x1"] < result["x0"] or result["y1"] < result["y0"]:
        fail(f"{path} has inverted bounds")
    return result


def polygon(value: Any, path: str) -> list[list[int | float]]:
    """Preserve the canvas fractions consumed by the native shared rig loader."""
    values = expect_list(value, path, 3, MAX_POLYGON_POINTS)
    result: list[list[int | float]] = []
    for index, item in enumerate(values):
        pair = expect_list(item, f"{path}[{index}]", 2, 2)
        point = [
            keep_number(pair[0], f"{path}[{index}][0]"),
            keep_number(pair[1], f"{path}[{index}][1]"),
        ]
        if any(coordinate < 0 or coordinate > 1 for coordinate in point):
            fail(f"{path}[{index}] polygon coordinates must be canvas fractions in [0,1]")
        result.append(point)
    return result


def profile_array(value: Any, path: str) -> list[int | float]:
    values = expect_list(value, path, 2, MAX_PROFILE_SAMPLES)
    return [scale_profile_pixel(item, f"{path}[{index}]") for index, item in enumerate(values)]


def mouth_shape(value: Any, path: str, *, closed: bool) -> dict[str, Any]:
    fields = {"u0", "u1", "upper", "lower"}
    data = exact_object(value, path, fields, required=fields)
    result = {
        "u0": scale_profile_pixel(data["u0"], f"{path}.u0"),
        "u1": scale_profile_pixel(data["u1"], f"{path}.u1"),
        "upper": profile_array(data["upper"], f"{path}.upper"),
        "lower": profile_array(data["lower"], f"{path}.lower"),
    }
    if result["u1"] <= result["u0"]:
        fail(f"{path}.u1 must be greater than u0")
    if len(result["upper"]) != len(result["lower"]):
        fail(f"{path} curve lengths must match")
    for index, (upper, lower) in enumerate(zip(result["upper"], result["lower"])):
        if lower < upper or (not closed and lower == upper):
            relation = "at or above" if closed else "above"
            fail(f"{path}.lower[{index}] must stay {relation} upper")
    return result


def eye_blink(value: Any, path: str) -> dict[str, Any]:
    fields = {
        "center",
        "angleDeg",
        "u0",
        "u1",
        "upper",
        "lower",
        "closed",
        "closedSource",
        "closedTarget",
        "closedRotationDeg",
    }
    data = exact_object(value, path, fields, required=fields)
    result = {
        "center": point(data["center"], f"{path}.center"),
        "angleDeg": keep_number(data["angleDeg"], f"{path}.angleDeg"),
        "u0": scale_profile_pixel(data["u0"], f"{path}.u0"),
        "u1": scale_profile_pixel(data["u1"], f"{path}.u1"),
        "upper": profile_array(data["upper"], f"{path}.upper"),
        "lower": profile_array(data["lower"], f"{path}.lower"),
        "closed": profile_array(data["closed"], f"{path}.closed"),
        "closedSource": point(data["closedSource"], f"{path}.closedSource"),
        "closedTarget": point(data["closedTarget"], f"{path}.closedTarget"),
        "closedRotationDeg": keep_number(data["closedRotationDeg"], f"{path}.closedRotationDeg"),
    }
    if result["u1"] <= result["u0"]:
        fail(f"{path}.u1 must be greater than u0")
    if not (len(result["upper"]) == len(result["lower"]) == len(result["closed"])):
        fail(f"{path} curve lengths must match")
    for index, (upper, lower) in enumerate(zip(result["upper"], result["lower"])):
        if lower <= upper:
            fail(f"{path}.lower[{index}] must stay above upper")
    return result


def eye_anchor(value: Any, path: str) -> dict[str, Any]:
    required = {"x0", "y0", "x1", "y1", "icx", "icy", "closeY"}
    data = exact_object(value, path, required | {"blink"}, required=required)
    result = {
        "x0": scale_coordinate(data["x0"], f"{path}.x0"),
        "y0": scale_coordinate(data["y0"], f"{path}.y0"),
        "x1": scale_coordinate(data["x1"], f"{path}.x1"),
        "y1": scale_coordinate(data["y1"], f"{path}.y1"),
        "icx": scale_coordinate(data["icx"], f"{path}.icx"),
        "icy": scale_coordinate(data["icy"], f"{path}.icy"),
        "closeY": scale_coordinate(data["closeY"], f"{path}.closeY"),
    }
    if result["x1"] < result["x0"] or result["y1"] < result["y0"]:
        fail(f"{path} has inverted bounds")
    if "blink" in data:
        result["blink"] = eye_blink(data["blink"], f"{path}.blink")
    return result


def mouth_morph(value: Any, path: str) -> dict[str, Any]:
    fields = {"center", "angleDeg", "neutral", "open", "smile"}
    data = exact_object(value, path, fields, required=fields)
    return {
        "center": point(data["center"], f"{path}.center"),
        "angleDeg": keep_number(data["angleDeg"], f"{path}.angleDeg"),
        "neutral": mouth_shape(data["neutral"], f"{path}.neutral", closed=True),
        "open": mouth_shape(data["open"], f"{path}.open", closed=False),
        "smile": mouth_shape(data["smile"], f"{path}.smile", closed=True),
    }
def mouth_anchor(value: Any, path: str) -> dict[str, Any]:
    required = {"x0", "y0", "x1", "y1", "cx", "cy"}
    data = exact_object(value, path, required | {"morph"}, required=required)
    result = {
        "x0": scale_coordinate(data["x0"], f"{path}.x0"),
        "y0": scale_coordinate(data["y0"], f"{path}.y0"),
        "x1": scale_coordinate(data["x1"], f"{path}.x1"),
        "y1": scale_coordinate(data["y1"], f"{path}.y1"),
        "cx": scale_coordinate(data["cx"], f"{path}.cx"),
        "cy": scale_coordinate(data["cy"], f"{path}.cy"),
    }
    if result["x1"] < result["x0"] or result["y1"] < result["y0"]:
        fail(f"{path} has inverted bounds")
    if "morph" in data:
        result["morph"] = mouth_morph(data["morph"], f"{path}.morph")
    return result


def anchor_overrides(value: Any, path: str) -> dict[str, Any]:
    data = exact_object(
        value,
        path,
        {
            "face",
            "eyeL",
            "eyeR",
            "mouth",
            "neckPivot",
            "bodyPivot",
            "neckTop",
            "neckBottom",
            "hairRootY",
            "faceScale",
        },
    )
    result: dict[str, Any] = {}
    if "face" in data:
        face_fields = {"x0", "y0", "x1", "y1", "cx", "cy"}
        face = exact_object(data["face"], f"{path}.face", face_fields, required=face_fields)
        result["face"] = {
            key: scale_coordinate(face[key], f"{path}.face.{key}")
            for key in ("x0", "y0", "x1", "y1", "cx", "cy")
        }
        if result["face"]["x1"] < result["face"]["x0"] or result["face"]["y1"] < result["face"]["y0"]:
            fail(f"{path}.face has inverted bounds")
    if "eyeL" in data:
        result["eyeL"] = eye_anchor(data["eyeL"], f"{path}.eyeL")
    if "eyeR" in data:
        result["eyeR"] = eye_anchor(data["eyeR"], f"{path}.eyeR")
    if "mouth" in data:
        result["mouth"] = mouth_anchor(data["mouth"], f"{path}.mouth")
    for key in ("neckPivot", "bodyPivot"):
        if key in data:
            result[key] = point(data[key], f"{path}.{key}")
    for key in ("neckTop", "neckBottom", "hairRootY"):
        if key in data:
            result[key] = scale_coordinate(data[key], f"{path}.{key}")
    if "faceScale" in data:
        value_number = expect_number(data["faceScale"], f"{path}.faceScale")
        if value_number <= 0:
            fail(f"{path}.faceScale must be positive")
        result["faceScale"] = value_number
    return result


def tuning(value: Any, path: str) -> dict[str, Any]:
    fields = ("amplitude", "stiffness", "damping", "wind", "inertia", "rootLock", "maxOffset")
    data = exact_object(value, path, set(fields))
    result: dict[str, Any] = {}
    for key in fields:
        if key not in data:
            continue
        if key == "maxOffset":
            result[key] = scale_nonnegative_pixel(data[key], f"{path}.{key}")
        else:
            result[key] = keep_number(data[key], f"{path}.{key}")
    return result
def transform_overrides(value: Any) -> dict[str, Any]:
    source = expect_dict(value, "rig-overrides")
    unknown = sorted(set(source) - ROOT_FIELDS)
    if unknown:
        fail(f"rig-overrides.{unknown[0]} is an unsupported field; refusing unknown dimensional schema")
    result: dict[str, Any] = {}

    if "layerAliases" in source:
        entries = bounded_map(source["layerAliases"], "rig-overrides.layerAliases")
        result["layerAliases"] = {key: expect_string(item, f"rig-overrides.layerAliases.{key}") for key, item in entries.items()}
    if "layerOrder" in source:
        result["layerOrder"] = bounded_name_array(source["layerOrder"], "rig-overrides.layerOrder")
    if "layerOrderConstraints" in source:
        constraints = expect_list(source["layerOrderConstraints"], "rig-overrides.layerOrderConstraints", 0, MAX_REFERENCE_ENTRIES)
        output = []
        for index, item in enumerate(constraints):
            path = f"rig-overrides.layerOrderConstraints[{index}]"
            fields = {"behind", "inFrontOf"}
            data = exact_object(item, path, fields, required=fields)
            output.append({
                "behind": expect_string(data["behind"], f"{path}.behind"),
                "inFrontOf": expect_string(data["inFrontOf"], f"{path}.inFrontOf"),
            })
        result["layerOrderConstraints"] = output
    if "maskedLayerOverlays" in source:
        overlays = expect_list(source["maskedLayerOverlays"], "rig-overrides.maskedLayerOverlays", 0, MAX_REFERENCE_ENTRIES)
        output = []
        for index, item in enumerate(overlays):
            path = f"rig-overrides.maskedLayerOverlays[{index}]"
            data = exact_object(
                item,
                path,
                {"source", "name", "inFrontOf", "polygon", "replaceExisting", "featherPx", "textureSource", "excludeConnectedNeutral"},
                required={"source", "name", "inFrontOf", "polygon"},
            )
            entry: dict[str, Any] = {
                "source": expect_string(data["source"], f"{path}.source"),
                "name": expect_string(data["name"], f"{path}.name"),
                "inFrontOf": expect_string(data["inFrontOf"], f"{path}.inFrontOf"),
                "polygon": polygon(data["polygon"], f"{path}.polygon"),
            }
            if "replaceExisting" in data:
                entry["replaceExisting"] = expect_bool(data["replaceExisting"], f"{path}.replaceExisting")
            if "featherPx" in data:
                entry["featherPx"] = scale_nonnegative_pixel(data["featherPx"], f"{path}.featherPx")
            if "textureSource" in data:
                entry["textureSource"] = expect_string(data["textureSource"], f"{path}.textureSource")
            if "excludeConnectedNeutral" in data:
                neutral_fields = {"maxChroma", "minLuminance", "maxLuminance", "maxColorStep"}
                neutral = exact_object(
                    data["excludeConnectedNeutral"],
                    f"{path}.excludeConnectedNeutral",
                    neutral_fields,
                    required=neutral_fields,
                )
                entry["excludeConnectedNeutral"] = {
                    key: keep_number(neutral[key], f"{path}.excludeConnectedNeutral.{key}")
                    for key in ("maxChroma", "minLuminance", "maxLuminance", "maxColorStep")
                }
                if entry["excludeConnectedNeutral"]["maxChroma"] < 0 or entry["excludeConnectedNeutral"]["minLuminance"] < 0 or entry["excludeConnectedNeutral"]["maxLuminance"] < 0 or entry["excludeConnectedNeutral"]["maxColorStep"] < 0:
                    fail(f"{path}.excludeConnectedNeutral thresholds must be non-negative")
                if entry["excludeConnectedNeutral"]["maxLuminance"] < entry["excludeConnectedNeutral"]["minLuminance"]:
                    fail(f"{path}.excludeConnectedNeutral luminance bounds are inverted")
            output.append(entry)
        result["maskedLayerOverlays"] = output
    if "interpolatedPatchRepairs" in source:
        repairs = expect_list(source["interpolatedPatchRepairs"], "rig-overrides.interpolatedPatchRepairs", 0, MAX_REFERENCE_ENTRIES)
        output = []
        for index, item in enumerate(repairs):
            path = f"rig-overrides.interpolatedPatchRepairs[{index}]"
            fields = {"layer", "polygon", "axis"}
            data = exact_object(item, path, fields, required=fields)
            axis = expect_string(data["axis"], f"{path}.axis")
            if axis != "horizontal":
                fail(f"{path}.axis must be horizontal")
            output.append({
                "layer": expect_string(data["layer"], f"{path}.layer"),
                "polygon": polygon(data["polygon"], f"{path}.polygon"),
                "axis": axis,
            })
        result["interpolatedPatchRepairs"] = output
    for key in ("depthOverrides", "cleanupThresholds"):
        if key in source:
            entries = bounded_map(source[key], f"rig-overrides.{key}")
            result[key] = {name: keep_number(item, f"rig-overrides.{key}.{name}") for name, item in entries.items()}
    for key in ("groupOverrides", "deformationSources", "meshSources"):
        if key in source:
            entries = bounded_map(source[key], f"rig-overrides.{key}")
            result[key] = {name: expect_string(item, f"rig-overrides.{key}.{name}") for name, item in entries.items()}
    for key in ("hiddenLayers", "excludeAfterMeshResolution"):
        if key in source:
            result[key] = bounded_name_array(source[key], f"rig-overrides.{key}")
    if "hairAttachments" in source:
        entries = bounded_map(source["hairAttachments"], "rig-overrides.hairAttachments")
        output = {}
        for name, item in entries.items():
            path = f"rig-overrides.hairAttachments.{name}"
            fields = {"rootY", "bodyY"}
            data = exact_object(item, path, fields, required=fields)
            root_y = scale_coordinate(data["rootY"], f"{path}.rootY")
            body_y = scale_coordinate(data["bodyY"], f"{path}.bodyY")
            if body_y <= root_y:
                fail(f"{path}.bodyY must be greater than rootY")
            output[name] = {"rootY": root_y, "bodyY": body_y}
        result["hairAttachments"] = output
    if "headFollow" in source:
        entries = bounded_map(source["headFollow"], "rig-overrides.headFollow")
        output = {}
        for name, item in entries.items():
            path = f"rig-overrides.headFollow.{name}"
            fields = {"center", "radius", "falloffRadius"}
            data = exact_object(item, path, fields, required=fields)
            radius = scale_nonnegative_pixel(data["radius"], f"{path}.radius")
            falloff = scale_nonnegative_pixel(data["falloffRadius"], f"{path}.falloffRadius")
            if radius <= 0:
                fail(f"{path}.radius must be positive")
            if falloff <= radius:
                fail(f"{path}.falloffRadius must be greater than radius")
            output[name] = {
                "center": point(data["center"], f"{path}.center"),
                "radius": radius,
                "falloffRadius": falloff,
            }
        result["headFollow"] = output
    if "anchorOverrides" in source:
        result["anchorOverrides"] = anchor_overrides(source["anchorOverrides"], "rig-overrides.anchorOverrides")
    if "interactionAreas" in source:
        entries = exact_object(source["interactionAreas"], "rig-overrides.interactionAreas", {"face", "head", "torso"})
        result["interactionAreas"] = {name: bounds(item, f"rig-overrides.interactionAreas.{name}") for name, item in entries.items()}
    if "mouthExpressions" in source:
        expression_fields = {"neutral", "open", "smile"}
        expressions = exact_object(
            source["mouthExpressions"],
            "rig-overrides.mouthExpressions",
            expression_fields,
            required=expression_fields,
        )
        result["mouthExpressions"] = {
            name: expect_string(expressions[name], f"rig-overrides.mouthExpressions.{name}")
            for name in ("neutral", "open", "smile")
        }
    if "blinkRepair" in source:
        repair = exact_object(source["blinkRepair"], "rig-overrides.blinkRepair", {"enabled", "paddingX", "paddingY"})
        output: dict[str, Any] = {}
        if "enabled" in repair:
            output["enabled"] = expect_bool(repair["enabled"], "rig-overrides.blinkRepair.enabled")
        for key in ("paddingX", "paddingY"):
            if key in repair:
                output[key] = scale_nonnegative_pixel(repair[key], f"rig-overrides.blinkRepair.{key}")
        result["blinkRepair"] = output
    if "hairSplit" in source:
        split = exact_object(source["hairSplit"], "rig-overrides.hairSplit", {"backHairLeftRight", "centerX"})
        output = {}
        if "backHairLeftRight" in split:
            output["backHairLeftRight"] = expect_bool(split["backHairLeftRight"], "rig-overrides.hairSplit.backHairLeftRight")
        if "centerX" in split:
            output["centerX"] = scale_coordinate(split["centerX"], "rig-overrides.hairSplit.centerX")
        result["hairSplit"] = output
    if "physics" in source:
        physics = exact_object(source["physics"], "rig-overrides.physics", {"frontHair", "backHair", "layers"})
        output: dict[str, Any] = {}
        for key in ("frontHair", "backHair"):
            if key in physics:
                output[key] = tuning(physics[key], f"rig-overrides.physics.{key}")
        if "layers" in physics:
            entries = bounded_map(physics["layers"], "rig-overrides.physics.layers")
            output["layers"] = {name: tuning(item, f"rig-overrides.physics.layers.{name}") for name, item in entries.items()}
        result["physics"] = output
    return result


def is_omitted_name(name: str, omitted: set[str]) -> bool:
    return normalized(name) in omitted


def clean_omitted_references(overrides: dict[str, Any], omitted_names: set[str]) -> list[str]:
    """Remove only references to layers this adapter actually omitted."""
    if not omitted_names:
        return []
    removed: list[str] = []

    def omitted(name: str) -> bool:
        return is_omitted_name(name, omitted_names)

    if "layerOrder" in overrides:
        old = overrides["layerOrder"]
        overrides["layerOrder"] = [name for name in old if not omitted(name)]
        removed.extend(f"layerOrder[{index}]" for index, name in enumerate(old) if omitted(name))
    if "layerOrderConstraints" in overrides:
        old = overrides["layerOrderConstraints"]
        kept = []
        for index, item in enumerate(old):
            if omitted(item["behind"]) or omitted(item["inFrontOf"]):
                removed.append(f"layerOrderConstraints[{index}]")
            else:
                kept.append(item)
        overrides["layerOrderConstraints"] = kept
    if "maskedLayerOverlays" in overrides:
        old = overrides["maskedLayerOverlays"]
        kept = []
        for index, item in enumerate(old):
            refs = [item["source"], item["name"], item["inFrontOf"]]
            if "textureSource" in item:
                refs.append(item["textureSource"])
            if any(omitted(name) for name in refs):
                removed.append(f"maskedLayerOverlays[{index}]")
            else:
                kept.append(item)
        overrides["maskedLayerOverlays"] = kept
    if "interpolatedPatchRepairs" in overrides:
        old = overrides["interpolatedPatchRepairs"]
        overrides["interpolatedPatchRepairs"] = [
            item for item in old if not omitted(item["layer"])
        ]
        removed.extend(
            f"interpolatedPatchRepairs[{index}]"
            for index, item in enumerate(old)
            if omitted(item["layer"])
        )
    for key in ("depthOverrides", "groupOverrides", "hairAttachments", "headFollow", "cleanupThresholds"):
        if key not in overrides:
            continue
        old = overrides[key]
        overrides[key] = {name: item for name, item in old.items() if not omitted(name)}
        removed.extend(f"{key}.{name}" for name in old if omitted(name))
    for key in ("hiddenLayers", "excludeAfterMeshResolution"):
        if key not in overrides:
            continue
        old = overrides[key]
        overrides[key] = [name for name in old if not omitted(name)]
        removed.extend(f"{key}[{index}]" for index, name in enumerate(old) if omitted(name))
    for key in ("deformationSources", "meshSources"):
        if key not in overrides:
            continue
        old = overrides[key]
        overrides[key] = {
            name: target
            for name, target in old.items()
            if not omitted(name) and not omitted(target)
        }
        removed.extend(
            f"{key}.{name}"
            for name, target in old.items()
            if omitted(name) or omitted(target)
        )
    if "layerAliases" in overrides:
        old = overrides["layerAliases"]
        overrides["layerAliases"] = {
            name: target
            for name, target in old.items()
            if not omitted(name) and not omitted(target)
        }
        removed.extend(
            f"layerAliases.{name}"
            for name, target in old.items()
            if omitted(name) or omitted(target)
        )
    if "mouthExpressions" in overrides:
        expressions = overrides["mouthExpressions"]
        if any(omitted(name) for name in expressions.values()):
            removed.append("mouthExpressions")
            del overrides["mouthExpressions"]
    if "physics" in overrides and "layers" in overrides["physics"]:
        old = overrides["physics"]["layers"]
        overrides["physics"]["layers"] = {name: item for name, item in old.items() if not omitted(name)}
        removed.extend(f"physics.layers.{name}" for name in old if omitted(name))
    return removed


def check_resolved_reference(name: str, available: list[str], path: str, *, exact: bool = False) -> None:
    normalized_names = {normalized(item) for item in available}
    canonical_names = {loader_canonical_name(item) for item in available}
    requested = normalized(name)
    if requested in normalized_names or loader_canonical_name(name) in canonical_names:
        return
    if not exact and any(base_name(item) == requested for item in available):
        return
    fail(f"{path} does not resolve to an output layer")


def reference_layer_sets(overrides: dict[str, Any], raw_names: list[str]) -> tuple[list[str], list[str]]:
    """Approximate decoder raw/loader names for pre-export reference checks."""
    loader_names: list[str] = []
    split_hair = overrides.get("hairSplit", {}).get("backHairLeftRight", False)
    for raw_name in raw_names:
        canonical = loader_canonical_name(raw_name)
        if split_hair and base_name(canonical) == "back hair":
            loader_names.extend([f"{canonical}_1", f"{canonical}_2"])
        else:
            loader_names.append(canonical)
    def unique(values: list[str]) -> list[str]:
        seen: set[str] = set()
        result: list[str] = []
        for value in values:
            key = normalized(value)
            if key not in seen:
                seen.add(key)
                result.append(value)
        return result
    return unique(raw_names), unique(loader_names)


def validate_references(overrides: dict[str, Any], layer_names: list[str]) -> None:
    """Reject real dangling references; cleanup is limited to omitted layers."""
    raw_names, loader_names = reference_layer_sets(overrides, layer_names)
    hidden = {normalized(name) for name in overrides.get("hiddenLayers", [])}
    active = [
        name for name in loader_names
        if normalized(name) not in hidden and base_name(name) not in hidden
    ]
    active_before_overlays = list(active)
    for index, item in enumerate(overrides.get("maskedLayerOverlays", [])):
        check_resolved_reference(item["source"], active, f"rig-overrides.maskedLayerOverlays[{index}].source")
        check_resolved_reference(item["inFrontOf"], active, f"rig-overrides.maskedLayerOverlays[{index}].inFrontOf")
        if "textureSource" in item:
            check_resolved_reference(item["textureSource"], active, f"rig-overrides.maskedLayerOverlays[{index}].textureSource", exact=True)
        overlay_name = item["name"]
        if item.get("replaceExisting"):
            active = [name for name in active if normalized(name) != normalized(overlay_name)]
        if any(normalized(name) == normalized(overlay_name) for name in active):
            fail(f"rig-overrides.maskedLayerOverlays[{index}].name collides with an output layer")
        active.append(overlay_name)
    if "layerOrder" in overrides:
        for index, name in enumerate(overrides["layerOrder"]):
            check_resolved_reference(name, active, f"rig-overrides.layerOrder[{index}]")
    if "layerOrderConstraints" in overrides:
        for index, item in enumerate(overrides["layerOrderConstraints"]):
            check_resolved_reference(item["behind"], active, f"rig-overrides.layerOrderConstraints[{index}].behind")
            check_resolved_reference(item["inFrontOf"], active, f"rig-overrides.layerOrderConstraints[{index}].inFrontOf")
    for key in ("depthOverrides", "groupOverrides", "hairAttachments", "headFollow", "cleanupThresholds", "physics"):
        if key == "physics":
            entries = overrides.get("physics", {}).get("layers", {})
            prefix = "rig-overrides.physics.layers"
        else:
            entries = overrides.get(key, {})
            prefix = f"rig-overrides.{key}"
        for name in entries:
            check_resolved_reference(name, active, f"{prefix}.{name}")
    for key in ("hiddenLayers", "excludeAfterMeshResolution"):
        for index, name in enumerate(overrides.get(key, [])):
            check_resolved_reference(name, raw_names + loader_names, f"rig-overrides.{key}[{index}]", exact=True)
    for key in ("deformationSources", "meshSources"):
        for name, target in overrides.get(key, {}).items():
            check_resolved_reference(name, active, f"rig-overrides.{key}.{name}")
            check_resolved_reference(target, active, f"rig-overrides.{key}.{name} target", exact=True)
    for index, item in enumerate(overrides.get("interpolatedPatchRepairs", [])):
        check_resolved_reference(item["layer"], active_before_overlays, f"rig-overrides.interpolatedPatchRepairs[{index}].layer")
    if "mouthExpressions" in overrides:
        for key, name in overrides["mouthExpressions"].items():
            check_resolved_reference(name, active, f"rig-overrides.mouthExpressions.{key}", exact=True)
    if "layerAliases" in overrides:
        for name, target in overrides["layerAliases"].items():
            check_resolved_reference(name, raw_names, f"rig-overrides.layerAliases.{name}", exact=True)
            check_resolved_reference(target, loader_names, f"rig-overrides.layerAliases.{name} target", exact=True)


def safe_relative_file(root: Path, name: str, label: str) -> Path:
    expect_string(name, label)
    if "\\" in name:
        fail(f"{label} must use POSIX relative separators")
    relative = Path(name)
    if relative.is_absolute() or not relative.parts or any(part in ("", ".", "..") for part in relative.parts):
        fail(f"{label} must be a relative path inside the round")
    path = root / relative
    try:
        resolved = path.resolve(strict=True)
    except OSError as error:
        fail(f"{label} is unavailable: {error}")
    if not resolved.is_relative_to(root.resolve()):
        fail(f"{label} escapes the model round")
    return resolved


def plain_regular_file(path: Path, label: str) -> None:
    try:
        metadata = path.lstat()
    except OSError as error:
        fail(f"{label} is unavailable: {error}")
    if stat.S_ISLNK(metadata.st_mode):
        fail(f"{label} must not be a symlink")
    if not stat.S_ISREG(metadata.st_mode):
        fail(f"{label} must be a regular file")
    if metadata.st_nlink != 1:
        fail(f"{label} must not be a hard link")


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            while True:
                chunk = stream.read(1024 * 1024)
                if not chunk:
                    break
                digest.update(chunk)
    except OSError as error:
        fail(f"cannot hash {path.name}: {error}")
    return digest.hexdigest()


def inventory_files(root: Path) -> dict[str, str]:
    hashes: dict[str, str] = {}
    try:
        entries = sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix())
    except OSError as error:
        fail(f"cannot inventory model round: {error}")
    for path in entries:
        if path.is_symlink():
            fail(f"model round contains a symlink: {path.relative_to(root).as_posix()}")
        if path.is_file():
            plain_regular_file(path, f"model file {path.relative_to(root).as_posix()}")
            relative = path.relative_to(root).as_posix()
            hashes[relative] = hash_file(path)
    return hashes


def image_size(path: Path, label: str) -> tuple[int, int, Any, bool]:
    if Image is None:
        fail("Pillow is required; run with the creator Python environment")
    try:
        opened = Image.open(path)
        image = opened.convert("RGBA")
        opened.close()
    except Exception as error:
        fail(f"cannot decode {label}: {error}")
    alpha_empty = image.getchannel("A").getbbox() is None
    return image.width, image.height, image, alpha_empty


def scaled_extent(start: int, end: int, axis: str, path: str) -> tuple[int, int]:
    """Scale bounds for an all-transparent plate retained for references.

    Visible layers use their actual alpha coverage after filtering the complete
    source canvas on the shared output grid; these rounded bounds are only the
    geometry-preserving fallback for a plate with no coverage to measure.
    """
    left = int(round(start * SCALE))
    right = int(round(end * SCALE))
    if end > start and right <= left:
        right = left + 1
    if left < 0 or right > OUTPUT_SIZE or right <= left:
        fail(f"{path}.{axis} placement does not fit the 1024 canvas")
    return left, right


def transform_layers(model: Path, stage: Path, metadata: dict[str, Any]) -> tuple[dict[str, Any], list[str], list[str], list[str]]:
    unknown_meta = sorted(set(metadata) - LAYER_META_FIELDS)
    if unknown_meta:
        fail(f"layers.json.{unknown_meta[0]} is an unsupported field; refusing unknown dimensional schema")
    width = expect_integer(metadata.get("width"), "layers.json.width")
    height = expect_integer(metadata.get("height"), "layers.json.height")
    if (width, height) != (INPUT_SIZE, INPUT_SIZE):
        fail(f"layers.json canvas must be exactly 1280x1280, got {width}x{height}")
    rows = expect_list(metadata.get("layers"), "layers.json.layers", 0, 4096)
    output_rows: list[dict[str, Any]] = []
    omitted: list[str] = []
    empty_source: list[str] = []
    seen_names: set[str] = set()
    seen_files: set[str] = set()
    output_names: list[str] = []
    try:
        source_canvas = Image.new("RGBa", (INPUT_SIZE, INPUT_SIZE), (0, 0, 0, 0))
        resampling = Image.Resampling.LANCZOS
    except Exception as error:
        fail(f"cannot initialize the global layer sampling canvas: {error}")
    for index, raw_row in enumerate(rows):
        path = f"layers.json.layers[{index}]"
        row = exact_object(raw_row, path, LAYER_ROW_FIELDS)
        name = expect_string(row.get("name"), f"{path}.name")
        normalized_name = normalized(name)
        if normalized_name in seen_names:
            fail(f"{path}.name duplicates a normalized layer name")
        seen_names.add(normalized_name)
        filename = expect_string(row.get("filename"), f"{path}.filename")
        if Path(filename).as_posix() in RESERVED_OUTPUT_FILES:
            fail(f"{path}.filename collides with an adapter output file")
        if filename in seen_files:
            fail(f"{path}.filename is shared by multiple layers; independent rasters are required")
        seen_files.add(filename)
        source_file = safe_relative_file(model, filename, f"{path}.filename")
        plain_regular_file(source_file, f"layer raster {filename}")
        left = expect_integer(row.get("left"), f"{path}.left")
        top = expect_integer(row.get("top"), f"{path}.top")
        right = expect_integer(row.get("right"), f"{path}.right")
        bottom = expect_integer(row.get("bottom"), f"{path}.bottom")
        if min(left, top, right, bottom) < 0 or max(left, top, right, bottom) > INPUT_SIZE:
            fail(f"{path} placement must fit the 1280 canvas")
        if right < left or bottom < top:
            fail(f"{path} has inverted placement bounds")
        image_width, image_height, image, alpha_empty = image_size(source_file, f"layer {name}")
        expected_width = right - left
        expected_height = bottom - top
        try:
            if expected_width == 0 or expected_height == 0:
                if not alpha_empty:
                    fail(f"{path} is an empty placement but its raster contains visible pixels")
                omitted.append(name)
                continue
            if (image_width, image_height) != (expected_width, expected_height):
                fail(
                    f"{path}.filename raster is {image_width}x{image_height}, "
                    f"expected {expected_width}x{expected_height}"
                )
            source_canvas.paste((0, 0, 0, 0), (0, 0, INPUT_SIZE, INPUT_SIZE))
            premultiplied = None
            filtered = None
            output_canvas = None
            cropped = None
            alpha = None
            try:
                premultiplied = image.convert("RGBa")
                source_canvas.paste(premultiplied, (left, top))
                filtered = source_canvas.resize((OUTPUT_SIZE, OUTPUT_SIZE), resampling)
                output_canvas = filtered.convert("RGBA")
                if alpha_empty:
                    output_left, output_right = scaled_extent(left, right, "x", path)
                    output_top, output_bottom = scaled_extent(top, bottom, "y", path)
                else:
                    alpha = output_canvas.getchannel("A")
                    filtered_bounds = alpha.getbbox()
                    if filtered_bounds is None:
                        fail(f"{path}.filename lost all alpha while filtering on the 1024 canvas")
                    output_left, output_top, output_right, output_bottom = filtered_bounds
                output_relative = Path(filename)
                output_file = stage / output_relative
                output_file.parent.mkdir(parents=True, exist_ok=True)
                cropped = output_canvas.crop((output_left, output_top, output_right, output_bottom))
                cropped.save(output_file, format="PNG", compress_level=9, optimize=False)
            except AdapterError:
                raise
            except Exception as error:
                fail(f"cannot write converted raster {filename}: {error}")
            finally:
                for temporary in (alpha, cropped, output_canvas, filtered, premultiplied):
                    if temporary is not None:
                        temporary.close()
            output_row: dict[str, Any] = dict(row)
            output_row.update({
                "left": output_left,
                "top": output_top,
                "right": output_right,
                "bottom": output_bottom,
            })
            output_rows.append(output_row)
            output_names.append(name)
            if alpha_empty:
                empty_source.append(name)
        finally:
            image.close()
    source_canvas.close()
    omitted_normalized = {normalized(name) for name in omitted}
    for row in output_rows:
        if "clipTo" in row:
            target = expect_string(row["clipTo"], "layers.json clipTo")
            if normalized(target) in omitted_normalized:
                fail(f"layers.json layer {row['name']} clips to an omitted empty layer {target!r}")
    output_metadata: dict[str, Any] = {}
    for key, value in metadata.items():
        if key in ("width", "height"):
            continue
        if key == "layers":
            continue
        output_metadata[key] = value
    output_metadata["width"] = OUTPUT_SIZE
    output_metadata["height"] = OUTPUT_SIZE
    output_metadata["layers"] = output_rows
    return output_metadata, omitted, empty_source, output_names


def resolve_under(root: Path, reference: str, label: str, *, base: Path | None = None) -> Path:
    expect_string(reference, label)
    candidate = ((base or root) / reference).resolve(strict=True)
    if candidate != root and root not in candidate.parents:
        fail(f"{label} escapes the upstream source root")
    plain_regular_file(candidate, label)
    return candidate


def selected_pose(model: Path, upstream_root: Path) -> tuple[Path, Path, dict[str, Any]]:
    model_file = model / "model.json"
    plain_regular_file(model_file, "selected model.json")
    config = exact_object(
        load_json(model_file, "selected model.json"),
        "selected model.json",
        {
            "schemaVersion", "id", "label", "psd", "overrides", "pose", "bodySource",
            "excludeAfterMeshResolution", "noExternalBaseModel",
        },
    )
    if expect_integer(config.get("schemaVersion"), "selected model.json.schemaVersion") != 1:
        fail("selected model.json.schemaVersion must be 1")
    pose_file = resolve_under(
        upstream_root,
        expect_string(config.get("pose"), "selected model.json.pose"),
        "selected model.json.pose",
        base=model,
    )
    pose = expect_dict(load_json(pose_file, "selected pose.json"), "selected pose.json")
    return model_file, pose_file, pose


def source_motion_from_model(
    reference: str,
    selected_model: Path,
    upstream_root: Path,
) -> tuple[Path, Path, dict[str, Any], str]:
    if reference == "selected":
        source_dir = selected_model
    else:
        model_file = resolve_under(upstream_root, reference, "motion profile model")
        if model_file.name != "model.json":
            fail("motion profile model must reference model.json or use 'selected'")
        source_dir = model_file.parent
    model_file, pose_file, pose = selected_pose(source_dir, upstream_root)
    pose_id = expect_string(pose.get("id"), f"{pose_file.name}.id")
    motion = pose.get("motion")
    if motion is None:
        fail(f"motion profile model has no authored pose motion: {model_file}")
    return model_file, pose_file, expect_dict(motion, f"{pose_file.name}.motion"), pose_id


def convert_source_track(
    value: Any,
    path: str,
    duration: int,
    bounds_value: tuple[float, float],
    *,
    value_scale: float = 1.0,
) -> dict[str, Any]:
    track = expect_dict(value, path)
    track_type = track.get("type")
    if track_type == "sine":
        fail(f"{path} uses sine sampling, which native v2 cannot represent exactly")
    if track_type == "constant":
        exact_object(track, path, {"type", "value"})
        source_keys = [{"atMs": 0, "value": track.get("value")}]
        interpolation = "linear"
    elif track_type == "keyframes":
        exact_object(track, path, {"type", "interpolation", "frames"})
        interpolation = track.get("interpolation")
        if interpolation not in ("linear", "smoothstep"):
            fail(f"{path}.interpolation must be linear or smoothstep")
        source_keys = expect_list(track.get("frames"), f"{path}.frames", 2, 32)
    else:
        fail(f"{path}.type must be constant or keyframes; sine is unsupported")
    keys: list[dict[str, Any]] = []
    previous = -1
    for index, raw_key in enumerate(source_keys):
        key = exact_object(raw_key, f"{path}.frames[{index}]", {"atMs", "value"})
        at_ms_value = expect_number(key.get("atMs"), f"{path}.frames[{index}].atMs")
        if int(at_ms_value) != at_ms_value:
            fail(f"{path}.frames[{index}].atMs must be an integer for native v2")
        at_ms = int(at_ms_value)
        source_value = expect_number(key.get("value"), f"{path}.frames[{index}].value")
        converted_value = float(source_value) * value_scale
        if at_ms < 0 or at_ms > duration or at_ms <= previous:
            fail(f"{path} key times must start at 0, increase strictly, and stay inside duration")
        if converted_value < bounds_value[0] or converted_value > bounds_value[1]:
            fail(f"{path}.frames[{index}].value is outside the native range after conversion")
        keys.append({"at_ms": at_ms, "value": compact_number(converted_value)})
        previous = at_ms
    if not keys or keys[0]["at_ms"] != 0:
        fail(f"{path} must start at 0ms")
    return {"interpolation": interpolation, "keys": keys}


def convert_source_motion(source: dict[str, Any], pose_id: str, transition_ms: int, path: str) -> dict[str, Any]:
    exact_object(
        source,
        path,
        {"loopDurationMs", "playback", "envelope", "transition", "parameters", "layers"},
    )
    duration_value = expect_number(source.get("loopDurationMs"), f"{path}.loopDurationMs")
    if int(duration_value) != duration_value or not 1 <= int(duration_value) <= 30_000:
        fail(f"{path}.loopDurationMs must be an integer from 1 through 30000")
    duration = int(duration_value)
    if source.get("envelope") is not None:
        fail(f"{path}.envelope cannot be represented exactly by native v2")
    if source.get("transition") not in (None, "continuous"):
        fail(f"{path}.transition must be continuous when present")
    playback = source.get("playback", "loop")
    if playback not in ("loop", "once"):
        fail(f"{path}.playback must be loop or once")
    if not 0 <= transition_ms <= min(duration, 2_000):
        fail(f"{path} mapped transition_ms exceeds its duration or 2000ms")

    parameters = bounded_map(source.get("parameters", {}), f"{path}.parameters")
    tracks: dict[str, Any] = {}
    for name, track in parameters.items():
        if name not in GLOBAL_PARAMETER_BOUNDS:
            fail(f"{path}.parameters references unsupported native parameter {name!r}")
        tracks[name] = convert_source_track(
            track, f"{path}.parameters.{name}", duration, GLOBAL_PARAMETER_BOUNDS[name]
        )

    source_layers = bounded_map(source.get("layers", {}), f"{path}.layers")
    layers: dict[str, Any] = {}
    total_tracks = len(tracks)
    total_keys = sum(len(track["keys"]) for track in tracks.values())
    for name, raw_layer in source_layers.items():
        layer_path = f"{path}.layers.{name}"
        layer = exact_object(
            raw_layer,
            layer_path,
            {"translateX", "translateY", "rotationDeg", "scale", "origin", "influence"},
        )
        if "origin" not in layer:
            fail(
                f"{layer_path}.origin is required: upstream omitted-origin uses the "
                "raw layer center and native v2 cannot represent that exactly"
            )
        origin = exact_object(layer["origin"], f"{layer_path}.origin", {"x", "y"})
        origin_out = {
            "x": keep_number(origin.get("x"), f"{layer_path}.origin.x"),
            "y": keep_number(origin.get("y"), f"{layer_path}.origin.y"),
        }
        if not all(0 <= float(value) <= 1 for value in origin_out.values()):
            fail(f"{layer_path}.origin is outside native normalized mesh bounds")
        influence_value = layer.get(
            "influence",
            {"axisX": 1, "axisY": 0, "start": -2, "end": -1},
        )
        influence = exact_object(
            influence_value, f"{layer_path}.influence", {"axisX", "axisY", "start", "end"}
        )
        influence_out = {
            key: keep_number(influence.get(key), f"{layer_path}.influence.{key}")
            for key in ("axisX", "axisY", "start", "end")
        }
        axis_length = math.hypot(
            float(influence_out["axisX"]), float(influence_out["axisY"])
        )
        if axis_length < 1e-6:
            fail(f"{layer_path}.influence axis must be non-zero")
        influence_out["axisX"] = compact_number(float(influence_out["axisX"]) / axis_length)
        influence_out["axisY"] = compact_number(float(influence_out["axisY"]) / axis_length)
        if not (-2 <= float(influence_out["start"]) < float(influence_out["end"]) <= 2):
            fail(f"{layer_path}.influence taper is outside native -2..2 bounds")
        converted_layer: dict[str, Any] = {"origin": origin_out, "influence": influence_out}
        layer_track_count = 0
        for track_name, track_bounds in LOCAL_TRACK_BOUNDS.items():
            if track_name not in layer:
                continue
            value_scale = SCALE if track_name in ("translateX", "translateY") else 1.0
            converted = convert_source_track(
                layer[track_name],
                f"{layer_path}.{track_name}",
                duration,
                track_bounds,
                value_scale=value_scale,
            )
            converted_layer[track_name] = converted
            layer_track_count += 1
            total_keys += len(converted["keys"])
        if layer_track_count == 0:
            fail(f"{layer_path} must contain at least one transform track")
        total_tracks += layer_track_count
        layers[name] = converted_layer
    if total_tracks > 40 or total_keys > 256:
        fail(f"{path} exceeds native v2 per-motion track or key quota")
    return {
        "duration_ms": duration,
        "pose": pose_id,
        "transition_ms": transition_ms,
        "playback": playback,
        "tracks": tracks,
        "layers": layers,
    }


def convert_motion_profile(
    profile_path: Path,
    selected_model: Path,
    upstream_root: Path,
) -> tuple[dict[str, Any], dict[str, Any]]:
    plain_regular_file(profile_path, "motion profile")
    profile = exact_object(
        load_json(profile_path, "motion profile"),
        "motion profile",
        {"version", "pose", "phases", "reactions"},
    )
    if expect_integer(profile.get("version"), "motion profile.version") != 1:
        fail("motion profile.version must be 1")
    pose_id = expect_string(profile.get("pose"), "motion profile.pose")
    if len(pose_id.encode("utf-8")) > 32 or any(
        not (char.isascii() and (char.isalnum() or char in "-_")) for char in pose_id
    ):
        fail("motion profile.pose must be a native artistic ID")
    _, selected_pose_file, selected_pose_value = selected_pose(selected_model, upstream_root)
    if expect_string(selected_pose_value.get("id"), "selected pose.json.id") != pose_id:
        fail("motion profile.pose must exactly match the selected source pose ID")

    source_records: list[dict[str, Any]] = []
    total_keys = 0
    output_maps: dict[str, dict[str, Any]] = {}
    for group_name, required_names in (("phases", PHASES), ("reactions", REACTIONS)):
        mappings = exact_object(profile.get(group_name), f"motion profile.{group_name}", set(required_names))
        converted_group: dict[str, Any] = {}
        for slot in required_names:
            mapping = exact_object(
                mappings.get(slot),
                f"motion profile.{group_name}.{slot}",
                {"model", "transition_ms"},
            )
            model_reference = expect_string(
                mapping.get("model"), f"motion profile.{group_name}.{slot}.model"
            )
            transition_ms = expect_integer(
                mapping.get("transition_ms"), f"motion profile.{group_name}.{slot}.transition_ms"
            )
            model_file, pose_file, source, source_pose_id = source_motion_from_model(
                model_reference, selected_model, upstream_root
            )
            if source_pose_id != pose_id:
                fail(
                    f"motion profile.{group_name}.{slot} source pose {source_pose_id!r} "
                    f"does not match {pose_id!r}; pose aliasing is forbidden"
                )
            converted = convert_source_motion(
                source, pose_id, transition_ms, f"motion profile.{group_name}.{slot}"
            )
            total_keys += sum(len(track["keys"]) for track in converted["tracks"].values())
            total_keys += sum(
                len(track["keys"])
                for layer in converted["layers"].values()
                for name, track in layer.items()
                if name in LOCAL_TRACK_BOUNDS
            )
            source_records.append({
                "slot": f"{group_name}.{slot}",
                "model": model_file.relative_to(upstream_root).as_posix(),
                "modelSha256": hash_file(model_file),
                "pose": pose_file.relative_to(upstream_root).as_posix(),
                "poseSha256": hash_file(pose_file),
                "sourceDurationMs": source["loopDurationMs"],
                "sourcePlayback": source.get("playback", "loop"),
                "nativeTransitionMs": transition_ms,
                "interpolations": sorted({
                    track["interpolation"]
                    for track in converted["tracks"].values()
                } | {
                    track["interpolation"]
                    for layer in converted["layers"].values()
                    for name, track in layer.items()
                    if name in LOCAL_TRACK_BOUNDS
                }),
                "localLayers": sorted(converted["layers"]),
            })
            converted_group[slot] = converted
        output_maps[group_name] = converted_group
    if total_keys > 1024:
        fail("converted motion exceeds native v2 total key quota")
    return {
        "version": 2,
        "phases": output_maps["phases"],
        "reactions": output_maps["reactions"],
    }, {
        "profile": str(profile_path),
        "profileSha256": hash_file(profile_path),
        "selectedPose": selected_pose_file.relative_to(upstream_root).as_posix(),
        "sources": source_records,
        "translationScale": SCALE,
        "resolvedDefaults": {
            "influence": {"axisX": 1, "axisY": 0, "start": -2, "end": -1},
        },
        "unsupportedSourceSemantics": ["sine", "envelope", "omitted-origin", "non-exact-layer-selector"],
    }


def resolve_model(argument: str, upstream_root: Path) -> Path:
    supplied = Path(argument).expanduser()
    candidates = [supplied]
    if not supplied.is_absolute():
        candidates.extend([
            upstream_root / supplied,
            upstream_root / "outputs" / "characters" / supplied,
        ])
    for candidate in candidates:
        try:
            if candidate.exists():
                resolved = candidate.resolve(strict=True)
                if resolved.is_dir():
                    if candidate.is_symlink():
                        fail("model round must not be a symlink")
                    return resolved
        except OSError:
            continue
    fail(f"model round is not an existing directory: {argument}")


def path_exists(path: Path) -> bool:
    return os.path.lexists(path)


def install_stage(stage: Path, output: Path, allowed_root: Path) -> None:
    if path_exists(output):
        fail(f"output already exists; refusing overwrite: {output.name}")
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.parent.resolve().is_relative_to(allowed_root):
        try:
            os.rename(stage, output)
            return
        except OSError as error:
            fail(f"cannot install converted output: {error}")
    try:
        shutil.copytree(stage, output, dirs_exist_ok=False)
    except Exception as error:
        if output.exists() and output.is_dir():
            shutil.rmtree(output, ignore_errors=True)
        fail(f"cannot install converted output: {error}")
    shutil.rmtree(stage, ignore_errors=True)


def output_hashes(root: Path, *, exclude: set[str]) -> dict[str, str]:
    hashes: dict[str, str] = {}
    for path in sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix()):
        if path.is_symlink():
            fail(f"converted output contains a symlink: {path.relative_to(root).as_posix()}")
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        if relative in exclude:
            continue
        plain_regular_file(path, f"converted output file {relative}")
        hashes[relative] = hash_file(path)
    return hashes


def run_exporter(upstream_root: Path, stage: Path) -> None:
    exporter = upstream_root / "scripts" / "characters" / "export-model-psd.mjs"
    if not exporter.is_file():
        fail(f"canonical PSD exporter is unavailable: {exporter.name}")
    allowed_root = upstream_root / "outputs" / "characters"
    if not allowed_root.is_dir():
        fail("upstream root must contain outputs/characters for the canonical exporter")
    command = ["node", "scripts/characters/export-model-psd.mjs", str(stage)]
    environment = os.environ.copy()
    environment["DAEMONLET_CREATOR_PYTHON"] = sys.executable
    try:
        completed = subprocess.run(
            command,
            cwd=upstream_root,
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )
    except OSError as error:
        fail(f"cannot execute canonical PSD exporter: {error}")
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "exporter failed").strip().splitlines()
        message = detail[-1] if detail else "exporter failed"
        fail(f"canonical PSD exporter failed: {message}")
    psd = stage / "model.psd"
    if not psd.is_file() or psd.stat().st_size == 0:
        fail("canonical PSD exporter did not produce model.psd")


def build_provenance(
    model: Path,
    metadata: dict[str, Any],
    input_hashes: dict[str, str],
    output_hashes_value: dict[str, str],
    omitted: list[str],
    empty_source: list[str],
    removed_references: list[str],
    exporter_hash: str,
    motion_conversion: dict[str, Any] | None,
) -> dict[str, Any]:
    referenced = {row["filename"] for row in metadata["layers"]}
    referenced.update({"layers.json", "rig-overrides.json", "model.json", "pose.json"})
    command = ADAPTER_COMMAND if motion_conversion is not None else [
        *ADAPTER_COMMAND[:-2], "--raster-only",
    ]
    return {
        "schemaVersion": 1,
        "adapter": "tools/adapt-rig-source.py",
        "model": model.name,
        "inputCanvas": [INPUT_SIZE, INPUT_SIZE],
        "outputCanvas": [OUTPUT_SIZE, OUTPUT_SIZE],
        "transform": {
            "scale": SCALE,
            "samplingGrid": "shared full-canvas 1280-to-1024",
            "resampling": "Pillow.LANCZOS",
            "alpha": "premultiplied RGBA",
            "rasterBounds": "nonzero filtered alpha coverage",
        },
        "inputHashes": input_hashes,
        "outputHashes": output_hashes_value,
        "outputHashesExclude": ["provenance.json"],
        "command": command,
        "exporter": {"command": ["node", "scripts/characters/export-model-psd.mjs", "<output>"], "sha256": exporter_hash},
        "scaledFields": SCALED_FIELDS,
        "unsupportedFields": [],
        "omittedEmptyLayers": omitted,
        "transparentSourceLayers": empty_source,
        "removedOverrideReferences": removed_references,
        "unreferencedInputFiles": sorted(set(input_hashes) - referenced),
        "independentRasterNames": [row["name"] for row in metadata["layers"]],
        "motion": motion_conversion if motion_conversion is not None else {
            "status": "raster-export-only",
            "completeMotionAdaptation": False,
            "reason": "explicit --raster-only request; authored pose motion was not converted",
        },
    }


def make_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Convert a canonical 1280-square independent rig round to native 1024 assets."
    )
    parser.add_argument("--model", required=True, metavar="ROUND", help="canonical source-model round directory")
    parser.add_argument("--output", required=True, metavar="NEWDIR", help="new converted output directory")
    parser.add_argument("--upstream-root", required=True, metavar="PATH", help="pinned upstream authoring root")
    parser.add_argument("--size1024", action="store_true", help="convert to the bounded native 1024-square canvas")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--motion-profile", metavar="JSON", help="explicit per-slot authored source-model mapping")
    mode.add_argument("--raster-only", action="store_true", help="export geometry/rasters only; explicitly incomplete for motion")
    return parser


def adapt(args: argparse.Namespace) -> dict[str, Any]:
    if not args.size1024:
        fail("--size1024 is required; the adapter has no unbounded output mode")
    if Image is None:
        fail("Pillow is required; run with the creator Python environment")
    upstream_root = Path(args.upstream_root).expanduser().resolve()
    if not upstream_root.is_dir():
        fail(f"upstream root is not an existing directory: {args.upstream_root}")
    model = resolve_model(args.model, upstream_root)
    output = Path(args.output).expanduser().resolve(strict=False)
    if path_exists(output):
        fail(f"output already exists; refusing overwrite: {output.name}")
    if output == model or model in output.parents:
        fail("output must not be inside the immutable model round")
    allowed_root = (upstream_root / "outputs" / "characters").resolve()
    if not allowed_root.is_dir():
        fail("upstream root must contain outputs/characters for the canonical exporter")
    model_hashes = inventory_files(model)
    layers_path = model / "layers.json"
    overrides_path = model / "rig-overrides.json"
    plain_regular_file(layers_path, "layers.json")
    plain_regular_file(overrides_path, "rig-overrides.json")
    metadata = load_json(layers_path, "layers.json")
    _, _, selected_pose_value = selected_pose(model, upstream_root)
    selected_has_motion = selected_pose_value.get("motion") is not None
    motion_payload: dict[str, Any] | None = None
    motion_conversion: dict[str, Any] | None = None
    if args.motion_profile is not None:
        profile_path = Path(args.motion_profile).expanduser().resolve(strict=True)
        motion_payload, motion_conversion = convert_motion_profile(
            profile_path, model, upstream_root
        )
    elif not args.raster_only:
        fail("choose --motion-profile or --raster-only")
    metadata = expect_dict(metadata, "layers.json")
    source_overrides = load_json(overrides_path, "rig-overrides.json")
    source_overrides = expect_dict(source_overrides, "rig-overrides")
    transformed_overrides = transform_overrides(source_overrides)
    exporter_hash = hash_file(upstream_root / "scripts" / "characters" / "export-model-psd.mjs") if (upstream_root / "scripts" / "characters" / "export-model-psd.mjs").is_file() else ""

    stage = Path(tempfile.mkdtemp(prefix=".adapt-rig-source-", dir=allowed_root))
    installed = False
    try:
        transformed_metadata, omitted_names, empty_source, output_names = transform_layers(model, stage, metadata)
        omitted_normalized = {
            key
            for name in omitted_names
            for key in (normalized(name), loader_canonical_name(name))
        }
        if motion_payload is not None:
            motion_layer_names = {
                name
                for group in ("phases", "reactions")
                for spec in motion_payload[group].values()
                for name in spec["layers"]
            }
            _, decoded_output_names = reference_layer_sets(transformed_overrides, output_names)
            missing_motion_layers = sorted(motion_layer_names - set(decoded_output_names))
            if missing_motion_layers:
                fail(
                    "authored motion layer selectors must exactly name one decoded "
                    "output layer; unresolved or selector-only names: "
                    + ", ".join(missing_motion_layers)
                )
            write_json(stage / "motion.json", motion_payload)
        removed_references = clean_omitted_references(transformed_overrides, omitted_normalized)
        validate_references(transformed_overrides, output_names)
        write_json(stage / "layers.json", transformed_metadata)
        write_json(stage / "rig-overrides.json", transformed_overrides)
        run_exporter(upstream_root, stage)
        hashes_before_provenance = output_hashes(stage, exclude={"provenance.json"})
        provenance = build_provenance(
            model,
            transformed_metadata,
            model_hashes,
            hashes_before_provenance,
            omitted_names,
            empty_source,
            removed_references,
            exporter_hash,
            motion_conversion,
        )
        write_json(stage / "provenance.json", provenance)
        install_stage(stage, output, allowed_root)
        installed = True
    finally:
        if not installed:
            shutil.rmtree(stage, ignore_errors=True)
    complete_motion = motion_payload is not None
    command = ADAPTER_COMMAND if complete_motion else [*ADAPTER_COMMAND[:-2], "--raster-only"]
    return {
        "status": "ok",
        "model": model.name,
        "output": output.name,
        "canvas": [OUTPUT_SIZE, OUTPUT_SIZE],
        "scale": SCALE,
        "layers": len(transformed_metadata["layers"]),
        "psd": "model.psd",
        "motion": "motion.json" if complete_motion else None,
        "completeMotionAdaptation": complete_motion,
        "sourceHadAuthoredMotion": selected_has_motion,
        "command": command,
        "exportCommand": ["node", "scripts/characters/export-model-psd.mjs", "<output>"],
        "scaledFields": SCALED_FIELDS,
        "unsupportedFields": [],
        "omittedEmptyLayers": omitted_names,
        "removedOverrideReferences": removed_references,
    }


def main(argv: list[str] | None = None) -> int:
    parser = make_parser()
    try:
        args = parser.parse_args(argv)
        result = adapt(args)
    except AdapterError as error:
        print(json.dumps({"status": "error", "error": str(error)}, ensure_ascii=False), file=sys.stderr)
        return 1
    except (OSError, ValueError) as error:
        print(json.dumps({"status": "error", "error": str(error)}, ensure_ascii=False), file=sys.stderr)
        return 1
    print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
