#!/usr/bin/env python3
"""Refine reviewed white-background native hair pockets without changing other paint."""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image
from scipy.ndimage import binary_dilation, binary_erosion, label
from pymatting import estimate_alpha_cf

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--original-run', type=Path, required=True)
parser.add_argument('--plan', type=Path, required=True)
parser.add_argument('--pose', required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
if args.output.exists():
    raise ValueError('Use a new output directory; preserve every prior matte')
args.output.mkdir(parents=True)
plan = json.loads(args.plan.read_text())
band = plan.get('unknownBandPixels')
if not isinstance(band, int) or isinstance(band, bool) or not 1 <= band <= 12:
    raise ValueError('Record a reviewed 1..12 native-pixel unknown band in the plan')
model = args.original_run / 'native-models' / args.pose
layers = json.loads((model / 'layers.json').read_text())['layers']
repairs = []
records = []
for name in sorted({item['layer'] for item in plan['pockets']}):
    hand_boundary = name in ('handwear-l', 'handwear-r') and name in plan.get('handBoundaryLayers', [])
    if name not in ('back hair', 'front hair') and not hand_boundary:
        raise ValueError('Only reviewed hair layers or explicitly reviewed hand boundaries may be matted')
    layer_output = args.output / name.replace(' ', '_')
    layer_output.mkdir()
    row = next(item for item in layers if item['name'] == name)
    source = model / row['filename']
    old = np.array(Image.open(source).convert('RGBA'))
    rgb = old[:, :, :3].astype(np.float64) / 255
    old_alpha = old[:, :, 3].astype(np.float64) / 255
    reference = rgb * old_alpha[:, :, None] + 1 - old_alpha[:, :, None]
    eligible = (old[:, :, :3].min(axis=2) >= plan['minimumChannel']) & (
        np.ptp(old[:, :, :3].astype(np.int16), axis=2) <= plan['maximumChroma']) & (old[:, :, 3] > 0)
    components, _ = label(eligible)
    selected = np.zeros(eligible.shape, dtype=bool)
    for pocket in plan['pockets']:
        if pocket['layer'] != name:
            continue
        sx, sy = pocket['seed']
        if not isinstance(sx, int) or not isinstance(sy, int) or not (
                row['left'] <= sx < row['right'] and row['top'] <= sy < row['bottom']):
            raise ValueError('Seed outside the named native hair layer')
        component = components[sy - row['top'], sx - row['left']]
        if not component:
            raise ValueError('Reviewed seed does not select a background component')
        selected |= components == component
    # The uncertain band is measured and reviewed in this pose's native coordinates.
    support = binary_dilation(selected, iterations=band)
    ys, xs = np.where(support)
    x0, x1 = max(0, int(xs.min()) - band), min(old.shape[1], int(xs.max()) + band + 1)
    y0, y1 = max(0, int(ys.min()) - band), min(old.shape[0], int(ys.max()) + band + 1)
    local = selected[y0:y1, x0:x1]
    supported = support[y0:y1, x0:x1]
    local_old = old[y0:y1, x0:x1]
    sure_background = binary_erosion(local, iterations=1) | (local_old[:, :, 3] == 0)
    trimap = np.ones(local.shape, dtype=np.float64)
    trimap[supported] = 0.5
    trimap[sure_background] = 0
    if not np.any(trimap == 0) or not np.any(trimap == 1):
        raise ValueError('Each reviewed region needs both foreground and background evidence')
    local_reference = reference[y0:y1, x0:x1]
    alpha = np.clip(estimate_alpha_cf(local_reference, trimap,
        laplacian_kwargs={'epsilon': 1e-7}, cg_kwargs={'maxiter': 3000, 'rtol': 1e-7}), 0, 1)
    # Undo the observed white-background mixture, not a painted recolour.
    foreground = np.clip((local_reference - (1 - alpha[:, :, None])) / np.maximum(alpha[:, :, None], 1e-8), 0, 1)
    result = local_old.copy()
    result[supported, :3] = np.rint(foreground[supported] * 255).astype(np.uint8)
    result[supported, 3] = np.minimum(local_old[supported, 3], np.rint(alpha[supported] * 255)).astype(np.uint8)
    result[supported & (result[:, :, 3] == 0)] = 0
    changed = np.any(result != local_old, axis=2)
    if np.any(changed & ~supported) or np.any(result[:, :, 3] > local_old[:, :, 3]):
        raise AssertionError('Matte must only reduce reviewed alpha support')
    output = old.copy()
    output[y0:y1, x0:x1] = result
    pixels = []
    for yy, xx in zip(*np.where(changed)):
        pixels.append([int(xx + x0 + row['left']), int(yy + y0 + row['top']), *result[yy, xx].tolist()])
    repairs.append({'layer': name, 'pixels': pixels})
    Image.fromarray(output).save(layer_output / 'corrected-hair.png')
    Image.fromarray(np.rint(trimap * 255).astype(np.uint8)).save(layer_output / 'trimap.png')
    Image.fromarray(np.rint(alpha * 255).astype(np.uint8)).save(layer_output / 'estimated-alpha.png')
    records.append({'layer': name, 'source': str(source), 'sourceSha256': hashlib.sha256(source.read_bytes()).hexdigest(),
        'sourceRgbaSha256': hashlib.sha256(old.tobytes()).hexdigest(),
        'backgroundRgb': [255, 255, 255],
        'canvasRegion': [x0 + row['left'], y0 + row['top'], x1 + row['left'], y1 + row['top']],
        'unknownBandPixels': band, 'changedPixels': int(changed.sum()),
        'trimapSha256': hashlib.sha256((layer_output / 'trimap.png').read_bytes()).hexdigest(),
        'method': 'Closed-form alpha matting on source-layer RGB with reviewed foreground/background trimap; white-background colour unmixing only in reviewed support; no warp or new painting'})
plan['matteRepairs'] = repairs
plan['matteRecords'] = records
(args.output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
print(json.dumps(records))
