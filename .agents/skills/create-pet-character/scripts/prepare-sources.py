#!/usr/bin/env python3
"""Immutable per-pose matte and See-through source preparation for one run.

``matte`` runs with the matting-env Python (onnxruntime CPU, scipy, optional
PyMatting refinement), clears partial-alpha pixels that are plain generation
background (enclosed background pockets) and writes
``poses/<pose>/inputs/master.png`` plus ``matting.json``; ``--supersede-rejected``
first archives a pose whose matte was rejected. ``decompose`` runs with ordinary
Python and Node.js, submits the matte to the See-through ComfyUI server only when
both ComfyUI servers are idle, and writes ``poses/<pose>/native/`` parts,
``<pose>_layers.json`` and
``generation.json``. Every attempt keeps its raw files and failure records;
existing outputs are never overwritten.
"""
from __future__ import annotations

import argparse
import importlib.metadata
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path
from typing import Any, Optional

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_config import (  # noqa: E402
    Run,
    RunConfigError,
    encode_json,
    load_json,
    load_run,
    sha256_file,
    write_new,
)

SEETHROUGH_STEPS = 30
DEFAULT_VRAM_GIB = 64
MATTE_METHOD_VERSION = 2
# Background-pocket suppression (methodVersion 2). Enclosed plain-background
# pockets (between hair strands, between hair and arm) can keep partial matte
# alpha with near-background RGB; they render as grey bands on dark desktops.
# Pixels within POCKET_TOLERANCE (max per-channel |raw - background|, 0-255) of
# the border background colour and with alpha below POCKET_ALPHA_BELOW lose
# their alpha. Opaque light garment interiors (alpha >= 0.98) stay untouched.
POCKET_TOLERANCE = 16
POCKET_ALPHA_BELOW = 250  # uint8, i.e. alpha < 0.98
BORDER_UNIFORM_FRACTION = 0.75  # border pixels that must match the background colour
POCKET_COMPONENT_MIN_PIXELS = 64
MATTE_REVIEW = 'matte-review.json'
SUPERSEDED_DIR = 'superseded'
ARCHIVE_MANIFEST = 'superseded.json'
REQUIRED_NODES = ('LoadImage', 'JoinImageWithAlpha', 'SeeThrough_LoadLayerDiffModel',
                  'SeeThrough_LoadDepthModel', 'SeeThrough_GenerateLayers',
                  'SeeThrough_GenerateDepth', 'SeeThrough_PostProcess', 'SeeThrough_SavePSD', 'PreviewImage')
# A garment may be decomposed as one topwear piece (camisole + shorts); the rig
# needs some own lower-body plate, matching build-rigs.py geometry's `lower` rule.
REQUIRED_PARTS = {'face': {'face'}, 'hair': {'hair', 'hairf', 'hairb', 'front hair', 'back hair'},
                  'eyes': {'eyes', 'eyel', 'eyer', 'irides-l', 'irides-r', 'irides'},
                  'mouth': {'mouth'}, 'hands': {'handwear', 'handwear-l', 'handwear-r'},
                  'outfit-top': {'topwear'}, 'outfit-bottom': {'bottomwear', 'legwear', 'body'}}
OPTIONAL_PARTS = {'headwear', 'eyewear', 'wings', 'tail', 'objects'}


class Source:
    """One hash-verified pose source registered in ``RUN/source-index.json``."""

    def __init__(self, path: Path, sha256: str, provenance: Path):
        self.path = path
        self.sha256 = sha256
        self.provenance = provenance


def exclusive(path: Path, data: bytes) -> None:
    """Create ``path``; an existing file is accepted only with identical bytes."""
    try:
        write_new(path, data)
    except FileExistsError:
        if path.read_bytes() != data:
            raise RuntimeError(f'Existing bytes differ; never overwrite: {path}')


def copy_exact(src: Path, dest: Path) -> None:
    exclusive(dest, src.read_bytes())
    if sha256_file(src) != sha256_file(dest):
        raise RuntimeError(f'Copy mismatch: {dest}')


def new_attempt(root: Path) -> Path:
    root.mkdir(parents=True, exist_ok=True)
    folder = root / (time.strftime('%Y%m%d-%H%M%S') + '-' + uuid.uuid4().hex[:10])
    folder.mkdir(exist_ok=False)
    return folder


def record_failure(folder: Path, exc: BaseException) -> None:
    name = 'failure.json' if not (folder / 'failure.json').exists() else f'recovery-failure-{uuid.uuid4().hex}.json'
    exclusive(folder / name, encode_json({'error': str(exc), 'type': type(exc).__name__, 'time': time.time()}))


def shown(run: Run, path: Path) -> str:
    """Record run-local paths relative to the run root, external paths absolutely."""
    resolved = path.resolve()
    try:
        return resolved.relative_to(run.root).as_posix()
    except ValueError:
        return str(resolved)


def slug(value: str) -> str:
    return re.sub(r'[^A-Za-z0-9._-]+', '-', value).strip('-') or 'character'


def load_sources(run: Run, selected: list[str]) -> dict[str, Source]:
    index_path = run.root / 'source-index.json'
    if not index_path.is_file():
        raise RuntimeError(f'{index_path} absent; run generate-sources.py poses first')
    index = load_json(index_path)
    if index.get('version', 1) != 1:
        raise RuntimeError('source-index.json version must be 1')
    recorded_authority = index.get('authority')
    if recorded_authority is not None and recorded_authority.get('sha256') != run.authority['sha256']:
        raise RuntimeError('source-index.json authority differs from run.json authority; regenerate sources')
    entries = index.get('poses')
    if not isinstance(entries, list):
        raise RuntimeError('source-index.json requires a poses list')
    ids = [entry.get('id') for entry in entries]
    if len(set(ids)) != len(ids) or any(pose not in run.poses for pose in ids):
        raise RuntimeError('source-index.json must list known pose IDs at most once')
    missing = [pose for pose in selected if pose not in ids]
    if missing:
        raise RuntimeError(f'source-index.json lacks selected poses: {", ".join(missing)}')
    result = {}
    for entry in entries:
        pose = entry['id']
        if pose not in selected:
            continue
        path = run.path(entry['path'])
        provenance = run.path(entry['provenance'])
        if not provenance.is_file():
            raise RuntimeError(f'Unverifiable source provenance for {pose}: {provenance}')
        if not path.is_file() or sha256_file(path) != entry['sha256']:
            raise RuntimeError(f'Source hash mismatch for {pose}: {path}')
        if pose == run.authority['sourcePose'] and entry['sha256'] != run.authority['sha256']:
            raise RuntimeError(f'Source pose {pose} differs from the run.json authority bytes')
        result[pose] = Source(path, entry['sha256'], provenance)
    return {pose: result[pose] for pose in selected}


def versions(names: list[str]) -> dict[str, str]:
    result = {'python': sys.version, 'executable': str(Path(sys.executable).resolve())}
    for name in names:
        result[name] = importlib.metadata.version(name)
    return result


def downstream_exists(run: Run, base: Path, pose: str) -> bool:
    native = base / 'native'
    if native.exists() and any(native.iterdir()):
        return True
    if (base / 'plan' / 'geometry.json').exists() or (run.root / 'native-models' / pose).exists():
        return True
    # Any rig round directory (or other derived folder) beside inputs/raw/native/plan.
    return any(child.name not in ('inputs', 'raw', 'native', 'plan') for child in base.iterdir())


def suppress_background_pockets(rgb: Any, alpha: Any) -> tuple[Any, dict[str, Any]]:
    """Mask of partial-alpha pixels whose raw RGB is the plain generation background.

    The background colour is the per-channel median of the raw image's one-pixel
    border; the border must be a plain uniform background, otherwise the matte
    fails instead of guessing.
    """
    import numpy as np
    from scipy.ndimage import find_objects, label

    border = np.concatenate((rgb[0], rgb[-1], rgb[1:-1, 0], rgb[1:-1, -1])).astype(np.int16)
    background = np.median(border, axis=0)
    uniform = float(np.mean(np.abs(border - background).max(axis=1) <= POCKET_TOLERANCE))
    if uniform < BORDER_UNIFORM_FRACTION:
        raise RuntimeError(f'Raw image border is not a plain uniform background ({uniform:.1%} of border pixels '
                           f'within {POCKET_TOLERANCE} of median {background.tolist()}; need '
                           f'{BORDER_UNIFORM_FRACTION:.0%}); cannot suppress background pockets')
    distance = np.abs(rgb.astype(np.int16) - background).max(axis=2)
    cleared = (distance <= POCKET_TOLERANCE) & (alpha > 0) & (alpha < POCKET_ALPHA_BELOW)
    labels, _ = label(cleared, structure=np.ones((3, 3), bool))
    components = []
    for index, box in enumerate(find_objects(labels), 1):
        inside = labels[box] == index
        size = int(np.count_nonzero(inside))
        if size >= POCKET_COMPONENT_MIN_PIXELS:
            components.append({'pixels': size, 'bounds': [box[1].start, box[0].start, box[1].stop, box[0].stop],
                               'medianAlpha': float(np.median(alpha[box][inside]))})
    components.sort(key=lambda item: -item['pixels'])
    record = {'backgroundRgb': [float(value) for value in background], 'borderUniformFraction': uniform,
              'tolerance': POCKET_TOLERANCE, 'toleranceMetric': 'max per-channel |raw RGB - backgroundRgb|',
              'alphaBelow': POCKET_ALPHA_BELOW / 255, 'alphaBelowUint8': POCKET_ALPHA_BELOW,
              'pixelsCleared': int(np.count_nonzero(cleared)),
              'alphaMassCleared': float(alpha[cleared].sum(dtype=np.int64)) / 255,
              'componentMinPixels': POCKET_COMPONENT_MIN_PIXELS, 'components': components}
    return cleared, record


def tree_hashes(root: Path) -> dict[str, str]:
    files = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise RuntimeError(f'Symlink {path} inside {root}; archive by hand')
        if path.is_file():
            files[path.relative_to(root).as_posix()] = sha256_file(path)
    return files


def archived_mattes(run: Run, pose: str) -> list[dict[str, Any]]:
    """Superseded-matte manifests under RUN/superseded/<pose>/, ordered 1..N."""
    root = run.root / SUPERSEDED_DIR / pose
    if not root.exists():
        return []
    manifests = []
    for entry in sorted(root.iterdir()):
        manifest_path = entry / ARCHIVE_MANIFEST
        if not entry.is_dir() or not manifest_path.is_file():
            raise RuntimeError(f'{entry} is not a superseded-matte archive ({ARCHIVE_MANIFEST} missing); inspect it by hand')
        manifest = load_json(manifest_path)
        if entry.name != f'{manifest.get("number")}-{str(manifest.get("masterSha256", ""))[:16]}':
            raise RuntimeError(f'Archive manifest does not describe its directory: {manifest_path}')
        manifests.append(manifest)
    manifests.sort(key=lambda item: item['number'])
    if [item['number'] for item in manifests] != list(range(1, len(manifests) + 1)):
        raise RuntimeError(f'Superseded-matte archives in {root} are not numbered 1..{len(manifests)}')
    return manifests


def complete_archive(run: Run, manifest: dict[str, Any], verify: bool) -> None:
    """Finish (or confirm) an archival by renaming each still-in-place tree; nothing is deleted."""
    for move in manifest['moves']:
        origin, destination = run.path(move['from']), run.path(move['to'])
        if not destination.exists():
            if not origin.is_dir() or (verify and tree_hashes(origin) != move['files']):
                raise RuntimeError(f'Cannot complete archival of {origin}: missing or changed; '
                                   f'inspect {destination.parent}')
            os.rename(origin, destination)
        if move.get('recreateEmpty'):
            origin.mkdir(exist_ok=True)


def supersede_rejected_matte(run: Run, pose: str) -> None:
    """Archive a pose whose published matte is rejected by ``inputs/matte-review.json``.

    Renames the whole ``poses/<pose>/`` tree (leaving a new empty root) and
    ``native-models/<pose>/`` into ``superseded/<pose>/<n>-<masterSha[:16]>/``
    after writing ``superseded.json``; an interrupted archival is finished first.
    A pose without a review is left for the ordinary matte resume/refusal.
    """
    archives = archived_mattes(run, pose)
    if archives:
        complete_archive(run, archives[-1], verify=True)
    base = run.pose_dir(pose)
    review_path = base / 'inputs' / MATTE_REVIEW
    if not review_path.exists():
        return
    master = base / 'inputs' / 'master.png'
    published = base / 'inputs' / 'matting.json'
    if not master.is_file() or not published.is_file():
        raise RuntimeError(f'{review_path} exists without a published matte to reject; inspect it')
    master_sha = sha256_file(master)
    record = load_json(published)
    if record.get('masterSha256') != master_sha:
        raise RuntimeError(f'Matte changed or incomplete for {pose}; preserve existing files and inspect attempts')
    review = load_json(review_path)
    problems = []
    if not isinstance(review, dict) or review.get('decision') != 'rejected':
        problems.append('decision must be "rejected"')
    elif review.get('masterSha256') != master_sha:
        problems.append('masterSha256 must equal the published inputs/master.png')
    elif not isinstance(review.get('reason'), str) or not review['reason'].strip():
        problems.append('reason is required')
    if problems:
        raise RuntimeError(f'{review_path} is not a valid rejection: ' + '; '.join(problems))
    number = len(archives) + 1
    archive = run.root / SUPERSEDED_DIR / pose / f'{number}-{master_sha[:16]}'
    moves = [{'from': shown(run, base), 'to': shown(run, archive / 'pose'), 'recreateEmpty': True,
              'files': tree_hashes(base)}]
    native = run.root / 'native-models' / pose
    if native.is_symlink():
        raise RuntimeError(f'{native} is a symlink; archive by hand')
    if native.exists():
        moves.append({'from': shown(run, native), 'to': shown(run, archive / 'native-model'),
                      'files': tree_hashes(native)})
    manifest = {'version': 1, 'pose': pose, 'number': number,
                'archivedAt': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
                'masterSha256': master_sha, 'mattingSha256': sha256_file(published),
                'methodVersion': record.get('inputs', {}).get('methodVersion'),
                'reason': review['reason'], 'review': review, 'reviewSha256': sha256_file(review_path),
                'moves': moves,
                'note': 'Paths recorded inside archived files refer to their pre-archive location; '
                        'moves maps each archived tree from -> to.',
                'method': 'rename into a new archive directory; nothing deleted or overwritten',
                'recordedBy': 'prepare-sources.py matte --supersede-rejected'}
    archive.parent.mkdir(parents=True, exist_ok=True)
    archive.mkdir()
    exclusive(archive / ARCHIVE_MANIFEST, encode_json(manifest))
    complete_archive(run, manifest, verify=False)
    print(f'{pose}: archived rejected matte {master_sha[:16]} and its downstream work into {archive}')


def matte(run: Run, args: argparse.Namespace, pose: str, source: Source) -> None:
    import numpy as np
    import onnxruntime as ort
    from PIL import Image
    from scipy.ndimage import binary_dilation, binary_erosion

    canvas_size = run.canvas
    model = run.tool('mattingModel')
    if not model.is_file():
        raise RuntimeError(f'Matting model missing: {model}')
    env = versions(['numpy', 'Pillow', 'onnxruntime', 'scipy'] + (['pymatting'] if args.refine else []))
    key = {'source': shown(run, source.path), 'sourceSha256': source.sha256,
           'provenance': shown(run, source.provenance), 'provenanceSha256': sha256_file(source.provenance),
           'model': shown(run, model), 'modelSha256': sha256_file(model),
           'environment': env, 'refine': args.refine, 'margin': args.margin, 'methodVersion': MATTE_METHOD_VERSION}
    base = run.pose_dir(pose)
    master = base / 'inputs' / 'master.png'
    published = base / 'inputs' / 'matting.json'
    archives = archived_mattes(run, pose)
    if archives:
        complete_archive(run, archives[-1], verify=True)
    review_path = base / 'inputs' / MATTE_REVIEW
    if review_path.exists():
        raise RuntimeError(f'Published matte for {pose} is rejected by {review_path}; rerun matte with '
                           '--supersede-rejected to archive the pose and matte it again')
    previous = None
    if published.exists():
        previous = load_json(published)
        if not master.is_file() or sha256_file(master) != previous.get('masterSha256'):
            raise RuntimeError(f'Matte changed or incomplete for {pose}; preserve existing files and inspect attempts')
        if previous.get('inputs') == key:
            print(f'{pose}: matte exact-hash resume {master}')
            return
        old = previous.get('inputs', {})
        # The only permitted supersession is a previously completed raw ISNet
        # matte refined from exactly the same source/model/runtime/crop.
        comparable = dict(key, refine=False, environment={k: v for k, v in env.items() if k != 'pymatting'})
        old_attempt = run.path(previous.get('attempt', '')) if previous.get('attempt') else None
        if (not args.supersede_unreviewed or previous.get('status') != 'pending_visual_review'
                or not args.refine or old != comparable or old_attempt is None or not old_attempt.is_dir()
                or not (old_attempt / 'master.png').is_file()
                or sha256_file(old_attempt / 'master.png') != previous['masterSha256']):
            raise RuntimeError(f'Matte settings differ for {pose}; --supersede-unreviewed permits only '
                               'exact-source unreviewed ISNet-to-refined transition; a reviewed rejection of '
                               f'the published matte (inputs/{MATTE_REVIEW}) permits --supersede-rejected')
        if downstream_exists(run, base, pose):
            raise RuntimeError(f'Downstream native/geometry/rig work exists for {pose}; cannot supersede')
    elif master.exists():
        raise RuntimeError(f'Orphaned master for {pose}; never overwrite')
    folder = new_attempt(base / 'raw' / 'matting')
    exclusive(folder / 'inputs.json', encode_json(key))
    if previous:
        copy_exact(master, folder / 'superseded-master.png')
        copy_exact(published, folder / 'superseded-matting.json')
    try:
        options = ort.SessionOptions()
        options.intra_op_num_threads = 4
        options.inter_op_num_threads = 1
        session = ort.InferenceSession(str(model), sess_options=options, providers=['CPUExecutionProvider'])
        if session.get_providers() != ['CPUExecutionProvider']:
            raise RuntimeError(f'Unexpected ONNX providers: {session.get_providers()}')
        entry = session.get_inputs()[0]
        shape = entry.shape
        if shape[1] != 3 or not isinstance(shape[2], int) or shape[2] != shape[3]:
            raise RuntimeError(f'Unsupported model input shape: {shape}')
        original = Image.open(source.path).convert('RGB')
        if original.size != (canvas_size, canvas_size):
            raise RuntimeError(f'Expected unwarped {canvas_size}-square source, got {original.size}')
        original.save(folder / 'original-rgb.png')
        side = shape[2]
        input_image = np.asarray(original.resize((side, side), Image.Resampling.BILINEAR), np.float32) / 255
        output = session.run(None, {entry.name: input_image.transpose(2, 0, 1)[None]})[0]
        coarse = np.clip(output[0, 0], 0, 1)
        coarse = np.asarray(Image.fromarray(coarse).resize(original.size, Image.Resampling.BILINEAR))
        raw_alpha = np.uint8(np.rint(coarse * 255))
        Image.fromarray(raw_alpha).save(folder / 'alpha-model.png')
        if args.refine:
            from pymatting import estimate_alpha_cf, estimate_foreground_ml
            rgb = np.asarray(original)
            foreground = binary_erosion(coarse > .98, iterations=2)
            background = ~binary_dilation(coarse > .005, iterations=2)
            trimap = np.full(coarse.shape, .5)
            trimap[foreground] = 1
            trimap[background] = 0
            Image.fromarray(np.uint8(trimap * 255)).save(folder / 'trimap.png')
            refined = np.clip(estimate_alpha_cf(rgb.astype(np.float64) / 255, trimap,
                                                laplacian_kwargs={'epsilon': 1e-7},
                                                cg_kwargs={'maxiter': 2000}), 0, 1)
            paint = np.uint8(np.rint(np.clip(estimate_foreground_ml(rgb.astype(np.float64) / 255, refined), 0, 1) * 255))
            paint[foreground] = rgb[foreground]
            alpha = np.uint8(np.rint(refined * 255))
            Image.fromarray(alpha).save(folder / 'alpha-refined.png')
        else:
            paint = np.asarray(original).copy()
            alpha = raw_alpha
        pockets, suppression = suppress_background_pockets(np.asarray(original), alpha)
        alpha = np.where(pockets, np.uint8(0), alpha)
        Image.fromarray(np.uint8(pockets) * 255).save(folder / 'pocket-cleared.png')
        suppression['mask'] = 'pocket-cleared.png'
        suppression['maskSha256'] = sha256_file(folder / 'pocket-cleared.png')
        if not np.any(alpha > 0) or not np.any(alpha == 0):
            raise RuntimeError('Matte lacks foreground or transparent background; visual review required')
        paint[alpha == 0] = 0
        rgba = Image.fromarray(np.dstack((paint, alpha)), 'RGBA')
        rgba.save(folder / 'master.png')
        for name, color in (('white', '#ffffff'), ('dark', '#252a33')):
            backdrop = Image.new('RGBA', rgba.size, color)
            backdrop.alpha_composite(rgba)
            backdrop.convert('RGB').save(folder / f'review-{name}.png')
        ys, xs = np.nonzero(alpha > 0)
        margin = args.margin
        crop = [max(0, int(xs.min()) - margin), max(0, int(ys.min()) - margin),
                min(canvas_size, int(xs.max()) + 1 + margin), min(canvas_size, int(ys.max()) + 1 + margin)]
        result = {'inputs': key, 'attempt': shown(run, folder),
                  'modelAlphaSha256': sha256_file(folder / 'alpha-model.png'),
                  'masterSha256': sha256_file(folder / 'master.png'), 'sourceCanvasCrop': crop,
                  'canvas': [canvas_size, canvas_size], 'providers': session.get_providers(),
                  'transparentPixels': int(np.count_nonzero(alpha == 0)),
                  'backgroundPocketSuppression': suppression, 'status': 'pending_visual_review'}
        exclusive(folder / 'matting.json', encode_json(result))
        if previous:
            temporary_master = master.with_name('master-' + uuid.uuid4().hex + '.pending')
            temporary_record = published.with_name('matting-' + uuid.uuid4().hex + '.pending')
            copy_exact(folder / 'master.png', temporary_master)
            exclusive(temporary_record, encode_json(result))
            os.replace(temporary_master, master)
            os.replace(temporary_record, published)
        else:
            copy_exact(folder / 'master.png', master)
            exclusive(published, encode_json(result))
        print(f'{pose}: matted {master}; inspect {folder}/review-white.png and review-dark.png')
    except Exception as exc:
        record_failure(folder, exc)
        raise


def endpoint(value: str) -> str:
    return value.rstrip('/')


def request(base: str, path: str, payload: Any = None, timeout: float = 30, form: Optional[bytes] = None,
            headers: Optional[dict[str, str]] = None) -> bytes:
    url = base + path
    data = form if form is not None else (json.dumps(payload).encode() if payload is not None else None)
    sent = dict(headers or {})
    if payload is not None:
        sent['Content-Type'] = 'application/json'
    req = urllib.request.Request(url, data=data, headers=sent, method='POST' if data is not None else 'GET')
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return response.read()
    except urllib.error.HTTPError as exc:
        raise RuntimeError(f'{url}: HTTP {exc.code} {exc.read()[:1000]!r}') from exc


def queue_idle(base: str) -> None:
    queue = json.loads(request(base, '/queue', timeout=15))
    if queue.get('queue_running') or queue.get('queue_pending'):
        raise RuntimeError(f'{base} has running or pending work; will not interrupt it')


def node_executable(run: Run) -> str:
    if run.data['tools'].get('node'):
        node = run.tool('node')
        if not node.is_file():
            raise RuntimeError(f'tools.node is not a file: {node}')
        return str(node)
    found = shutil.which('node')
    if not found:
        raise RuntimeError('Node.js not found on PATH; set tools.node in run.json')
    return found


def vram_gib(run: Run) -> float:
    value = (run.data.get('seeThrough') or {}).get('vramGiB', DEFAULT_VRAM_GIB)
    if isinstance(value, bool) or not isinstance(value, (int, float)) or value <= 0:
        raise RunConfigError('run.json seeThrough.vramGiB must be a positive number')
    return value


def build_graph(run: Run, workflow: Path, profile: Path, seed: int, prefix: str, vram: float) -> dict[str, Any]:
    js = """const {makeSeeThroughPrompt} = await import(process.env.WORKFLOW);
const {seethroughProfile} = await import(process.env.PROFILE);
console.log(JSON.stringify(makeSeeThroughPrompt({name:'INPUT'}, Number(process.env.SEED), process.env.PREFIX,
  seethroughProfile(Number(process.env.RESOLUTION), Number(process.env.VRAM)))));"""
    proc = subprocess.run([node_executable(run), '--input-type=module', '-e', js], capture_output=True, text=True,
                          env=dict(os.environ, WORKFLOW=workflow.as_uri(), PROFILE=profile.as_uri(),
                                   SEED=str(seed), PREFIX=prefix, RESOLUTION=str(run.canvas), VRAM=str(vram)),
                          timeout=20, check=True)
    graph = json.loads(proc.stdout)
    graph['4']['inputs']['num_inference_steps'] = SEETHROUGH_STEPS
    graph['9'] = {'class_type': 'JoinImageWithAlpha', 'inputs': {'image': ['1', 0], 'alpha': ['1', 1]}}
    graph['4']['inputs']['image'] = ['9', 0]
    return graph


def upload(host: str, path: Path, name: str) -> dict[str, Any]:
    boundary = '----herdr-' + uuid.uuid4().hex
    body = (f'--{boundary}\r\nContent-Disposition: form-data; name="image"; filename="{name}"\r\n'
            'Content-Type: image/png\r\n\r\n').encode() + path.read_bytes() + (
            f'\r\n--{boundary}\r\nContent-Disposition: form-data; name="overwrite"\r\n\r\nfalse\r\n'
            f'--{boundary}--\r\n').encode()
    return json.loads(request(host, '/upload/image', form=body, timeout=120,
                              headers={'Content-Type': 'multipart/form-data; boundary=' + boundary}))


def remote_file(host: str, info: Any, destination: Path) -> None:
    if not isinstance(info, dict) or not isinstance(info.get('filename'), str):
        raise RuntimeError(f'Invalid ComfyUI output descriptor: {info!r}')
    filename = info['filename']
    if Path(filename).name != filename or filename in ('', '.', '..') or '/' in filename or '\\' in filename:
        raise RuntimeError(f'Unsafe output name: {filename}')
    params = urllib.parse.urlencode({'filename': filename, 'type': info.get('type', 'output'),
                                     'subfolder': info.get('subfolder', '')})
    exclusive(destination, request(host, '/view?' + params, timeout=120))


def history_failed(entry: dict[str, Any]) -> bool:
    status = entry.get('status', {})
    return status.get('status_str') == 'error' or any(
        message[0] == 'execution_error' for message in status.get('messages', []))


def recovered_history(host: str, folder: Path, prompt_id: str) -> dict[str, Any]:
    """Load (or fetch once, without submitting anything) the terminal history of a prior attempt."""
    stored = folder / 'history.json'
    if stored.exists():
        return load_json(stored)
    history = json.loads(request(host, '/history/' + urllib.parse.quote(prompt_id), timeout=30))
    entry = history.get(prompt_id)
    if not entry or not (entry.get('status', {}).get('completed') or history_failed(entry)):
        raise RuntimeError(f'{prompt_id} is not terminal on {host}; wait for an idle queue before recovering')
    exclusive(stored, encode_json(entry))
    return entry


def recovery_folder(attempts: Path, value: str) -> Path:
    candidate = Path(value).expanduser()
    if not candidate.is_absolute() and len(candidate.parts) == 1:
        candidate = attempts / value
    folder = candidate.resolve()
    if folder.parent != attempts.resolve() or not folder.is_dir():
        raise RuntimeError(f'Recovery must name an existing attempt under {attempts}')
    return folder


def decompose(run: Run, args: argparse.Namespace, pose: str, source: Source) -> None:
    host = endpoint(run.endpoints['seeThrough'])
    qwen = endpoint(run.endpoints['qwen'])
    scripts = run.tool('runtime') / 'scripts'
    workflow = scripts / 'seethrough-workflow.mjs'
    profile = scripts / 'seethrough-profile.mjs'
    for script in (workflow, profile):
        if not script.is_file():
            raise RuntimeError(f'See-through runtime script missing: {script}')
    base = run.pose_dir(pose)
    matting = base / 'inputs' / 'matting.json'
    master = base / 'inputs' / 'master.png'
    if not matting.exists() or not master.exists():
        raise RuntimeError(f'Matte {pose} before decomposing')
    matte_record = load_json(matting)
    master_sha = sha256_file(master)
    if master_sha != matte_record['masterSha256'] or matte_record['inputs']['sourceSha256'] != source.sha256:
        raise RuntimeError(f'Source or matte changed for {pose}')
    vram = vram_gib(run)
    key = {'sourceSha256': master_sha, 'source': shown(run, master), 'rawSource': shown(run, source.path),
           'rawSourceSha256': source.sha256, 'sourceProvenance': shown(run, source.provenance),
           'sourceProvenanceSha256': sha256_file(source.provenance), 'mattingSha256': sha256_file(matting),
           'workflowSha256': sha256_file(workflow), 'profileSha256': sha256_file(profile),
           'endpoint': host, 'resolution': run.canvas, 'steps': SEETHROUGH_STEPS, 'vramGiB': vram,
           'seed': run.seeds['seeThrough'] + run.poses.index(pose)}
    native = base / 'native'
    record = native / 'generation.json'
    if record.exists():
        previous = load_json(record)
        if previous.get('inputs') != key or any(not (native / name).is_file() or sha256_file(native / name) != value
                                                for name, value in previous['files'].items()):
            raise RuntimeError(f'Native output differs for {pose}; never overwrite')
        print(f'{pose}: decomposition exact-hash resume {native}')
        return
    if native.exists() and any(native.iterdir()):
        raise RuntimeError(f'Incomplete native output for {pose}; inspect without overwriting')
    attempts = base / 'raw' / 'decomposition'
    recovery = args.recover_attempt
    if recovery:
        folder = recovery_folder(attempts, recovery)
        if not (folder / 'inputs.json').is_file() or load_json(folder / 'inputs.json') != key:
            raise RuntimeError('Recovery must use this pose exact-source decomposition attempt')
    else:
        folder = new_attempt(attempts)
        exclusive(folder / 'inputs.json', encode_json(key))
    try:
        if recovery:
            prompt = load_json(folder / 'prompt.json')
            prompt_id = load_json(folder / 'queued.json')['prompt_id']
            entry = recovered_history(host, folder, prompt_id)
            if (entry['prompt'][1] != prompt_id or entry['prompt'][2] != prompt
                    or sha256_file(folder / 'uploaded.png') != key['sourceSha256']):
                raise RuntimeError('Recovered history, graph or uploaded source differs')
        else:
            # Shared unified memory: require the Qwen server idle and release its
            # models, then require See-through idle. Never cancel other jobs.
            queue_idle(qwen)
            request(qwen, '/free', {'unload_models': True, 'free_memory': True}, timeout=60)
            queue_idle(host)
            definitions = json.loads(request(host, '/object_info', timeout=30))
            for node in REQUIRED_NODES:
                if node not in definitions:
                    raise RuntimeError(f'Missing ComfyUI node {node}')
            join_inputs = definitions['JoinImageWithAlpha']['input']['required']
            if 'image' not in join_inputs or 'alpha' not in join_inputs:
                raise RuntimeError('JoinImageWithAlpha inputs changed')
            label = slug(run.character['id'])
            uploaded = upload(host, master, f'{label}-{pose}-{uuid.uuid4().hex}.png')
            copy_exact(master, folder / 'uploaded.png')
            exclusive(folder / 'upload.json', encode_json({'response': uploaded, 'uploadedSha256': master_sha,
                                                           'uploadedBytes': master.stat().st_size}))
            prefix = f'{label}-{pose}-{uuid.uuid4().hex[:10]}'
            prompt = build_graph(run, workflow, profile, key['seed'], prefix, vram)
            prompt['1']['inputs']['image'] = (
                (uploaded['subfolder'] + '/' if uploaded.get('subfolder') else '') + uploaded['name'])
            exclusive(folder / 'prompt.json', encode_json(prompt))
            queued = json.loads(request(host, '/prompt', {'client_id': f'{label}-{uuid.uuid4().hex}',
                                                          'prompt': prompt}, timeout=60))
            if not queued.get('prompt_id'):
                raise RuntimeError(f'ComfyUI rejected graph: {queued}')
            prompt_id = queued['prompt_id']
            exclusive(folder / 'queued.json', encode_json(queued))
            deadline = time.monotonic() + 30 * 60
            entry = None
            while time.monotonic() < deadline:
                history = json.loads(request(host, '/history/' + urllib.parse.quote(prompt_id), timeout=30))
                entry = history.get(prompt_id)
                if entry:
                    if not (folder / 'history-latest.json').exists():
                        exclusive(folder / 'history-latest.json', encode_json(entry))
                    if history_failed(entry):
                        exclusive(folder / 'history.json', encode_json(entry))
                        raise RuntimeError(f'See-through execution failed for {prompt_id}')
                    if entry.get('status', {}).get('completed'):
                        exclusive(folder / 'history.json', encode_json(entry))
                        break
                time.sleep(3)
            else:
                raise RuntimeError(f'See-through timed out for {prompt_id}; job may still be running. Do not retry; '
                                   f'once the queue is idle run decompose --poses {pose} --recover-attempt {folder.name}')
        if not entry.get('status', {}).get('completed') or entry.get('status', {}).get('status_str') != 'success':
            raise RuntimeError('Only terminal-successful decomposition can be downloaded')
        info = entry.get('outputs', {}).get('7', {}).get('text', [])
        if not info:
            # The installed SavePSD node writes the filename to its output log,
            # not ComfyUI history UI text. Bind that filename to this exact graph.
            info = [request(host, '/view?filename=seethrough_psd_info.log&type=output', timeout=30).decode('utf-8').strip()]
        if len(info) != 1 or not info[0]:
            raise RuntimeError(f'See-through missing exact metadata output: {info}')
        metadata_name = Path(info[0].replace('\\', '/')).name
        remote_file(host, {'filename': metadata_name}, folder / 'layers.json')
        metadata = load_json(folder / 'layers.json')
        if metadata.get('prefix') != prompt['7']['inputs']['filename_prefix']:
            raise RuntimeError('See-through metadata belongs to another generation; preserve this attempt')
        if metadata.get('width') != run.canvas or metadata.get('height') != run.canvas or not metadata.get('layers'):
            raise RuntimeError('See-through metadata has missing layers or wrong canvas')
        files = {'layers.json': sha256_file(folder / 'layers.json')}
        for layer in metadata['layers']:
            for field in ('filename', 'depth_filename'):
                if field in layer:
                    name = layer[field]
                    remote_file(host, {'filename': name}, folder / name)
                    files[name] = sha256_file(folder / name)
        preview = entry.get('outputs', {}).get('8', {}).get('images', [])
        if preview:
            remote_file(host, preview[0], folder / 'preview.png')
        tags = {layer['name'] for layer in metadata['layers']}
        missing = [category for category, choices in REQUIRED_PARTS.items() if not tags.intersection(choices)]
        if 'eyes' not in tags and not ({'eyel', 'eyer'} <= tags or {'irides-l', 'irides-r'} <= tags):
            if 'eyes' not in missing:
                missing.append('second-eye')
        summary = {'inputs': key, 'promptId': prompt_id, 'metadataSha256': files['layers.json'],
                   'tags': sorted(tags), 'missingRequiredParts': missing, 'files': files,
                   'optionalAbsent': sorted(OPTIONAL_PARTS - tags),
                   'status': 'requires_source_geometry_review' if not missing else 'missing_required_parts'}
        summary_path = folder / 'run-summary.json'
        if recovery and summary_path.exists() and summary_path.read_bytes() != encode_json(summary):
            # Keep the earlier evaluation (e.g. under stricter part rules) as evidence.
            summary_path = folder / f'run-summary-recovered-{uuid.uuid4().hex}.json'
        exclusive(summary_path, encode_json(summary))
        if missing:
            raise RuntimeError(f'{pose} missing required semantic parts {missing}; raw results retained at {folder}')
        native.mkdir(parents=True, exist_ok=True)
        layers_name = f'{pose}_layers.json'
        for filename in files:
            copy_exact(folder / filename, native / (layers_name if filename == 'layers.json' else filename))
        generation = dict(key)
        generation.update({'inputs': key, 'sourceSha256': key['sourceSha256'], 'attempt': shown(run, folder),
                           'promptId': prompt_id,
                           'files': {(layers_name if name == 'layers.json' else name): sha
                                     for name, sha in files.items()},
                           'tags': sorted(tags), 'optionalAbsent': summary['optionalAbsent'],
                           'runSummary': shown(run, summary_path),
                           'status': 'requires_source_geometry_review'})
        exclusive(record, encode_json(generation))
        print(f'{pose}: native decomposition {native}; inspect {folder}/preview.png')
    except Exception as exc:
        record_failure(folder, exc)
        raise


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', choices=('matte', 'decompose'))
    parser.add_argument('--run', required=True, help='run directory containing run.json and source-index.json')
    parser.add_argument('--poses', default='all', help='all or comma-separated pose IDs from run.json')
    parser.add_argument('--recover-attempt',
                        help='decompose only: download this exact terminal attempt (folder name or path) '
                             'without new inference')
    refine_group = parser.add_mutually_exclusive_group()
    refine_group.add_argument('--refine', action='store_true', dest='refine', default=True,
                              help='Default: installed PyMatting closed-form fringe refinement')
    refine_group.add_argument('--no-refine', action='store_false', dest='refine',
                              help='Keep raw ISNet alpha (diagnostic candidate only)')
    supersede_group = parser.add_mutually_exclusive_group()
    supersede_group.add_argument('--supersede-unreviewed', action='store_true',
                                 help='Replace an unreviewed raw ISNet master with a refined candidate, '
                                      'retaining the first master and record in the new attempt')
    supersede_group.add_argument('--supersede-rejected', action='store_true',
                                 help=f'matte only: archive poses whose inputs/{MATTE_REVIEW} rejects the published '
                                      f'master (whole poses/<pose>/ and native-models/<pose>/ renamed into '
                                      f'{SUPERSEDED_DIR}/<pose>/<n>-<masterSha[:16]>/), then matte them again')
    parser.add_argument('--margin', type=int, default=24,
                        help='source-space crop annotation margin; artwork stays on the original square canvas')
    args = parser.parse_args()
    if args.margin < 0 or args.margin > 256:
        parser.error('Invalid margin')
    try:
        run = load_run(args.run)
        selected = run.select(args.poses)
    except RunConfigError as exc:
        parser.error(str(exc))
    if args.recover_attempt and (args.stage != 'decompose' or len(selected) != 1):
        parser.error('--recover-attempt requires decompose and exactly one selected pose')
    if args.supersede_rejected and args.stage != 'matte':
        parser.error('--supersede-rejected applies to matte only')
    sources = load_sources(run, selected)
    stage = matte if args.stage == 'matte' else decompose
    for pose in selected:
        if args.supersede_rejected:
            supersede_rejected_matte(run, pose)
        stage(run, args, pose, sources[pose])


if __name__ == '__main__':
    main()
