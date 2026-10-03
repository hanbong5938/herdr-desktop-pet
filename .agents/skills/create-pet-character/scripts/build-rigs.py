#!/usr/bin/env python3
"""Pose-local geometry review and independent source-rig/native conversion.

``geometry`` measures each pose only from this run's own matted master and its
See-through rasters and writes reviewable ``plan/geometry.json`` +
``plan/expression-plan.json`` + masks (never overwritten). ``build`` runs the
upstream DAEMONLET ``build-source-models.py`` on the reviewed geometry and the
local Qwen expression sheet with the fork fixes from ``expression_fixes``
patched in, exports the round PSD through the restricted upstream exporter,
converts it with this repository's ``adapt-rig-source.py`` and retargets the
established motion profile to the own measured hands.

No installed/default artwork is used as a source of paint or placement.
"""
from __future__ import annotations

import argparse
import contextlib
import copy
import importlib.util
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_config import load_json, load_run, sha256_file, write_new, write_new_json  # noqa: E402
import expression_fixes  # noqa: E402

FIXES_MODULE = Path(expression_fixes.__file__).resolve()
REQUIRED_EXPORTED = {'face', 'front hair', 'back hair', 'mouth_close', 'eyewhite-l', 'eyewhite-r', 'irides-l', 'irides-r'}


def environment(run):
    runtime = run.tool('runtime').resolve()
    scripts = runtime / 'scripts/characters'
    if not (scripts / 'build-source-models.py').is_file():
        raise ValueError(f'tools.runtime does not contain scripts/characters/build-source-models.py: {runtime}')
    for path in (scripts, scripts / 'lib'):
        if str(path) not in sys.path:
            sys.path.insert(0, str(path))
    return SimpleNamespace(
        run=run, root=run.root, runtime=runtime, scripts=scripts, adapter=run.tool('adapter').resolve(),
        motion_reference=run.tool('motionReference').resolve(), size=run.canvas,
        labels={pose: pose.replace('-', ' ').title() for pose in run.poses})


def node_executable(run):
    """run.json ``tools.node``, else ``node`` on PATH (same rule as prepare-sources.py)."""
    if run.data['tools'].get('node'):
        node = run.tool('node')
        if not node.is_file():
            raise RuntimeError(f'tools.node is not a file: {node}')
        return str(node)
    found = shutil.which('node')
    if not found:
        raise RuntimeError('Node.js not found on PATH; set tools.node in run.json')
    return found


def node_path(node):
    """PATH with the resolved node's directory first, for exporters that run a bare ``node``."""
    current = os.environ.get('PATH', '')
    return os.pathsep.join([str(Path(node).parent)] + ([current] if current else []))


@contextlib.contextmanager
def node_on_path(node):
    """Make a bare ``node`` (the repository adapter's exporter) resolve to ``node`` while active."""
    previous = os.environ.get('PATH')
    os.environ['PATH'] = node_path(node)
    try:
        yield
    finally:
        if previous is None:
            os.environ.pop('PATH', None)
        else:
            os.environ['PATH'] = previous


def image_new(path, array):
    buffer = io.BytesIO()
    Image.fromarray(array.astype(np.uint8)).save(buffer, format='PNG')
    write_new(path, buffer.getvalue())


def box(mask):
    yy, xx = np.where(mask)
    if not len(xx):
        raise ValueError('Required source part/mask has no pixels')
    return [int(xx.min()), int(yy.min()), int(xx.max() + 1), int(yy.max() + 1)]


def contained(a, b, slack=0):
    return a[0] >= b[0] - slack and a[1] >= b[1] - slack and a[2] <= b[2] + slack and a[3] <= b[3] + slack


def expand(size, b, x, y=None):
    y = x if y is None else y
    return [max(0, b[0] - x), max(0, b[1] - y), min(size, b[2] + x), min(size, b[3] + y)]


def mask_box(size, b):
    m = np.zeros((size, size), bool)
    m[b[1]:b[3], b[0]:b[2]] = True
    return m


def source_layers(env, pose):
    root = env.run.pose_dir(pose)
    master = root / 'inputs/master.png'
    generation = load_json(root / 'native/generation.json')
    if generation['sourceSha256'] != sha256_file(master):
        raise ValueError(f'{pose}: native decomposition does not match own master SHA')
    with Image.open(master) as im:
        if im.size != (env.size, env.size) or im.mode != 'RGBA':
            raise ValueError(f'{pose}: master must be {env.size} RGBA')
        source = np.array(im)
    metas = list((root / 'native').glob('*_layers.json'))
    if len(metas) != 1:
        raise ValueError(f'{pose}: expected one own SeeThrough layer manifest, found {len(metas)}')
    meta = load_json(metas[0])
    if len(meta['layers']) < 10:
        raise ValueError(f'{pose}: incomplete semantic decomposition')
    from build_base_pose_a import load_layers
    raw, _, files = load_layers(root / 'native', meta, pose_local_sides=True)
    return root, master, source, raw, files


def part(raw, name, pose, min_pixels=5):
    if name not in raw or int((raw[name][:, :, 3] > 20).sum()) < min_pixels:
        raise ValueError(f'{pose}: missing own nonempty {name}; correct SeeThrough decomposition')
    return raw[name][:, :, 3]


def pigments(source, white, iris, lash, aperture):
    """Check actual paint, never exclude green irides with a warm-skin RGB rule."""
    rgb = source[:, :, :3].astype(np.int16)
    alpha = source[:, :, 3]
    visible = (alpha > 24) & aperture
    if (visible & iris).sum() < 8:
        raise ValueError('Iris alpha does not intersect the visible source eye aperture')
    if np.ptp(rgb[visible].astype(float), axis=0).max() < 12:
        raise ValueError('Eye aperture has no inspected source pigmentation contrast')
    dark = (rgb.max(2) < 160) | ((rgb[:, :, 0] - rgb[:, :, 1] > 22) & (rgb[:, :, 1] < 175))
    if not (lash & dark & (alpha > 24)).any():
        raise ValueError('Own eyelash contour does not align with dark source pixels')
    if not (white & visible).any():
        raise ValueError('Own eye-white does not overlap painted source')


def mouth_mask(size, source, face, mouth, eye_bounds):
    mb = box(mouth > 16)
    if not contained(mb, box(face > 16), 12):
        raise ValueError('Neutral mouth lies outside own face; correct decomposition')
    if mb[1] <= max(b[3] for b in eye_bounds):
        raise ValueError('Neutral mouth collides with eyes; correct mouth decomposition')
    roi = expand(size, mb, min(5, max(2, (mb[2] - mb[0]) // 5)), 3)
    candidate = (mouth > 16) & mask_box(size, roi) & (source[:, :, 3] > 24)
    # Strict source local ownership; never inflate to a nearby jaw or hair edge.
    if int(candidate.sum()) < 4 or roi[3] - roi[1] > 34:
        raise ValueError('Neutral mouth is ambiguous or contains jaw/hair; correct native mouth part')
    return candidate, roi


def geometry(env, pose):
    size = env.size
    d, master, source, raw, files = source_layers(env, pose)
    master_sha = sha256_file(master)
    plan = d / 'plan'
    if (plan / 'geometry.json').exists():
        old = load_json(plan / 'geometry.json')
        if old.get('sourceSha256') == master_sha:
            raise FileExistsError(f'{pose}: reviewed geometry exists; do not overwrite it')
        raise ValueError(f'{pose}: geometry belongs to different source SHA')
    from build_own_pose_models import canon
    face = part(raw, 'face', pose, 100)
    front = part(raw, 'front hair', pose, 30)
    back = part(raw, 'back hair', pose, 30)
    mouth = part(raw, 'mouth_close', pose, 4)
    neck = part(raw, 'neck', pose, 20)
    top = part(raw, 'topwear', pose, 30)
    lower = [name for name in ('bottomwear', 'legwear', 'body') if name in raw and (raw[name][:, :, 3] > 20).sum() > 30]
    if not lower:
        raise ValueError(f'{pose}: own lower body/shorts/legs unavailable; correct decomposition')
    body_parts = ['neck', 'topwear', *lower]
    body_alpha = (neck > 20) | (top > 20)
    for name in lower:
        body_alpha |= raw[name][:, :, 3] > 20
    fb, bb = box(face > 20), box(body_alpha)
    nose = part(raw, 'nose', pose, 4)
    nose_bounds = box(nose > 20)
    if (not contained(nose_bounds, fb, 8) or nose_bounds[2] - nose_bounds[0] > (fb[2] - fb[0]) * .45
            or nose_bounds[3] - nose_bounds[1] > (fb[3] - fb[1]) * .4):
        raise ValueError(f'{pose}: nose includes unrelated source skin/hair; correct native nose mask before expression registration')
    eyes = {}
    irises = {}
    eye_bounds = []
    measurements = {}
    for side in ('l', 'r'):
        w = part(raw, 'eyewhite-' + side, pose, 5)
        i = part(raw, 'irides-' + side, pose, 5)
        lash = part(raw, 'eyelash-' + side, pose, 5)
        # SeeThrough alpha is the source-local aperture. Keep all visible iris
        # alpha, including green and dark rim; do not fit an ellipse.
        visible = source[:, :, 3] > 24
        aperture = ((w > 24) | (i > 24)) & visible
        ib = (i > 24) & aperture
        if aperture.sum() < 12 or ib.sum() < 8:
            raise ValueError(f'{pose}: {side} aperture/iris too sparse; inspect own part')
        pigments(source, w > 24, i > 24, lash > 24, aperture)
        b = box(aperture)
        if not contained(b, fb, 24):
            raise ValueError(f'{pose}: {side} eye is outside own face')
        eyes[side] = aperture
        irises[side] = ib
        eye_bounds.append(b)
        measurements[side] = {'apertureBounds': b, 'aperturePixels': int(aperture.sum()), 'irisBounds': box(ib),
                              'irisPixels': int(ib.sum()), 'lashBounds': box(lash > 24)}
    if measurements['l']['apertureBounds'][0] >= measurements['r']['apertureBounds'][0]:
        raise ValueError(f'{pose}: image-left eyeL and image-right eyeR ambiguous; correct side labels')
    if eye_bounds[0][2] >= eye_bounds[1][0]:
        raise ValueError(f'{pose}: eye contours overlap; correct decomposition')
    mm, mb = mouth_mask(size, source, face, mouth, eye_bounds)
    head = expand(size, fb, max(12, (fb[2] - fb[0]) // 3), max(12, (fb[3] - fb[1]) // 3))
    # Body often includes legs: constrain the interaction rectangle to actual
    # upper-body garment, not the full painted lower body.
    topb = box(top > 24)
    torso = expand(size, topb, 14)
    for name, b in [('head', head), ('torso', torso)]:
        if not ((source[:, :, 3] > 24) & mask_box(size, b)).any():
            raise ValueError(f'{pose}: interaction {name} has no source paint')
    cx = (fb[0] + fb[2]) / 2
    cy = (fb[1] + fb[3]) / 2
    head_radius = max(fb[2] - fb[0], fb[3] - fb[1]) * .65
    follow = {}
    for name in ('face', 'front hair', 'back hair', 'nose', 'headwear', 'earwear', 'ears-l', 'ears-r'):
        if name in raw and (raw[name][:, :, 3] > 20).sum() > 4:
            # Runtime names are decoded (canonical) names.
            follow[canon(name)] = {'center': {'cx': cx, 'cy': cy}, 'radius': head_radius, 'falloffRadius': head_radius * 1.6}
    front_hands = []
    hair = (front > 24) | (back > 24)
    for hand in ('handwear-l', 'handwear-r'):
        if hand in raw and ((raw[hand][:, :, 3] > 24) & hair).sum() > 5:
            front_hands.append(hand)
            follow[canon(hand)] = {'center': {'cx': cx, 'cy': cy}, 'radius': head_radius, 'falloffRadius': head_radius * 1.6}
    if not any(n.startswith('handwear-') for n in raw):
        raise ValueError(f'{pose}: own hand layers missing; correct decomposition')
    fy, fx = np.where(face > 20)
    nearest = np.argmin((fx - cx) ** 2 + (fy - cy) ** 2)
    face_point = [int(fx[nearest]), int(fy[nearest])]
    masks = {('aperture', s): f'plan/masks/own-aperture-{s}.png' for s in ('l', 'r')}
    masks.update({('iris', s): f'plan/masks/own-iris-{s}.png' for s in ('l', 'r')})
    facial = expand(size, fb, max(18, (fb[2] - fb[0]) // 4), max(18, (fb[3] - fb[1]) // 5))
    # Expression sheet must contain own ears/eyebrows/face and lip registration,
    # not the torso or another pose's source geometry.
    crop = [max(0, facial[0] - 10), max(0, facial[1] - 10), min(size, facial[2] + 10), min(size, max(facial[3], mb[3]) + 12)]
    if crop[3] >= topb[3] or not contained(mb, crop):
        raise ValueError(f'{pose}: cannot isolate own face crop from body; inspect artwork')
    for side in ('l', 'r'):
        image_new(d / masks['aperture', side], eyes[side] * 255)
        image_new(d / masks['iris', side], irises[side] * 255)
    image_new(d / 'plan/masks/own-neutral-mouth.png', mm * 255)
    data = {'sourceSha256': master_sha, 'nativeSourceFiles': files,
            'sourceEyeApertures': {s: measurements[s]['apertureBounds'] for s in ('l', 'r')},
            'sourceEyeMasks': {s: masks['aperture', s] for s in ('l', 'r')},
            'sourceIrisMasks': {s: masks['iris', s] for s in ('l', 'r')},
            'neutralMouthBounds': mb, 'neutralMouthMask': 'plan/masks/own-neutral-mouth.png',
            'interactionAreas': {'head': dict(zip(('x0', 'y0', 'x1', 'y1'), head)),
                                 'torso': dict(zip(('x0', 'y0', 'x1', 'y1'), torso))},
            'headFollowLayers': follow, 'frontHands': front_hands, 'faceComponentPoint': face_point,
            'measurements': {'face': fb, 'body': bb, 'bodyParts': body_parts, 'topwear': topb,
                             'frontHair': box(front > 24), 'backHair': box(back > 24), 'mouth': box(mm), 'eyes': measurements,
                             'parts': {name: box(ar[:, :, 3] > 20) for name, ar in raw.items() if (ar[:, :, 3] > 20).any()}}}
    write_new_json(plan / 'geometry.json', data)
    write_new_json(plan / 'expression-plan.json', {'source_canvas_crop': crop, 'source_master_sha256': master_sha})
    return {'pose': pose, 'geometry': str(plan / 'geometry.json'), 'sourceSha256': master_sha, 'expressionCrop': crop}


def own_neutral(builder):
    """Replace historical restrictive RGB eye filter with reviewed own contours."""
    old = builder.neutral_features

    def neutral(raw, source, geo):
        # The original routine already consumes reviewed aperture and iris masks
        # before its legacy RGB fallback; require them rather than permitting fallback.
        for side in ('l', 'r'):
            if side not in geo.get('_sourceEyeMasks', {}) or side not in geo.get('_sourceIrisMasks', {}):
                raise ValueError(f'Missing own reviewed {side} eye masks')
        if geo.get('_neutralMouthMask') is None:
            raise ValueError('Missing own neutral mouth mask')
        return old(raw, source, geo)
    builder.neutral_features = neutral


def export_private_round(env, round_dir, node):
    """Use the real restricted exporter without moving private source models."""
    export_root = env.runtime / 'outputs/characters'
    export_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.own-anime-export-', dir=export_root) as tmp:
        staged = Path(tmp) / 'model'
        shutil.copytree(round_dir, staged)
        subprocess.run([node, str(env.scripts / 'export-model-psd.mjs'), str(staged)], cwd=env.runtime, check=True,
                       env=dict(os.environ, PATH=node_path(node)))
        for name in ('model.psd', 'model-authoring.psd', 'neutral-authored.png', 'PSD-order-audit.json'):
            with (round_dir / name).open('xb') as target, (staged / name).open('rb') as source:
                shutil.copyfileobj(source, target)


def model_motion(env, pose, geometry_data, layer_names):
    """Retain proven phase/reaction timing and global intent, retarget local gestures."""
    from build_own_pose_models import canon
    size = env.size
    reference = load_json(env.motion_reference / (pose + '-motion.json'))
    if (set(reference['phases']) != {'idle', 'running', 'waiting', 'unknown'}
            or set(reference['reactions']) != {'head_tap', 'body_tap', 'pet', 'completion_observed'}):
        raise ValueError(f'{pose}: incomplete source motion profile')
    boxes = geometry_data['measurements']['parts']
    # Do not propagate any source-local transforms: their pivots belong to a
    # different body. Explicit own measured hand centers replace them below.
    out = copy.deepcopy(reference)
    own_hands = [n for n in ('handwear-l', 'handwear-r') if n in boxes]
    if not own_hands:
        raise ValueError(f'{pose}: no independent hand plate for gestures')

    def hand_layer(name, amplitude, duration):
        b = boxes[name]
        cx = (b[0] + b[2]) / 2 / size
        cy = (b[1] + b[3]) / 2 / size
        return {'origin': {'x': cx, 'y': cy}, 'influence': {'axisX': 1, 'axisY': 0, 'start': -2, 'end': -1},
                'translateY': {'interpolation': 'smoothstep', 'keys': [
                    {'at_ms': 0, 'value': 0}, {'at_ms': duration // 2, 'value': -amplitude * .8}, {'at_ms': duration, 'value': 0}]}}
    transform_keys = ('translateX', 'translateY', 'rotationDeg', 'scale')
    face = geometry_data['measurements']['face']
    for group in ('phases', 'reactions'):
        for slot, clip in out[group].items():
            clip['layers'] = {}
            # Global shape/eye/expression intent is dimensionless; downscale
            # head/body amplitudes to avoid overextending the newly measured face.
            for pname, track in clip['tracks'].items():
                if pname in ('angleX', 'angleY', 'angleZ', 'body', 'bust', 'bustY'):
                    for key in track['keys']:
                        key['value'] = round(key['value'] * .65, 6)
            if group == 'reactions' and slot in ('head_tap', 'body_tap', 'pet'):
                chosen = min(own_hands, key=lambda n: abs((boxes[n][0] + boxes[n][2]) / 2 - (face[0] + face[2]) / 2))
                amp = min(12, max(4, (boxes[chosen][3] - boxes[chosen][1]) * .1))
                clip['layers'][canon(chosen)] = hand_layer(chosen, amp, clip['duration_ms'])
            if group == 'phases' and pose in ('writing', 'head-tap', 'head-pet', 'torso-tap') and slot in ('idle', 'waiting'):
                chosen = own_hands[0]
                clip['layers'][canon(chosen)] = hand_layer(chosen, 4, clip['duration_ms'])
            if any(name not in layer_names for name in clip['layers']):
                raise ValueError(f'{pose}/{slot}: motion references absent hand layer')
            keys = (sum(len(t['keys']) for t in clip['tracks'].values())
                    + sum(len(t['keys']) for l in clip['layers'].values() for k, t in l.items() if k in transform_keys))
            tracks = len(clip['tracks']) + sum(sum(k in l for k in transform_keys) for l in clip['layers'].values())
            if tracks > 40 or keys > 256 or any(len(t['keys']) > 32 for t in clip['tracks'].values()):
                raise ValueError(f'{pose}/{slot}: native track/key limit exceeded')
    total = sum(len(t['keys']) for group in ('phases', 'reactions') for c in out[group].values() for t in c['tracks'].values())
    total += sum(len(t['keys']) for group in ('phases', 'reactions') for c in out[group].values()
                 for l in c['layers'].values() for k, t in l.items() if k in transform_keys)
    if total > 1024:
        raise ValueError(f'{pose}: native 1024-key catalog quota exceeded ({total})')
    return out


def build(env, pose, round_name):
    if not round_name or '/' in round_name or round_name.startswith('.'):
        raise ValueError('Round must be a simple non-hidden name')
    d, master, source, raw, files = source_layers(env, pose)
    master_sha = sha256_file(master)
    geo = load_json(d / 'plan/geometry.json')
    expression = load_json(d / 'plan/expression-plan.json')
    if geo['sourceSha256'] != master_sha or expression['source_master_sha256'] != master_sha:
        raise ValueError(f'{pose}: stale geometry or expression plan')
    request = load_json(d / 'plan/own-expression-request.json')
    if request['source_master_sha256'] != master_sha or request['source_canvas_crop'] != expression['source_canvas_crop']:
        raise ValueError(f'{pose}: expression sheet derived from different own master/crop')
    atlas = d / 'raw/own-expression-G0.png'
    if request['expressionSheetSha256'] != sha256_file(atlas):
        raise ValueError(f'{pose}: expression sheet hash mismatch')
    out = d / round_name
    native = env.root / 'native-models' / pose
    if out.exists() or native.exists():
        raise FileExistsError(f'{pose}: round/native destination exists; preserve history')
    node = node_executable(env.run)
    spec = importlib.util.spec_from_file_location('source_builder', env.scripts / 'build-source-models.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    own_neutral(module)
    fixes = expression_fixes.FixContext(geo)
    expression_fixes.install(module, fixes)
    expected_exporter = (env.scripts / 'export-model-psd.mjs').resolve()

    def restricted_export(command, *, cwd, check):
        # Upstream requests a bare ``node``; execute the run's resolved Node instead.
        if (len(command) != 3 or command[0] != 'node' or Path(command[1]).resolve() != expected_exporter
                or Path(command[2]).resolve() != out.resolve() or Path(cwd).resolve() != env.runtime or not check):
            raise ValueError(f'{pose}: unexpected source builder subprocess request')
        export_private_round(env, out, node)
    module.subprocess = SimpleNamespace(run=restricted_export)
    module.build(env.root, {'id': pose, 'label': env.labels[pose]}, round_name)
    # The adapter exports via the installed runtime's real Node PSD exporter.
    # Its default selected_pose guard assumes models are physically below
    # RUNTIME/outputs/characters; our private source is intentionally outside
    # that tree. Narrowly permit ONLY this model's own pose.json here.
    adapter_spec = importlib.util.spec_from_file_location('native_rig_adapter', env.adapter)
    adapter = importlib.util.module_from_spec(adapter_spec)
    adapter_spec.loader.exec_module(adapter)
    original_selected_pose = adapter.selected_pose

    def own_selected_pose(model, upstream_root):
        if model.resolve() != out.resolve() or upstream_root.resolve() != env.runtime:
            return original_selected_pose(model, upstream_root)
        model_file = model / 'model.json'
        config = adapter.load_json(model_file, 'own model')
        if config.get('schemaVersion') != 1 or config.get('pose') != 'pose.json' or config.get('id') != pose:
            raise ValueError(f'{pose}: malformed or foreign own source model')
        pose_file = model / 'pose.json'
        selected = adapter.load_json(pose_file, 'own pose')
        if selected.get('id') != pose:
            raise ValueError(f'{pose}: model/pose identity mismatch')
        return model_file, pose_file, selected
    adapter.selected_pose = own_selected_pose
    # The adapter's run_exporter invokes a bare ``node``; put the resolved one first on PATH.
    with node_on_path(node):
        adapter.adapt(argparse.Namespace(upstream_root=str(env.runtime), model=str(out), output=str(native),
                                         size1024=True, motion_profile=None, raster_only=True))
    exported = load_json(out / 'layers.json')
    present = {r['name'] for r in exported['layers']}
    if len(present) < 12 or not REQUIRED_EXPORTED.issubset(present):
        raise ValueError(f'{pose}: source PSD lacks essential independent semantic parts')
    converted = load_json(native / 'rig-overrides.json')
    # Resolve against exported PSD's decoded names, not planned empty plates.
    exported_native = load_json(native / 'layers.json')
    _, decoded = adapter.reference_layer_sets(converted, [r['name'] for r in exported_native['layers']])
    motion = model_motion(env, pose, geo, set(decoded))
    write_new_json(native / 'motion.json', motion)
    write_new_json(native / 'own-source-record.json', {
        'pose': pose, 'round': round_name, 'sourceSha256': master_sha,
        'geometrySha256': sha256_file(d / 'plan/geometry.json'),
        'expressionSheetSha256': sha256_file(atlas), 'sourceRig': str(out), 'rasterAdapter': str(env.adapter),
        'rasterProvenanceSha256': sha256_file(native / 'provenance.json'),
        'upstreamBuilderSha256': sha256_file(env.scripts / 'build-source-models.py'),
        'fixesModuleSha256': sha256_file(FIXES_MODULE), 'fixes': fixes.record,
        'motionMethod': 'own measured hand pivots and conservative global retiming of established complete eight-slot phase/reaction curves',
        'oldArtworkUsed': False, 'pendingNativeValidationAndVisualReview': True})
    return {'pose': pose, 'sourceRig': str(out), 'native': str(native), 'psd': str(native / 'model.psd'),
            'motion': str(native / 'motion.json'),
            'registration': [{'scale': r['scale'], 'translation': r['translation'], 'normalizedScore': r['normalizedScore']}
                             for r in fixes.record['registration']['panels']]}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    subs = parser.add_subparsers(dest='command', required=True)
    g = subs.add_parser('geometry', help='measure reviewable pose-local geometry (never overwrites)')
    g.add_argument('--run', required=True)
    g.add_argument('--poses', required=True, help='all or comma-separated pose IDs')
    b = subs.add_parser('build', help='build source rig round + native raster model')
    b.add_argument('--run', required=True)
    b.add_argument('--poses', required=True, help='all or comma-separated pose IDs')
    b.add_argument('--round', required=True, help='new round directory name, e.g. R1')
    args = parser.parse_args()
    run = load_run(args.run)
    env = environment(run)
    poses = run.select(args.poses)
    if args.command == 'geometry':
        path = run.root / 'plan/poses.json'
        if not path.exists():
            write_new_json(path, {'poses': [{'id': p, 'label': env.labels[p]} for p in run.poses]})
    for pose in poses:
        result = geometry(env, pose) if args.command == 'geometry' else build(env, pose, args.round)
        print(json.dumps(result, ensure_ascii=False), flush=True)


if __name__ == '__main__':
    main()
