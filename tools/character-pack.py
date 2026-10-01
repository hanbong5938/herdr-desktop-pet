#!/usr/bin/env python3
"""Create, inventory, validate, assemble, preview, review, and package Herdr v4/v5 characters.

The creator tool deliberately leaves semantic and PSD validation to the native
validator while enforcing bounded, deterministic source handling locally.
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
import struct
import subprocess
import sys
import tempfile
import zlib
from typing import Any, Mapping, Sequence


MANIFEST_NAME = "manifest.json"
MAX_MANIFEST_BYTES = 64 * 1024
MAX_ENTRY_BYTES = 64 * 1024
MAX_MOTION_BYTES = 64 * 1024
MAX_OVERRIDES_BYTES = 256 * 1024
MAX_ORDINARY_BYTES = 4 * 1024 * 1024
MAX_PAYLOADS = 128
MAX_EXPANDED_BYTES = 64 * 1024 * 1024
MAX_ARCHIVE_BYTES = 64 * 1024 * 1024
MAX_PREVIEW_BYTES = 8 * 1024 * 1024
PHASES = ("idle", "running", "waiting", "unknown")
REACTIONS = ("head_tap", "body_tap", "pet", "completion_observed")
PAYLOAD_KINDS = {"png", "psd", "entry", "overrides", "motion", "license", "attribution", "source"}
MOTION_PHASES = ("idle", "running", "waiting", "unknown")
MOTION_REACTIONS = ("head_tap", "body_tap", "pet", "completion_observed")
MOTION_PARAMETER_BOUNDS = {
    **{name: (-1.0, 1.0) for name in (
        "angleX", "angleY", "angleZ", "eyeX", "eyeY", "brow", "browAngL",
        "browAngR", "browAngSym", "mouthForm", "mouthCY", "body", "armY",
        "armPos", "bangL", "bangC", "bangR", "eyeCY", "eyeCAng", "mouthCAng",
    )},
    **{name: (0.0, 1.0) for name in (
        "eyeOpenL", "eyeOpenR", "mouthOpen", "eyeEase", "mouthEase",
    )},
    "irisScale": (0.5, 1.3), "physAmp": (0.0, 3.0), "soft": (0.0, 3.0),
    "fhAmp": (0.0, 3.0), "bust": (0.0, 4.0), "bustY": (-3.0, 3.0),
    "fhSoft": (0.0, 2.0), "eyeScaleL": (0.5, 1.5),
    "eyeScaleR": (0.5, 1.5), "mouthScale": (0.5, 1.5),
}
MOTION_LOCAL_BOUNDS = {
    "translateX": (-1024.0, 1024.0), "translateY": (-1024.0, 1024.0),
    "rotationDeg": (-360.0, 360.0), "scale": (0.0, 4.0),
}
PACK_ID_CHARS = frozenset("abcdefghijklmnopqrstuvwxyz0123456789_-")
RIG_POSE_ID_CHARS = frozenset("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-")

def nonnegative_int(text: str) -> int:
    try:
        value = int(text, 10)
    except ValueError as error:
        raise argparse.ArgumentTypeError("must be a decimal integer") from error
    if value < 0:
        raise argparse.ArgumentTypeError("must be non-negative")
    return value


class ToolError(Exception):
    """An expected creator-tool failure with a user-facing message."""


def fail(message: str) -> None:
    raise ToolError(message)


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ToolError(f"JSON contains duplicate key {key!r}")
        result[key] = value
    return result


def reject_nulls(value: Any, path: str = "$") -> None:
    if value is None:
        fail(f"optional JSON fields must be omitted, not null ({path})")
    if isinstance(value, list):
        for index, child in enumerate(value):
            reject_nulls(child, f"{path}[{index}]")
    elif isinstance(value, dict):
        for key, child in value.items():
            reject_nulls(child, f"{path}.{key}")


def read_bytes(path: Path, limit: int, label: str) -> bytes:
    """Read one no-follow regular file while enforcing a bounded size."""
    try:
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
    except OSError as error:
        fail(f"cannot open {label} {path}: {error}")
    try:
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode):
            fail(f"{label} must be a regular file: {path}")
        if before.st_nlink != 1:
            fail(f"{label} must not be a hard link: {path}")
        if before.st_mode & 0o111:
            fail(f"{label} must not be executable: {path}")
        if before.st_size > limit:
            fail(f"{label} exceeds {limit} bytes: {path}")
        chunks: list[bytes] = []
        remaining = limit + 1
        while remaining:
            chunk = os.read(fd, min(1024 * 1024, remaining))
            if not chunk:
                break
            chunks.append(chunk)
            remaining -= len(chunk)
        data = b"".join(chunks)
        after = os.fstat(fd)
        if (
            after.st_dev != before.st_dev
            or after.st_ino != before.st_ino
            or after.st_nlink != 1
            or after.st_size != len(data)
        ):
            fail(f"{label} changed while being read: {path}")
        if len(data) > limit:
            fail(f"{label} exceeds {limit} bytes: {path}")
        return data
    finally:
        os.close(fd)


def check_root(root: Path) -> Path:
    try:
        root_lstat = root.lstat()
    except OSError as error:
        fail(f"character source is unavailable: {root}: {error}")
    if stat.S_ISLNK(root_lstat.st_mode):
        fail(f"character source must not be a symlink: {root}")
    if not stat.S_ISDIR(root_lstat.st_mode):
        fail(f"character source must be a directory: {root}")
    if root_lstat.st_nlink < 1:
        fail(f"character source has invalid link metadata: {root}")
    return root.absolute()


def validate_filename(name: str, *, allow_manifest: bool = False) -> None:
    encoded = name.encode("utf-8", "surrogateescape")
    if not 1 <= len(encoded) <= 255:
        fail(f"payload filename must be 1..255 bytes: {name!r}")
    if any(byte < 0x20 or byte == 0x7F for byte in encoded) or not name.isascii():
        fail(f"payload filename must be printable ASCII: {name!r}")
    if "/" in name or "\\" in name or name in (".", "..") or name.startswith(".") or name.endswith("."):
        fail(f"payload filename must be a root-only component: {name!r}")
    if name == MANIFEST_NAME and not allow_manifest:
        fail("manifest.json is not a payload filename")


def case_key(name: str) -> bytes:
    return name.encode("ascii").lower()


def bounded_limit(kind: str) -> int:
    if kind == "entry":
        return MAX_ENTRY_BYTES
    if kind == "motion":
        return MAX_MOTION_BYTES
    if kind == "overrides":
        return MAX_OVERRIDES_BYTES
    return MAX_ORDINARY_BYTES


def list_flat_files(root: Path, *, include_manifest: bool = False) -> dict[str, bytes]:
    root = check_root(root)
    files: dict[str, bytes] = {}
    folded: set[bytes] = set() if include_manifest else {case_key(MANIFEST_NAME)}
    try:
        entries = []
        with os.scandir(root) as iterator:
            for entry in iterator:
                if len(entries) >= MAX_PAYLOADS + 1:
                    fail(f"character source contains more than {MAX_PAYLOADS} payloads plus manifest")
                entries.append(entry)
        entries.sort(key=lambda entry: entry.name)
    except OSError as error:
        fail(f"cannot inventory character source {root}: {error}")
    total = 0
    for entry in entries:
        name = entry.name
        if name == MANIFEST_NAME:
            if not include_manifest:
                continue
            validate_filename(name, allow_manifest=True)
        else:
            validate_filename(name)
        if len(files) >= MAX_PAYLOADS + (1 if include_manifest else 0):
            fail(f"character source contains more than {MAX_PAYLOADS} payloads plus manifest")
        try:
            metadata = os.lstat(entry.path)
        except OSError as error:
            fail(f"cannot inspect character source entry {entry.path}: {error}")
        if stat.S_ISLNK(metadata.st_mode):
            fail(f"character sources cannot contain symlinks: {name}")
        if not stat.S_ISREG(metadata.st_mode):
            fail(f"character sources must contain flat regular files: {name}")
        if metadata.st_nlink != 1:
            fail(f"character sources cannot contain hard links: {name}")
        if case_key(name) in folded:
            fail(f"character source contains a case-colliding filename: {name}")
        folded.add(case_key(name))
        # Use the ordinary limit until the manifest classifies the file.  JSON
        # payloads are checked again with their narrower kind limit below.
        limit = min(MAX_ORDINARY_BYTES, MAX_EXPANDED_BYTES - total)
        data = read_bytes(Path(entry.path), limit, "character payload")
        total += len(data)
        files[name] = data
    return files


def load_manifest(root: Path) -> tuple[bytes, dict[str, Any]]:
    manifest_path = root / MANIFEST_NAME
    raw = read_bytes(manifest_path, MAX_MANIFEST_BYTES, "manifest")
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=reject_duplicate_keys)
    except (UnicodeDecodeError, json.JSONDecodeError, ToolError) as error:
        fail(f"manifest.json is invalid JSON: {error}")
    if not isinstance(value, dict):
        fail("manifest.json must contain an object")
    reject_nulls(value)
    if value.get("format") != "herdr.character" or value.get("version") not in (4, 5):
        fail("creator tool only manages v4/v5 herdr.character manifests")
    if value.get("render_mode") not in ("png", "rig"):
        fail("manifest render_mode must be png or rig")
    payloads = value.get("payloads")
    if not isinstance(payloads, list):
        fail("manifest payloads must be an array")
    for index, payload in enumerate(payloads):
        if not isinstance(payload, dict):
            fail(f"manifest payload {index} must be an object")
        required = {"path", "size", "sha256", "kind"}
        if set(payload) != required:
            fail(f"manifest payload {index} must contain exactly path, size, sha256, and kind")
        if not isinstance(payload["path"], str) or not isinstance(payload["size"], int) or isinstance(payload["size"], bool) or not isinstance(payload["sha256"], str) or not isinstance(payload["kind"], str):
            fail(f"manifest payload {index} has invalid field types")
        validate_filename(payload["path"])
        if payload["kind"] not in PAYLOAD_KINDS:
            fail(f"manifest payload {index} has unsupported kind {payload['kind']!r}")
    return raw, value


def infer_kind(name: str, manifest: Mapping[str, Any]) -> str:
    for item in manifest.get("payloads", []):
        if isinstance(item, dict) and item.get("path") == name and isinstance(item.get("kind"), str):
            return item["kind"]
    entry = manifest.get("entry")
    if name == entry or name in {"entry.json", "rig-entry.json", "png-entry.json"}:
        return "entry"
    if name == "motion.json" or name.endswith("-motion.json"):
        return "motion"
    if name == "rig-overrides.json" or name.endswith("-overrides.json"):
        return "overrides"
    if name.lower().endswith(".png"):
        return "png"
    if name.lower().endswith(".psd"):
        return "psd"
    upper = name.upper()
    if "LICENSE" in upper:
        return "license"
    if "ATTRIBUTION" in upper:
        return "attribution"
    if "SOURCE" in upper or name.endswith(".txt"):
        return "source"
    fail(f"cannot infer payload kind for {name!r}; add it to manifest payloads before regenerating")


def check_payload_magic(name: str, kind: str, data: bytes) -> None:
    if len(data) > bounded_limit(kind):
        fail(f"payload {name} exceeds its {kind} limit of {bounded_limit(kind)} bytes")
    if kind == "png" and not data.startswith(b"\x89PNG\r\n\x1a\n"):
        fail(f"payload {name} is not a PNG")
    if kind == "psd" and not data.startswith(b"8BPS"):
        fail(f"payload {name} is not a PSD")
    if kind in {"entry", "overrides", "motion"}:
        try:
            parsed = json.loads(data.decode("utf-8"), object_pairs_hook=reject_duplicate_keys)
        except (UnicodeDecodeError, json.JSONDecodeError, ToolError) as error:
            fail(f"payload {name} is invalid JSON: {error}")
        reject_nulls(parsed)
    if kind in {"license", "attribution", "source"}:
        try:
            data.decode("utf-8")
        except UnicodeDecodeError as error:
            fail(f"text payload {name} is not UTF-8: {error}")


def inventory_records(root: Path) -> tuple[bytes, dict[str, Any], dict[str, bytes], list[dict[str, Any]]]:
    root = check_root(root)
    manifest_raw, manifest = load_manifest(root)
    files = list_flat_files(root)
    existing = {item["path"]: item for item in manifest["payloads"]}
    if len(existing) != len(manifest["payloads"]):
        fail("manifest payload inventory contains duplicate paths")
    folded: set[bytes] = set()
    records: list[dict[str, Any]] = []
    for name in sorted(files):
        if case_key(name) in folded:
            fail(f"manifest inventory contains a case collision: {name}")
        folded.add(case_key(name))
        kind = infer_kind(name, manifest)
        data = files[name]
        check_payload_magic(name, kind, data)
        records.append({"path": name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest(), "kind": kind})
    manifest_paths = {item["path"] for item in manifest["payloads"]}
    actual_paths = set(files)
    missing = sorted(manifest_paths - actual_paths)
    extra = sorted(actual_paths - manifest_paths)
    if missing:
        # Regeneration is allowed to add newly authored files, but a missing
        # declared payload is almost always accidental and must be explicit.
        fail(f"manifest declares missing payload(s): {', '.join(missing)}")
    if extra:
        # New files are inventory candidates, not silently ignored files.
        pass
    order = {name: index for index, name in enumerate(item["path"] for item in manifest["payloads"])}
    records.sort(key=lambda item: (order.get(item["path"], len(order)), item["path"]))
    return manifest_raw, manifest, files, records


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, separators=(",", ": ")) + "\n").encode("utf-8")


def write_atomic(path: Path, data: bytes) -> None:
    if path.exists() or path.is_symlink():
        # The manifest is intentionally regenerated in place, but do not
        # follow a hostile replacement.
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
            fail(f"manifest destination is not a private regular file: {path}")
    directory = path.parent
    directory_metadata = directory.lstat()
    if not stat.S_ISDIR(directory_metadata.st_mode) or stat.S_ISLNK(directory_metadata.st_mode):
        fail(f"manifest parent is not a real directory: {directory}")
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=str(directory))
    temporary_path = Path(temporary)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_path, path)
    except Exception:
        try:
            temporary_path.unlink()
        except OSError:
            pass
        raise


def regenerate_inventory(root: Path, *, check_only: bool = False) -> dict[str, Any]:
    manifest_raw, manifest, files, records = inventory_records(root)
    del manifest_raw
    updated = dict(manifest)
    updated["payloads"] = records
    encoded = canonical_json(updated)
    changed = encoded != (root / MANIFEST_NAME).read_bytes()
    if changed and not check_only:
        write_atomic(root / MANIFEST_NAME, encoded)
    return {
        "ok": True,
        "command": "inventory",
        "path": str(root.absolute()),
        "changed": changed,
        "payload_count": len(records),
        "payload_bytes": sum(item["size"] for item in records),
        "manifest_sha256": hashlib.sha256(encoded if changed and not check_only else (root / MANIFEST_NAME).read_bytes()).hexdigest(),
    }

def read_root_payload(root: Path, name: str, kind: str, label: str) -> tuple[Path, bytes]:
    """Read an explicitly named root-local payload with the normal file gates."""
    validate_filename(name)
    path = root / name
    try:
        metadata = path.lstat()
    except OSError as error:
        fail(f"{label} is unavailable: {path}: {error}")
    if stat.S_ISLNK(metadata.st_mode):
        fail(f"{label} must not be a symlink: {path}")
    if not stat.S_ISREG(metadata.st_mode):
        fail(f"{label} must be a regular file: {path}")
    if metadata.st_nlink != 1:
        fail(f"{label} must not be a hard link: {path}")
    data = read_bytes(path, bounded_limit(kind), label)
    check_payload_magic(name, kind, data)
    return path, data


def decode_object(data: bytes, label: str) -> dict[str, Any]:
    try:
        value = json.loads(data.decode("utf-8"), object_pairs_hook=reject_duplicate_keys)
    except (UnicodeDecodeError, json.JSONDecodeError, ToolError) as error:
        fail(f"{label} is invalid JSON: {error}")
    if not isinstance(value, dict):
        fail(f"{label} must contain an object")
    reject_nulls(value)
    return value


def validate_artistic_id(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        fail(f"{label} must be a non-empty artistic ID")
    encoded = value.encode("utf-8", "surrogateescape")
    if len(encoded) > 128 or any(ord(char) < 0x20 or ord(char) == 0x7F for char in value):
        fail(f"{label} must be at most 128 UTF-8 bytes without control characters")
    return value


def validate_pose_id(value: Any, label: str) -> str:
    """Mirror native validate_rig_pose_id: 1..32 ASCII [A-Za-z0-9_-]."""
    if not isinstance(value, str) or not 1 <= len(value) <= 32 or any(char not in RIG_POSE_ID_CHARS for char in value):
        fail(f"{label} must contain 1..32 ASCII letters, digits, '_' or '-'")
    return value


def motion_number(value: Any, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        fail(f"{label} must be a finite number")
    return float(value)


def motion_integer(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        fail(f"{label} must be an integer")
    return value


def validate_v2_track(
    value: Any,
    label: str,
    duration: int,
    bounds: tuple[float, float],
) -> int:
    if not isinstance(value, dict) or set(value) != {"interpolation", "keys"}:
        fail(f"{label} must contain exactly interpolation and keys")
    if value["interpolation"] not in ("linear", "smoothstep"):
        fail(f"{label}.interpolation must be linear or smoothstep")
    keys = value["keys"]
    if not isinstance(keys, list) or not 1 <= len(keys) <= 32:
        fail(f"{label}.keys must contain 1..32 keys")
    previous = -1
    for index, key in enumerate(keys):
        key_label = f"{label}.keys[{index}]"
        if not isinstance(key, dict) or set(key) != {"at_ms", "value"}:
            fail(f"{key_label} must contain exactly at_ms and value")
        at_ms = motion_integer(key["at_ms"], f"{key_label}.at_ms")
        amount = motion_number(key["value"], f"{key_label}.value")
        if at_ms < 0 or at_ms > duration or at_ms <= previous:
            fail(f"{label} key times must start at 0, increase strictly, and stay inside duration")
        if not bounds[0] <= amount <= bounds[1]:
            fail(f"{key_label}.value is outside the native range")
        previous = at_ms
    if keys[0]["at_ms"] != 0:
        fail(f"{label} must start at 0ms")
    return len(keys)


def validate_v2_motion(
    data: bytes,
    label: str,
    allowed_poses: set[str],
    canvas_extent: int = 1024,
) -> None:
    motion = decode_object(data, label)
    if motion.get("version") != 2 or set(motion) != {"version", "phases", "reactions"}:
        if motion.get("version") == 1:
            return
        fail(f"{label} v2 must contain exactly version, phases, and reactions")
    total_keys = 0
    for group_name, required_names in (
        ("phases", MOTION_PHASES),
        ("reactions", MOTION_REACTIONS),
    ):
        group = motion[group_name]
        if not isinstance(group, dict) or set(group) != set(required_names):
            fail(f"{label}.{group_name} must contain the native four slots exactly")
        for slot in required_names:
            spec_label = f"{label}.{group_name}.{slot}"
            spec = group[slot]
            required = {
                "duration_ms", "pose", "transition_ms", "playback", "tracks", "layers",
            }
            if not isinstance(spec, dict) or set(spec) != required:
                fail(f"{spec_label} must contain the exact native v2 fields")
            duration = motion_integer(spec["duration_ms"], f"{spec_label}.duration_ms")
            transition = motion_integer(spec["transition_ms"], f"{spec_label}.transition_ms")
            if not 1 <= duration <= 30_000:
                fail(f"{spec_label}.duration_ms must be 1..30000")
            if not 0 <= transition <= min(duration, 2_000):
                fail(f"{spec_label}.transition_ms exceeds its duration or 2000ms")
            if not isinstance(spec["pose"], str) or spec["pose"] not in allowed_poses:
                fail(f"{spec_label}.pose references an undeclared artistic model")
            if spec["playback"] not in ("loop", "once"):
                fail(f"{spec_label}.playback must be loop or once")
            tracks = spec["tracks"]
            layers = spec["layers"]
            if not isinstance(tracks, dict) or not isinstance(layers, dict):
                fail(f"{spec_label}.tracks and layers must be objects")
            track_count = len(tracks)
            key_count = 0
            for name, track in tracks.items():
                bounds = MOTION_PARAMETER_BOUNDS.get(name)
                if bounds is None:
                    fail(f"{spec_label}.tracks references unknown parameter {name!r}")
                key_count += validate_v2_track(track, f"{spec_label}.tracks.{name}", duration, bounds)
            for layer_name, layer in layers.items():
                validate_artistic_id(layer_name, f"{spec_label}.layers layer name")
                if not isinstance(layer, dict):
                    fail(f"{spec_label}.layers.{layer_name} must be an object")
                local_names = set(layer) - {"origin", "influence"}
                if not local_names or not local_names <= set(MOTION_LOCAL_BOUNDS):
                    fail(f"{spec_label}.layers.{layer_name} has unsupported or missing transform tracks")
                if "origin" not in layer or "influence" not in layer:
                    fail(f"{spec_label}.layers.{layer_name} requires origin and influence")
                origin = layer["origin"]
                influence = layer["influence"]
                if not isinstance(origin, dict) or set(origin) != {"x", "y"}:
                    fail(f"{spec_label}.layers.{layer_name}.origin is invalid")
                x = motion_number(origin["x"], f"{spec_label}.layers.{layer_name}.origin.x")
                y = motion_number(origin["y"], f"{spec_label}.layers.{layer_name}.origin.y")
                if not (0 <= x <= 1 and 0 <= y <= 1):
                    fail(f"{spec_label}.layers.{layer_name}.origin must be normalized")
                if not isinstance(influence, dict) or set(influence) != {"axisX", "axisY", "start", "end"}:
                    fail(f"{spec_label}.layers.{layer_name}.influence is invalid")
                axis_x = motion_number(influence["axisX"], f"{spec_label}.layers.{layer_name}.influence.axisX")
                axis_y = motion_number(influence["axisY"], f"{spec_label}.layers.{layer_name}.influence.axisY")
                start = motion_number(influence["start"], f"{spec_label}.layers.{layer_name}.influence.start")
                end = motion_number(influence["end"], f"{spec_label}.layers.{layer_name}.influence.end")
                if (
                    math.hypot(axis_x, axis_y) < 1e-6
                    or abs(axis_x) > 1
                    or abs(axis_y) > 1
                    or not (-2 <= start < end <= 2)
                ):
                    fail(f"{spec_label}.layers.{layer_name}.influence axis/taper is invalid")
                for name in local_names:
                    track_bounds = (
                        (-float(canvas_extent), float(canvas_extent))
                        if name in ("translateX", "translateY")
                        else MOTION_LOCAL_BOUNDS[name]
                    )
                    key_count += validate_v2_track(
                        layer[name],
                        f"{spec_label}.layers.{layer_name}.{name}",
                        duration,
                        track_bounds,
                    )
                track_count += len(local_names)
            if track_count > 40 or key_count > 256:
                fail(f"{spec_label} exceeds the native track or key quota")
            total_keys += key_count
    if total_keys > 1024:
        fail(f"{label} exceeds the native total key quota")
def load_rig_entry(root: Path, manifest: Mapping[str, Any]) -> tuple[str, bytes, dict[str, Any]]:
    if manifest.get("render_mode") != "rig":
        fail("rig authoring requires a manifest with render_mode rig")
    entry_name = manifest.get("entry")
    if not isinstance(entry_name, str):
        fail("rig manifest entry must name an existing entry payload")
    _, entry_bytes = read_root_payload(root, entry_name, "entry", "rig entry")
    return entry_name, entry_bytes, decode_object(entry_bytes, "rig entry")


def new_private_directory(path: Path, label: str) -> Path:
    path = path.absolute()
    if path.exists() or path.is_symlink():
        fail(f"{label} already exists: {path}")
    parent = path.parent
    if parent.is_symlink() or not parent.is_dir():
        fail(f"{label} parent must be a real directory: {parent}")
    try:
        path.mkdir(mode=0o700)
    except OSError as error:
        fail(f"cannot create private {label} {path}: {error}")
    try:
        metadata = path.lstat()
    except OSError as error:
        fail(f"cannot inspect private {label} {path}: {error}")
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        fail(f"private {label} is not a real directory: {path}")
    return path


def remove_private_directory(path: Path) -> None:
    try:
        metadata = path.lstat()
    except OSError:
        return
    if stat.S_ISDIR(metadata.st_mode) and not stat.S_ISLNK(metadata.st_mode):
        shutil.rmtree(path)


def validate_pack_identity(manifest: Mapping[str, Any]) -> None:
    pack_id = manifest.get("id")
    if (
        not isinstance(pack_id, str)
        or not pack_id
        or len(pack_id) > 64
        or any(char not in PACK_ID_CHARS for char in pack_id)
        or pack_id[0] in "_-"
    ):
        fail("manifest id must contain 1..64 lowercase ASCII letters, digits, '_' or '-' and start with a lowercase letter or digit")
    if pack_id == "default":
        fail("creator templates must not reserve builtin ID default")
    name = manifest.get("name")
    if not isinstance(name, str):
        fail("manifest name must be a string")
    try:
        encoded_name = name.encode("utf-8")
    except UnicodeEncodeError:
        fail("manifest name must be valid Unicode text")
    if not 1 <= len(encoded_name) <= 128:
        fail("manifest name must be 1..128 UTF-8 bytes")
    if any(ord(char) < 0x20 or 0x7F <= ord(char) <= 0x9F for char in name):
        fail("manifest name must not contain control characters")


def find_template_root(explicit: Path | None, mode: str) -> Path:
    candidates: list[Path] = []
    if explicit is not None:
        base = explicit.absolute()
        candidates.extend(
            [
                base / ("rig-default" if mode == "rig" else "png-example"),
                base / "default",
                base,
            ]
        )
    else:
        script = Path(__file__).resolve()
        # The bundled default is a rig; unmatched modes need external templates.
        if len(script.parents) >= 3 and script.parents[1].name == "Resources":
            candidates.append(script.parents[1] / "default")
        elif len(script.parents) >= 2 and mode == "rig":
            candidates.append(script.parents[1] / "assets" / "rubelia-default")
    for candidate in candidates:
        if candidate.is_dir() and not candidate.is_symlink() and (candidate / MANIFEST_NAME).is_file():
            root = check_root(candidate)
            _, manifest = load_manifest(root)
            if manifest["render_mode"] == mode:
                return root
    searched = ", ".join(str(path) for path in candidates)
    guidance = f" Supply a matching {mode} template with --templates-root PATH."
    fail(f"cannot locate {mode} template; searched bounded template paths: {searched}.{guidance}")


def copy_template(source: Path, destination: Path, pack_id: str | None, name: str | None) -> dict[str, Any]:
    if destination.exists() or destination.is_symlink():
        fail(f"template output already exists: {destination}")
    destination_parent = destination.parent
    if not destination_parent.exists() or destination_parent.is_symlink() or not destination_parent.is_dir():
        fail(f"template output parent must be a real directory: {destination_parent}")
    source_manifest_raw, source_manifest = load_manifest(source)
    del source_manifest_raw
    files = list_flat_files(source, include_manifest=True)
    destination.mkdir(mode=0o700)
    try:
        for filename, data in files.items():
            output = destination / filename
            with output.open("xb") as stream:
                os.fchmod(stream.fileno(), 0o600)
                stream.write(data)
        manifest = dict(source_manifest)
        if pack_id is not None:
            manifest["id"] = pack_id
        if name is not None:
            manifest["name"] = name
        validate_pack_identity(manifest)
        write_atomic(destination / MANIFEST_NAME, canonical_json(manifest))
        result = regenerate_inventory(destination)
        result.update({"command": "template", "source": str(source), "path": str(destination.absolute())})
        return result
    except Exception:
        shutil.rmtree(destination, ignore_errors=True)
        raise


def native_candidates(explicit: Path | None) -> list[Path]:
    candidates: list[Path] = []
    if explicit is not None:
        candidates.append(explicit)
    configured = os.environ.get("HERDR_DESKTOP_PET_BIN")
    if configured:
        candidates.append(Path(configured).expanduser())
    script = Path(__file__).resolve()
    if len(script.parents) >= 3 and script.parents[1].name == "Resources":
        candidates.append(script.parents[2] / "MacOS" / "herdr-desktop-pet")
    if len(script.parents) >= 2:
        repository = script.parents[1]
        candidates.extend(
            [
                repository / "target" / "debug" / "herdr-desktop-pet",
                repository / "target" / "release" / "herdr-desktop-pet",
                repository / "native" / "target" / "debug" / "herdr-desktop-pet",
                repository / "native" / "target" / "release" / "herdr-desktop-pet",
                repository / "Contents" / "MacOS" / "herdr-desktop-pet",
            ]
        )
    unique: list[Path] = []
    seen: set[str] = set()
    for candidate in candidates:
        candidate = candidate.absolute()
        key = str(candidate)
        if key not in seen:
            unique.append(candidate)
            seen.add(key)
    return unique


def locate_native(explicit: Path | None) -> Path:
    candidates = native_candidates(explicit)
    for candidate in candidates:
        try:
            metadata = candidate.lstat()
        except OSError:
            continue
        if stat.S_ISLNK(metadata.st_mode):
            continue
        if stat.S_ISREG(metadata.st_mode) and os.access(candidate, os.X_OK):
            return candidate
    searched = ", ".join(str(candidate) for candidate in candidates)
    fail(f"native executable is unavailable; pass --native PATH or set HERDR_DESKTOP_PET_BIN (searched: {searched})")


def run_native(native: Path, arguments: Sequence[str], *, emit: bool = True) -> dict[str, Any]:
    command = [str(native), *arguments]
    try:
        completed = subprocess.run(command, check=False, capture_output=True, timeout=180)
    except OSError as error:
        fail(f"cannot execute native validation command {native}: {error}")
    except subprocess.TimeoutExpired:
        fail("native validation command exceeded the 180-second creator-tool timeout")
    stdout = completed.stdout.decode("utf-8", "replace")
    stderr = completed.stderr.decode("utf-8", "replace").strip()
    if completed.returncode != 0:
        detail = stderr or stdout.strip() or f"exit status {completed.returncode}"
        fail(f"native command failed; full native validation did not pass: {detail}")
    text = stdout.strip()
    if not text:
        fail("native command completed without machine-readable JSON output")
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError as error:
        fail(f"native command returned non-JSON output; parser-only success is not accepted: {error}")
    if not isinstance(parsed, dict):
        fail("native command returned a non-object JSON response")
    # Preserve the native response fields rather than inventing a second status
    # protocol.  The native command remains the authority for full validation.
    if emit:
        print(json.dumps(parsed, ensure_ascii=False, separators=(",", ":")))
    return parsed


def read_pack_entries(root: Path) -> tuple[bytes, dict[str, Any], list[tuple[str, bytes]]]:
    root = check_root(root)
    manifest_raw, manifest = load_manifest(root)
    _, _, files, records = inventory_records(root)
    declared = {
        item["path"]: (item["size"], item["sha256"], item["kind"])
        for item in manifest["payloads"]
    }
    actual = {
        record["path"]: (record["size"], record["sha256"], record["kind"])
        for record in records
    }
    if declared != actual:
        fail("manifest inventory is stale; run inventory before packaging")
    by_name = {name: data for name, data in files.items()}
    entries: list[tuple[str, bytes]] = [(MANIFEST_NAME, manifest_raw)]
    for record in records:
        data = by_name[record["path"]]
        expected_hash = record["sha256"]
        if len(data) != record["size"] or hashlib.sha256(data).hexdigest() != expected_hash:
            fail(f"payload {record['path']} changed during packaging")
        entries.append((record["path"], data))
    return manifest_raw, manifest, entries


def safe_archive_name(name: str) -> None:
    validate_filename(name, allow_manifest=name == MANIFEST_NAME)


def deterministic_zip(entries: Sequence[tuple[str, bytes]]) -> bytes:
    if not entries or len(entries) > MAX_PAYLOADS + 1 or sum(len(data) for _, data in entries) > MAX_EXPANDED_BYTES:
        fail("archive entries exceed bounded v4/v5 archive limits")
    names: set[bytes] = set()
    output = bytearray()
    central = bytearray()
    for name, payload in entries:
        safe_archive_name(name)
        folded = case_key(name)
        if folded in names:
            fail(f"archive contains duplicate or case-colliding entry {name!r}")
        names.add(folded)
        offset = len(output)
        crc = zlib.crc32(payload) & 0xFFFFFFFF
        if len(payload) > 0xFFFFFFFF or len(name.encode("ascii")) > 0xFFFF or offset > 0xFFFFFFFF:
            fail("archive cannot be represented as ZIP32")
        encoded_name = name.encode("ascii")
        output.extend(b"PK\x03\x04")
        output.extend(struct.pack("<HHHHHIIIHH", 20, 0, 0, 0, 0, crc, len(payload), len(payload), len(encoded_name), 0))
        output.extend(encoded_name)
        output.extend(payload)
        central.extend(b"PK\x01\x02")
        central.extend(struct.pack("<HHHHHHIIIHHHHHII", 20, 20, 0, 0, 0, 0, crc, len(payload), len(payload), len(encoded_name), 0, 0, 0, 0, 0, offset))
        central.extend(encoded_name)
    central_offset = len(output)
    output.extend(central)
    if central_offset > 0xFFFFFFFF or len(central) > 0xFFFFFFFF or len(entries) > 0xFFFF:
        fail("archive central directory exceeds ZIP32 limits")
    output.extend(b"PK\x05\x06")
    output.extend(struct.pack("<HHHHIIH", 0, 0, len(entries), len(entries), len(central), central_offset, 0))
    if len(output) > MAX_ARCHIVE_BYTES:
        fail(f"archive exceeds {MAX_ARCHIVE_BYTES} bytes")
    return bytes(output)

def publish_archive(output: Path, archive: bytes, native: Path) -> dict[str, Any]:
    output = output.absolute()
    if output.suffix != ".herdrchar":
        fail("archive output must use the .herdrchar extension")
    if output.exists() or output.is_symlink():
        fail(f"archive output already exists: {output}")
    parent = output.parent
    if not parent.exists() or parent.is_symlink() or not parent.is_dir():
        fail(f"archive output parent must be a real directory: {parent}")
    fd, temporary = tempfile.mkstemp(prefix=f".{output.name}.", suffix=".herdrchar", dir=str(parent))
    temporary_path = Path(temporary)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(archive)
            stream.flush()
            os.fsync(stream.fileno())
        # Validate the actual archive path before making it visible at the
        # requested destination.  Native validation is deliberately mandatory.
        native_result = run_native(native, ["pack", "validate", "--path", str(temporary_path)], emit=False)
        try:
            os.link(temporary_path, output, follow_symlinks=False)
        except FileExistsError:
            fail(f"archive output appeared during validation and was not overwritten: {output}")
        os.unlink(temporary_path)
    except Exception:
        try:
            temporary_path.unlink()
        except OSError:
            pass
        raise
    return {
        "ok": True,
        "command": "package",
        "path": str(output),
        "bytes": len(archive),
        "sha256": hashlib.sha256(archive).hexdigest(),
        "native_validation": native_result,
    }


def command_template(args: argparse.Namespace) -> dict[str, Any]:
    source = find_template_root(Path(args.templates_root) if args.templates_root else None, args.mode)
    destination = Path(args.output).absolute()
    return copy_template(source, destination, args.id, args.name)


def command_inventory(args: argparse.Namespace) -> dict[str, Any]:
    return regenerate_inventory(check_root(Path(args.path)), check_only=args.check)

def publish_rig_entry(
    root: Path,
    native: Path,
    entry_name: str,
    entry_raw: bytes,
    manifest_raw: bytes,
    updated_entry: Mapping[str, Any],
) -> tuple[dict[str, Any], dict[str, Any]]:
    """Write the entry, refresh inventory, and validate natively.

    Any failure restores the original entry and manifest bytes so a rejected
    assembly never leaves the authored pack modified.
    """
    try:
        write_atomic(root / entry_name, canonical_json(updated_entry))
        inventory = regenerate_inventory(root)
        native_result = run_native(native, ["pack", "validate", "--path", str(root)], emit=False)
    except BaseException:
        write_atomic(root / entry_name, entry_raw)
        write_atomic(root / MANIFEST_NAME, manifest_raw)
        raise
    return inventory, native_result


def command_rig(args: argparse.Namespace) -> dict[str, Any]:
    root = check_root(Path(args.path))
    manifest_raw, manifest, _, _ = inventory_records(root)
    entry_name, entry_raw, existing_entry = load_rig_entry(root, manifest)
    native = locate_native(Path(args.native) if args.native else None)
    canvas_width = motion_integer(manifest.get("width"), "manifest.width")
    canvas_height = motion_integer(manifest.get("height"), "manifest.height")
    if not 1 <= canvas_width <= 1024 or not 1 <= canvas_height <= 1024:
        fail("rig manifest canvas must be within 1..1024 pixels")
    canvas_extent = max(canvas_width, canvas_height)

    if manifest.get("version") == 5:
        if manifest.get("render_mode") != "rig":
            fail("v5 rig assembly requires a rig manifest")
        models_file = getattr(args, "models_file", None)
        if models_file is None:
            fail("v5 rig assembly requires --models-file containing a version 2 entry")
        validate_filename(models_file)
        model_entry = decode_object(
            read_bytes(root / models_file, MAX_ENTRY_BYTES, "v5 rig entry"),
            "v5 rig entry",
        )
        if set(model_entry) != {"version", "initial", "models", "bindings"}:
            fail("v5 rig entry must contain exactly version, initial, models, and bindings")
        if model_entry["version"] != 2 or model_entry["initial"] != "waiting":
            fail("v5 rig entry must use version 2 and initial waiting")
        models = model_entry["models"]
        bindings = model_entry["bindings"]
        if not isinstance(models, list) or not 10 <= len(models) <= 16:
            fail("v5 rig entry must contain between 10 and 16 models")
        if not isinstance(bindings, dict) or set(bindings) != {
            "waiting", "writing", "failed", "cancelled", "disconnected",
            "bored", "happy", "head-tap", "torso-tap", "head-pet",
        }:
            fail("v5 rig entry bindings must name all ten semantic poses exactly")
        model_ids: set[str] = set()
        payload_names: set[str] = set()
        validated_models: list[dict[str, str]] = []
        for index, model in enumerate(models):
            if not isinstance(model, dict) or set(model) != {"id", "file", "overrides", "motion"}:
                fail(f"v5 model {index} must contain exactly id, file, overrides, and motion")
            model_id = validate_pose_id(model["id"], f"v5 model {index} id")
            if model_id in model_ids:
                fail(f"v5 model id is duplicated: {model_id!r}")
            model_ids.add(model_id)
            paths: dict[str, str] = {}
            for field, kind, label in (
                ("file", "psd", "v5 model PSD"),
                ("overrides", "overrides", "v5 model overrides"),
                ("motion", "motion", "v5 model motion"),
            ):
                value = model[field]
                if not isinstance(value, str):
                    fail(f"v5 model {index} {field} must be a filename")
                validate_filename(value)
                if value in payload_names:
                    fail(f"v5 model payload is referenced more than once: {value!r}")
                payload_names.add(value)
                _, payload_data = read_root_payload(root, value, kind, label)
                if field == "motion":
                    validate_v2_motion(payload_data, label, {model_id}, canvas_extent)
                paths[field] = value
            validated_models.append({
                "id": model_id,
                "file": paths["file"],
                "overrides": paths["overrides"],
                "motion": paths["motion"],
            })
        bound_ids: set[str] = set()
        for pose_name, model_id in bindings.items():
            if not isinstance(model_id, str) or model_id not in model_ids:
                fail(f"v5 binding {pose_name!r} references an unknown model")
            if model_id in bound_ids:
                fail("v5 semantic bindings may not reference one model more than once")
            bound_ids.add(model_id)
        if bindings["waiting"] not in model_ids:
            fail("v5 waiting binding references an unknown model")
        updated_entry = {
            "version": 2,
            "initial": "waiting",
            "models": validated_models,
            "bindings": {name: bindings[name] for name in (
                "waiting", "writing", "failed", "cancelled", "disconnected",
                "bored", "happy", "head-tap", "torso-tap", "head-pet",
            )},
        }
        inventory, native_result = publish_rig_entry(
            root, native, entry_name, entry_raw, manifest_raw, updated_entry,
        )
        return {
            "ok": True,
            "command": "rig",
            "path": str(root),
            "entry": entry_name,
            "models": validated_models,
            "bindings": updated_entry["bindings"],
            "inventory": inventory,
            "native_validation": native_result,
        }

    if manifest.get("version") != 4:
        fail("rig assembly requires a v4 or v5 manifest")
    if args.base_file is None or args.base_id is None:
        fail("v4 rig assembly requires --base-file and --base-id")
    base_file = args.base_file
    base_id = validate_pose_id(args.base_id, "--base-id")
    read_root_payload(root, base_file, "psd", "base PSD")

    if (args.pose_file is None) != (args.pose_id is None):
        fail("--pose-file and --pose-id must be supplied together")
    pose_file: str | None = None
    pose_id: str | None = None
    if args.pose_file is not None:
        pose_file = args.pose_file
        pose_id = validate_pose_id(args.pose_id, "--pose-id")
        if pose_id == base_id:
            fail("--pose-id must differ from --base-id")
        read_root_payload(root, pose_file, "psd", "pose PSD")

    overrides_file = args.overrides_file
    if overrides_file is None:
        overrides_file = existing_entry.get("overrides", "rig-overrides.json")
    motion_file = args.motion_file
    if motion_file is None:
        motion_file = existing_entry.get("motion", "motion.json")
    if not isinstance(overrides_file, str):
        fail("rig entry overrides filename must be a root-local filename")
    if not isinstance(motion_file, str):
        fail("rig entry motion filename must be a root-local filename")
    read_root_payload(root, overrides_file, "overrides", "rig overrides")
    _, motion_data = read_root_payload(root, motion_file, "motion", "rig motion")
    validate_v2_motion(
        motion_data,
        "rig motion",
        {base_id} | ({pose_id} if pose_id is not None else set()),
        canvas_extent,
    )

    updated_entry: dict[str, Any] = {
        "version": 1,
        "base": {"id": base_id, "file": base_file},
        "overrides": overrides_file,
        "motion": motion_file,
    }
    if pose_file is not None and pose_id is not None:
        updated_entry["pose"] = {"id": pose_id, "file": pose_file}

    inventory, native_result = publish_rig_entry(
        root, native, entry_name, entry_raw, manifest_raw, updated_entry,
    )
    result: dict[str, Any] = {
        "ok": True,
        "command": "rig",
        "path": str(root),
        "entry": entry_name,
        "base": {"id": base_id, "file": base_file},
        "overrides": overrides_file,
        "motion": motion_file,
        "inventory": inventory,
        "native_validation": native_result,
    }
    if pose_file is not None and pose_id is not None:
        result["pose"] = {"id": pose_id, "file": pose_file}
    return result


def command_validate(args: argparse.Namespace) -> None:
    path = Path(args.path).absolute()
    if not path.exists() and not path.is_symlink():
        fail(f"character path does not exist: {path}")
    # Directory preflight protects links and bounds, while native validation
    # remains the only success authority.
    if path.is_dir():
        inventory_records(path)
    native = locate_native(Path(args.native) if args.native else None)
    run_native(native, ["pack", "validate", "--path", str(path)])

def preview_command(path: Path, output: Path, *, phase: str | None = None, time_ms: int | None = None, reaction: str | None = None, reaction_age_ms: int | None = None, hit_overlay: bool = False) -> list[str]:
    command = ["pack", "preview", "--path", str(path), "--output", str(output)]
    if phase is not None:
        command.extend(["--phase", phase])
    if time_ms is not None:
        command.extend(["--time-ms", str(time_ms)])
    if reaction is not None:
        command.extend(["--reaction", reaction])
    if reaction_age_ms is not None:
        command.extend(["--reaction-age-ms", str(reaction_age_ms)])
    if hit_overlay:
        command.append("--hit-overlay")
    return command


def command_preview(args: argparse.Namespace) -> None:
    path = Path(args.path).absolute()
    output = Path(args.output).absolute()
    if not path.exists() and not path.is_symlink():
        fail(f"character path does not exist: {path}")
    if output.exists() or output.is_symlink():
        fail(f"preview output already exists: {output}")
    if output.parent.is_symlink() or not output.parent.is_dir():
        fail(f"preview output parent must be a real directory: {output.parent}")
    if path.is_dir():
        inventory_records(path)
    native = locate_native(Path(args.native) if args.native else None)
    command = preview_command(
        path,
        output,
        phase=args.phase,
        time_ms=args.time_ms,
        reaction=args.reaction,
        reaction_age_ms=args.reaction_age_ms,
        hit_overlay=args.hit_overlay,
    )
    run_native(native, command)


def command_review(args: argparse.Namespace) -> dict[str, Any]:
    path = Path(args.path).absolute()
    if not path.exists() and not path.is_symlink():
        fail(f"character path does not exist: {path}")
    if path.is_dir():
        inventory_records(path)
    native = locate_native(Path(args.native) if args.native else None)
    validation_command = ["pack", "validate", "--path", str(path)]
    native_validation = run_native(native, validation_command, emit=False)
    output = new_private_directory(Path(args.output), "review output")
    captures: list[dict[str, Any]] = []

    def capture(
        scene: str,
        filename: str,
        *,
        phase: str,
        time_ms: int,
        reaction: str | None = None,
        reaction_age_ms: int | None = None,
        hit_overlay: bool,
    ) -> None:
        output_path = output / filename
        command = preview_command(
            path,
            output_path,
            phase=phase,
            time_ms=time_ms,
            reaction=reaction,
            reaction_age_ms=reaction_age_ms,
            hit_overlay=hit_overlay,
        )
        native_result = run_native(native, command, emit=False)
        if native_result.get("command") != "preview":
            fail(f"native review capture returned the wrong command for {scene}")
        if native_result.get("hit_overlay") is not hit_overlay:
            fail(f"native review capture did not report the requested hit overlay for {scene}")
        data = read_bytes(output_path, MAX_PREVIEW_BYTES, "native review PNG")
        if not data.startswith(b"\x89PNG\r\n\x1a\n"):
            fail(f"native review output is not a PNG: {output_path}")
        controls: dict[str, Any] = {
            "phase": phase,
            "time_ms": time_ms,
            "hit_overlay": hit_overlay,
        }
        if reaction is not None:
            controls["reaction"] = reaction
            controls["reaction_age_ms"] = reaction_age_ms if reaction_age_ms is not None else 0
        captures.append(
            {
                "scene": scene,
                "variant": "hit_overlay" if hit_overlay else "baseline",
                "controls": controls,
                "command": [str(native), *command],
                "output": {
                    "path": str(output_path),
                    "bytes": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(),
                },
                "native_result": native_result,
            }
        )

    try:
        for phase in PHASES:
            capture(
                f"phase:{phase}",
                f"phase-{phase}-baseline.png",
                phase=phase,
                time_ms=args.time_ms,
                hit_overlay=False,
            )
            capture(
                f"phase:{phase}",
                f"phase-{phase}-hit-overlay.png",
                phase=phase,
                time_ms=args.time_ms,
                hit_overlay=True,
            )
        for reaction in REACTIONS:
            capture(
                f"reaction:{reaction}",
                f"reaction-{reaction}-baseline.png",
                phase="idle",
                time_ms=args.time_ms,
                reaction=reaction,
                reaction_age_ms=args.reaction_age_ms,
                hit_overlay=False,
            )
            capture(
                f"reaction:{reaction}",
                f"reaction-{reaction}-hit-overlay.png",
                phase="idle",
                time_ms=args.time_ms,
                reaction=reaction,
                reaction_age_ms=args.reaction_age_ms,
                hit_overlay=True,
            )
        review_manifest = {
            "version": 1,
            "command": "review",
            "path": str(path),
            "output": str(output),
            "native": str(native),
            "controls": {
                "time_ms": args.time_ms,
                "reaction_age_ms": args.reaction_age_ms,
            },
            "native_validation": {
                "command": [str(native), *validation_command],
                "result": native_validation,
            },
            "captures": captures,
        }
        manifest_path = output / "review.json"
        write_atomic(manifest_path, canonical_json(review_manifest))
    except Exception:
        remove_private_directory(output)
        raise

    return {
        "ok": True,
        "command": "review",
        "path": str(path),
        "output": str(output),
        "manifest": str(manifest_path),
        "capture_count": len(captures),
        "native_validation": native_validation,
    }


def command_package(args: argparse.Namespace) -> dict[str, Any]:
    root = check_root(Path(args.path))
    native = locate_native(Path(args.native) if args.native else None)
    _, _, entries = read_pack_entries(root)
    archive = deterministic_zip(entries)
    result = publish_archive(Path(args.output), archive, native)
    print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
    return result


def command_export(args: argparse.Namespace) -> None:
    output = Path(args.output).absolute()
    if output.exists() or output.is_symlink():
        fail(f"archive output already exists: {output}")
    if output.suffix != ".herdrchar":
        fail("export output must use the .herdrchar extension")
    if output.parent.is_symlink() or not output.parent.is_dir():
        fail(f"export output parent must be a real directory: {output.parent}")
    native = locate_native(Path(args.native) if args.native else None)
    command = ["pack", "export", args.id]
    if args.revision is not None:
        command.extend(["--revision", str(args.revision)])
    command.extend(["--output", str(output)])
    run_native(native, command)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    template = commands.add_parser("template", help="copy a v4 template and regenerate its inventory")
    template.add_argument("--mode", choices=("png", "rig"), required=True)
    template.add_argument("--output", required=True)
    template.add_argument("--id")
    template.add_argument("--name")
    template.add_argument("--templates-root")
    template.set_defaults(handler=command_template)

    inventory = commands.add_parser("inventory", help="regenerate payload sizes and SHA-256 hashes")
    inventory.add_argument("--path", required=True)
    inventory.add_argument("--check", action="store_true", help="report drift without rewriting manifest.json")
    inventory.set_defaults(handler=command_inventory)

    rig = commands.add_parser("rig", help="assemble and natively validate an authored v4/v5 rig entry")
    rig.add_argument("--path", required=True)
    rig.add_argument("--base-file", help="v4 root-local base PSD filename")
    rig.add_argument("--base-id", help="v4 artistic base pose ID")
    rig.add_argument("--pose-file", help="v4 root-local optional alternate PSD filename")
    rig.add_argument("--pose-id", help="v4 artistic alternate pose ID")
    rig.add_argument("--models-file", help="v5 root-local JSON entry containing the model catalog")
    rig.add_argument("--overrides-file", help="v4 root-local overrides JSON filename (defaults to the existing entry)")
    rig.add_argument("--motion-file", help="v4 root-local motion JSON filename (defaults to the existing entry)")
    rig.add_argument("--native")
    rig.set_defaults(handler=command_rig)

    validate = commands.add_parser("validate", help="run native full validation")
    validate.add_argument("--path", required=True)
    validate.add_argument("--native")
    validate.set_defaults(handler=command_validate)

    preview = commands.add_parser("preview", help="run a fixed-time native preview")
    preview.add_argument("--path", required=True)
    preview.add_argument("--output", required=True)
    preview.add_argument("--native")
    preview.add_argument("--phase", choices=PHASES)
    preview.add_argument("--time-ms", type=nonnegative_int)
    preview.add_argument("--reaction", choices=REACTIONS)
    preview.add_argument("--reaction-age-ms", type=nonnegative_int)
    preview.add_argument("--hit-overlay", action="store_true", help="ask native preview to draw sampled semantic hit regions")
    preview.set_defaults(handler=command_preview)
    review = commands.add_parser("review", help="capture native four-phase/four-reaction baseline and hit-overlay scenes")
    review.add_argument("--path", required=True)
    review.add_argument("--output", required=True)
    review.add_argument("--native")
    review.add_argument("--time-ms", type=nonnegative_int, required=True)
    review.add_argument("--reaction-age-ms", type=nonnegative_int, default=0)
    review.set_defaults(handler=command_review)

    package = commands.add_parser("package", help="write and natively validate a deterministic .herdrchar archive")
    package.add_argument("--path", required=True)
    package.add_argument("--output", required=True)
    package.add_argument("--native")
    package.set_defaults(handler=command_package)

    export = commands.add_parser("export", help="invoke native immutable store export")
    export.add_argument("id")
    export.add_argument("--revision", type=nonnegative_int)
    export.add_argument("--output", required=True)
    export.add_argument("--native")
    export.set_defaults(handler=command_export)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    try:
        args = parser.parse_args(argv)
        result = args.handler(args)
        if result is not None and args.command != "package":
            print(json.dumps(result, ensure_ascii=False, separators=(",", ":")))
        return 0
    except ToolError as error:
        print(json.dumps({"ok": False, "error": str(error)}, ensure_ascii=False, separators=(",", ":")), file=sys.stderr)
        return 1
    except BrokenPipeError:
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
