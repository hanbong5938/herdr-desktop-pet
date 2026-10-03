"""Shared run-configuration contract for the local Qwen character pipeline.

Every pipeline script takes ``--run RUN`` and loads ``RUN/run.json`` through
``load_run``. Relative paths in the configuration resolve against the run root.
Nothing here performs network access, image editing or file deletion.
"""
from __future__ import annotations

import hashlib
import json
import os
import uuid
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 1
POSES = ('waiting', 'writing', 'failed', 'cancelled', 'disconnected',
         'bored', 'happy', 'head-tap', 'torso-tap', 'head-pet')
CANVAS = 1280


class RunConfigError(ValueError):
    pass


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, 'rb') as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def load_json(path: Path) -> Any:
    with open(path, 'rb') as handle:
        return json.loads(handle.read().decode('utf-8'))


def encode_json(value: Any) -> bytes:
    return (json.dumps(value, indent=2, ensure_ascii=False) + '\n').encode('utf-8')


def write_new(path: Path, data: bytes) -> None:
    """Create ``path`` exclusively; never overwrite preserved evidence."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, 'xb') as handle:
        handle.write(data)


def write_new_json(path: Path, value: Any) -> None:
    write_new(path, encode_json(value))


def replace_json(path: Path, value: Any) -> None:
    """Atomically replace a derived index (never raw artwork or provenance)."""
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f'{path.name}.{uuid.uuid4().hex}.tmp')
    write_new(temporary, encode_json(value))
    os.replace(temporary, path)


def _box(value: Any, label: str, canvas: int | None = None) -> list[int]:
    if (not isinstance(value, list) or len(value) != 4
            or any(type(item) is not int for item in value)):
        raise RunConfigError(f'{label} must be four integers [x0, y0, x1, y1]')
    x0, y0, x1, y1 = value
    if x0 < 0 or y0 < 0 or x1 <= x0 or y1 <= y0:
        raise RunConfigError(f'{label} is empty or negative: {value}')
    if canvas is not None and (x1 > canvas or y1 > canvas):
        raise RunConfigError(f'{label} exceeds the {canvas}px canvas: {value}')
    return value


class Run:
    """Validated view of ``RUN/run.json``."""

    def __init__(self, root: Path, data: dict[str, Any]):
        self.root = root
        self.data = data

    def path(self, value: str) -> Path:
        candidate = Path(value).expanduser()
        return candidate if candidate.is_absolute() else self.root / candidate

    def pose_dir(self, pose: str) -> Path:
        if pose not in self.poses:
            raise RunConfigError(f'Unknown pose: {pose}')
        return self.root / 'poses' / pose

    def select(self, argument: str) -> list[str]:
        if argument == 'all':
            return list(self.poses)
        chosen = [item.strip() for item in argument.split(',') if item.strip()]
        unknown = [item for item in chosen if item not in self.poses]
        if not chosen or unknown or len(set(chosen)) != len(chosen):
            raise RunConfigError(f'Invalid pose selection {argument!r}; choose from {", ".join(self.poses)}')
        return chosen

    def checked_input(self, key: str) -> tuple[Path, bytes]:
        """Return a hash-verified input image declared under ``key``."""
        entry = self.data[key]
        path = self.path(entry['path'])
        data = path.read_bytes()
        if sha256_bytes(data) != entry['sha256']:
            raise RunConfigError(f'{key} SHA-256 mismatch for {path}')
        return path, data

    @property
    def character(self) -> dict[str, str]:
        return self.data['character']

    @property
    def poses(self) -> tuple[str, ...]:
        return tuple(self.data['poses'])

    @property
    def canvas(self) -> int:
        return self.data['canvas']

    @property
    def authority(self) -> dict[str, Any]:
        return self.data['authority']

    @property
    def identity(self) -> dict[str, Any]:
        return self.data['identityReference']

    @property
    def prompts(self) -> dict[str, Any]:
        return self.data['prompts']

    @property
    def endpoints(self) -> dict[str, str]:
        return self.data['endpoints']

    @property
    def seeds(self) -> dict[str, int]:
        return self.data['seeds']

    def tool(self, key: str) -> Path:
        return self.path(self.data['tools'][key])


def load_run(root: str | Path) -> Run:
    root = Path(root).expanduser().resolve()
    config = root / 'run.json'
    if not config.is_file():
        raise RunConfigError(f'Missing run configuration: {config}')
    data = load_json(config)
    if data.get('schemaVersion') != SCHEMA_VERSION:
        raise RunConfigError('run.json schemaVersion must be 1')
    character = data.get('character')
    if not isinstance(character, dict) or not character.get('id') or not character.get('label'):
        raise RunConfigError('run.json character requires id and label')
    if data.get('canvas') != CANVAS:
        raise RunConfigError(f'run.json canvas must be {CANVAS}')
    poses = data.get('poses')
    if not isinstance(poses, list) or sorted(poses) != sorted(POSES) or len(poses) != len(POSES):
        raise RunConfigError(f'run.json poses must list exactly: {", ".join(POSES)}')
    authority = data.get('authority')
    if not isinstance(authority, dict) or not authority.get('path') or not authority.get('sha256'):
        raise RunConfigError('run.json authority requires path and sha256')
    _box(authority.get('headCrop'), 'authority.headCrop', CANVAS)
    if authority.get('sourcePose') not in POSES:
        raise RunConfigError('authority.sourcePose must be one semantic pose ID')
    identity = data.get('identityReference')
    if not isinstance(identity, dict) or not identity.get('path') or not identity.get('sha256'):
        raise RunConfigError('run.json identityReference requires path and sha256')
    _box(identity.get('faceCrop'), 'identityReference.faceCrop')
    prompts = data.get('prompts')
    required_prompts = ('identityCorrect', 'pose', 'expression')
    if not isinstance(prompts, dict) or any(not isinstance(prompts.get(k), str) or not prompts[k] for k in required_prompts):
        raise RunConfigError('run.json prompts require identityCorrect, pose and expression text')
    intents = prompts.get('poseIntents')
    missing = [p for p in POSES if p != authority['sourcePose'] and not (isinstance(intents, dict) and intents.get(p))]
    if missing:
        raise RunConfigError(f'prompts.poseIntents missing: {", ".join(missing)}')
    if '{intent}' not in prompts['pose']:
        raise RunConfigError('prompts.pose must contain the {intent} placeholder')
    endpoints = data.get('endpoints')
    if not isinstance(endpoints, dict) or not endpoints.get('qwen') or not endpoints.get('seeThrough'):
        raise RunConfigError('run.json endpoints require qwen and seeThrough URLs')
    seeds = data.get('seeds')
    if not isinstance(seeds, dict) or any(type(seeds.get(k)) is not int or not 0 <= seeds[k] < 2 ** 63
                                          for k in ('identity', 'poses', 'expressions', 'seeThrough')):
        raise RunConfigError('run.json seeds require identity, poses, expressions and seeThrough integers')
    tools = data.get('tools')
    if not isinstance(tools, dict) or any(not tools.get(k) for k in ('runtime', 'mattingModel', 'adapter', 'motionReference')):
        raise RunConfigError('run.json tools require runtime, mattingModel, adapter and motionReference')
    return Run(root, data)
