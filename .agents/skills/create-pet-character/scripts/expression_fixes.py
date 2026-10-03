"""Fork fixes applied to the upstream DAEMONLET source-model builder.

``build-rigs.py build`` loads the upstream runtime's
``scripts/characters/build-source-models.py`` and calls :func:`install` on the
loaded module before ``module.build``. Every replacement keeps the upstream
call signature and return contract (the consumers are quoted next to each
patch), and records its parameters and measurements in ``FixContext.record``,
which ``build-rigs.py`` writes to ``native-models/<pose>/own-source-record.json``
under ``fixes``.

Fixes (numbering follows the skill's fork-fix list):

* #2 registration: ``register_panel`` searched translation only (+-12 px) on the
  nose patch. Qwen-Image-2.1 output drifts in scale as well. Both panels come
  from one render, so the replacement estimates a similarity transform (scale +
  translation) on panel 1's face-feature patch (nose + eyes + eyebrows, front
  hair excluded), registers panel 0 with that scale and a translation search
  near panel 1's, and rejects misregistered panels instead of using them.
* #4 closed lids: ``rebuild_lid`` scored lash pixels with a warm-brown rule
  ``(183-G)/80 * (R-G-18)/30`` that misses black/green lashes and picks up red
  hair. The replacement is a hue-independent luminance-contrast detector inside
  the eye anchor box that ignores the pose's own front-hair and eyebrow alpha.
* #5 mouth constants: ``add_mouth_expressions`` used fixed 1280-canvas pixel
  constants; the replacement scales them by the reviewed neutral mouth width,
  keeps the search window inside the own face (never reaching the chin contour)
  and rejects jaw/contour components.
* #7 blink stability: ``blink_profiles`` output is Gaussian-regularised (aperture
  and closure trajectories only; no RGB or alpha edits), reproducing the method
  recorded by the proven pipeline.

Only numpy + Pillow; Python 3.9.
"""
from __future__ import annotations

from typing import Any

import numpy as np
from PIL import Image, ImageFilter

# --- Fix #2: similarity registration -------------------------------------------------
# Both panels come from one Qwen render, split at w/2 and resized by the same
# factor, so they share the render's scale. Panel 1 (open eyes) is matched on the
# full nose/eye/eyebrow patch; that patch is well conditioned and estimates the
# similarity transform. Panel 0 (closed eyes) must exclude the eye interiors,
# which leaves nose, brows and flat skin: its free scale estimate is
# ill-conditioned (1.00-1.17 on the proven sheets), so panel 0 reuses panel 1's
# scale, searches translation only near panel 1's translation, and its scale is
# verified by matching panel 0 to panel 1 on their shared content.
#
# Calibrated on the ten proven Rubelia sheets (rubelia-anime-default-20261002):
# panel 1 scale 0.990-1.055, |translation| <= 16 px, normalized score
# 0.104-0.174; panel 0 offset from panel 1 <= 10.5 px per axis, normalized score
# 0.059-0.371; panel 0 -> panel 1 scale 0.996-1.005. Synthetic negatives on the
# same sheets: panel 0 alone rescaled by 1.08/0.92/1.15 -> panel scale
# 0.922-0.931/1.081-1.093/0.869-0.874; panel 0 shifted out of the offset range
# -> normalized score >= 1.18; panel 0 mirrored -> score >= 2.6.
REGISTRATION_MAX_SHIFT = 24.0
# Searched scale interval. It extends past the +-0.1 acceptance limit so that a
# panel drifting beyond the limit is measured and rejected, not clamped to 0.9/1.1.
REGISTRATION_SCALE_SEARCH = (0.82, 1.18)
REGISTRATION_MAX_SCALE_DELTA = 0.10
# Score is the upstream metric (zero-mean SSD + 0.15 * absolute SSD per channel)
# divided by the score a featureless panel filled with the source patch mean
# would get (1.15 * patch variance). 0 = identical structure; an uncorrelated
# panel of similar contrast scores ~2.
REGISTRATION_MAX_NORMALIZED_SCORE = 0.35
# Panel 0: translation searched within this many px of panel 1's translation at
# panel 1's scale; reaching the limit rejects. Its patch has the eye interiors
# removed and closed-lid paint nearby, so its score limit is looser.
REGISTRATION_PANEL0_MAX_OFFSET = 14.0
REGISTRATION_PANEL0_MAX_NORMALIZED_SCORE = 0.55
# Panel 0's scale is verified directly against panel 1 on their shared content
# (sheet crop inset by the shift limit, eyes/mouth excluded with this padding):
# that match is well conditioned.
REGISTRATION_PANEL_EXCLUSION_PAD = 16
REGISTRATION_PANEL_SCALE_TOLERANCE = 0.03
REGISTRATION_MIN_PIXELS = 200

# --- Fix #4: closed-lid lash detector ------------------------------------------------
# Local skin luminance = this quantile of unoccluded window luminance (same
# quantile the upstream unmixing uses for its background estimate).
LID_SKIN_QUANTILE = 0.78
# Luminance contrast (Rec.601 luma, 0-255) below local skin: cel-shaded skin
# shadow steps stay below ~18; lash/lid line art is far darker. Darkness ramps
# from 0 at 18 to 1 at 88 regardless of hue.
LID_CONTRAST_FLOOR = 18.0
LID_CONTRAST_RANGE = 70.0
LID_OCCLUDER_ALPHA = 24

# --- Fix #5: mouth constants ---------------------------------------------------------
# Upstream constants (1280 canvas). Its clamps are linear (unclamped) for neutral
# mouth widths (anchors['mouth'] x1-x0) of 30/1.4=21.4 .. 43/1.4=30.7 px; the
# unmodified upstream production run measured neutral mouths of 20-32 px. The
# scale k is therefore derived from the reviewed neutral mouth width itself:
# k = 1 inside that tuned range (exact upstream behaviour) and otherwise the
# factor that brings the width back to the nearest range end, so every window
# and width clamp stays proportional to the actual mouth. Face width is not a
# proxy: the proven sheets have 171-220 px faces with 26-38 px mouths.
MOUTH_HALF_WIDTH = (30.0, 43.0)
MOUTH_ROWS = 18.0
MOUTH_OPEN_WIDTH = (16.0, 32.0)
MOUTH_SMILE_WIDTH = (22.0, 45.0)
MOUTH_NEUTRAL_WIDTH_RANGE = (MOUTH_HALF_WIDTH[0] / 1.4, MOUTH_HALF_WIDTH[1] / 1.4)
# The search window stays inside the selected own face component: its bottom at
# least this many px (x k) above the face bottom, so it never reaches the chin
# contour, and its sides this many px (x k) inside the face's horizontal extent
# on the window rows.
MOUTH_CHIN_MARGIN = 8.0
MOUTH_FACE_EDGE_MARGIN = 4.0
# A feature wider than this multiple of the neutral width, or touching both
# window sides, is a jaw/contour/shading component rather than a mouth.
MOUTH_MAX_WIDTH_RATIO = 2.0

# --- Fix #7: blink regularisation ----------------------------------------------------
# Reproduces canon-face/blink-geometry-regularization.json of the proven run
# exactly (max error 5e-15 over all 20 eyes): sampled Gaussian with sigma = 3
# profile samples, truncated at 3 sigma (radius 9), edge-replicated ends,
# applied independently to the 49-sample upper/lower/closed trajectories.
BLINK_SIGMA = 3.0
BLINK_RADIUS = 9
BLINK_CURVES = ('upper', 'lower', 'closed')


def _bounds(mask: np.ndarray) -> list[int] | None:
    ys, xs = np.where(mask)
    if not len(xs):
        return None
    return [int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1]


def _union(boxes: list[list[int]]) -> list[int]:
    return [min(b[0] for b in boxes), min(b[1] for b in boxes), max(b[2] for b in boxes), max(b[3] for b in boxes)]


def _dilate(mask: np.ndarray, size: int) -> np.ndarray:
    image = Image.fromarray(np.uint8(mask) * 255).filter(ImageFilter.MaxFilter(size))
    return np.array(image) > 0


def _luma(rgb: np.ndarray) -> np.ndarray:
    return rgb[..., 0] * .299 + rgb[..., 1] * .587 + rgb[..., 2] * .114


class FixContext:
    """Pose-local inputs for the patches plus the record written to own-source-record."""

    def __init__(self, geometry: dict[str, Any]):
        self.geometry = geometry
        self.panel_index = 0
        self.front_hair: np.ndarray | None = None
        self.lid_occluders: np.ndarray | None = None
        self.face_mask: np.ndarray | None = None
        # Panel 1 as upstream ``add_expressions`` builds it, its sheet crop and its registration.
        self.reference_panel: np.ndarray | None = None
        self.panel_crop: list[int] | None = None
        self.reference: dict[str, Any] | None = None
        self.record: dict[str, Any] = {
            'eyeContours': {
                'fix': 4,
                'method': 'Neutral eye apertures and irides come only from reviewed See-through alpha masks '
                          '(sourceEyeMasks/sourceIrisMasks) checked against actual source pigment; the '
                          'upstream warm-RGB fallback is refused.',
            },
            'registration': {
                'fix': 2,
                'method': 'Similarity (scale + translation) coarse-to-fine search on panel 1 (open eyes) over the '
                          'own nose/eye/eyebrow patch with front hair and the neutral mouth excluded. Panel 0 '
                          '(closed eyes; eye apertures and lash contours also excluded) reuses panel 1 scale '
                          '(same Qwen render) and searches translation only near panel 1 translation; panel 0 '
                          'scale is verified by a free similarity match of panel 0 to panel 1 on the shared '
                          'sheet content outside eyes and mouth.',
                'scaleSearch': list(REGISTRATION_SCALE_SEARCH),
                'maxScaleDelta': REGISTRATION_MAX_SCALE_DELTA,
                'maxShiftPx': REGISTRATION_MAX_SHIFT,
                'maxNormalizedScore': REGISTRATION_MAX_NORMALIZED_SCORE,
                'panel0MaxOffsetPx': REGISTRATION_PANEL0_MAX_OFFSET,
                'panel0MaxNormalizedScore': REGISTRATION_PANEL0_MAX_NORMALIZED_SCORE,
                'panelScaleTolerance': REGISTRATION_PANEL_SCALE_TOLERANCE,
                'panelExclusionPadPx': REGISTRATION_PANEL_EXCLUSION_PAD,
                'panels': [],
            },
            'closedLids': {
                'fix': 4,
                'method': 'Hue-independent luma contrast below local skin inside the eye anchor box; own front '
                          'hair and eyebrow alpha excluded; upstream ridge tracing and edge unmixing retained.',
                'skinQuantile': LID_SKIN_QUANTILE,
                'contrastFloor': LID_CONTRAST_FLOOR,
                'contrastRange': LID_CONTRAST_RANGE,
                'eyes': {},
            },
            'mouthExpressions': {
                'fix': 5,
                'method': 'Upstream mouth window/width constants multiplied by k derived from the reviewed neutral '
                          'mouth width (k = 1 inside the upstream-tuned neutral range); window top kept below the '
                          'own nose, bottom above the own face bottom minus a margin, sides inside the own face; '
                          'features touching both window sides or wider than the ratio limit are rejected.',
                'neutralWidthRange': list(MOUTH_NEUTRAL_WIDTH_RANGE),
                'chinMarginPx': MOUTH_CHIN_MARGIN,
                'faceEdgeMarginPx': MOUTH_FACE_EDGE_MARGIN,
                'maxWidthRatio': MOUTH_MAX_WIDTH_RATIO,
                'reviewedNeutralMouthMaskRequired': True,
                'mouths': {},
            },
            'blinkRegularization': {
                'fix': 7,
                'method': 'Gaussian regularization of pose-local measured aperture and closure trajectories '
                          'for coarse mesh stability; no RGB or alpha edits.',
                'sigmaSamples': BLINK_SIGMA,
                'radiusSamples': BLINK_RADIUS,
                'padding': 'edge',
                'curves': list(BLINK_CURVES),
                'rasterPixelsChanged': False,
                'eyes': {},
            },
        }


# --- Fix #2 ----------------------------------------------------------------------------

def _bilinear(image: np.ndarray, x: np.ndarray, y: np.ndarray) -> np.ndarray:
    h, w = image.shape[:2]
    x = np.clip(x, 0, w - 1.0001)
    y = np.clip(y, 0, h - 1.0001)
    x0 = np.floor(x).astype(np.intp)
    y0 = np.floor(y).astype(np.intp)
    fx = (x - x0)[:, None]
    fy = (y - y0)[:, None]
    top = image[y0, x0] * (1 - fx) + image[y0, x0 + 1] * fx
    bottom = image[y0 + 1, x0] * (1 - fx) + image[y0 + 1, x0 + 1] * fx
    return top * (1 - fy) + bottom * fy


def _score(target: np.ndarray, sample: np.ndarray) -> float:
    # Upstream register_panel metric over the selected pixels.
    centred = (target - target.mean(0)) - (sample - sample.mean(0))
    return float(np.mean(centred ** 2) + .15 * np.mean((target - sample) ** 2))


def _search(points, center, target, image, scales, shifts_x, shifts_y, best=None):
    for scale in scales:
        base_x = center[0] + (points[:, 0] - center[0]) / scale
        base_y = center[1] + (points[:, 1] - center[1]) / scale
        for tx in shifts_x:
            for ty in shifts_y:
                score = _score(target, _bilinear(image, base_x - tx / scale, base_y - ty / scale))
                if best is None or score < best[0]:
                    best = (score, float(scale), float(tx), float(ty))
    return best


def _around(value: float, radius: float, step: float, low: float, high: float) -> np.ndarray:
    values = value + np.arange(-radius, radius + step / 2, step)
    return np.unique(np.round(np.clip(values, low, high), 6))


def _feature_patch(context: FixContext, source: np.ndarray, nose_bounds, closed_eyes: bool) -> tuple[list[int], np.ndarray]:
    geometry = context.geometry
    measured = geometry['measurements']
    boxes = [list(map(int, nose_bounds))]
    boxes += [list(geometry['sourceEyeApertures'][side]) for side in ('l', 'r')]
    boxes += [measured['parts'][name] for name in ('eyebrow-l', 'eyebrow-r') if name in measured.get('parts', {})]
    union = _union(boxes)
    margin = max(6, round(.1 * (union[2] - union[0])))
    h, w = source.shape[:2]
    patch = [max(0, union[0] - margin), max(0, union[1] - margin), min(w, union[2] + margin), min(h, union[3] + margin)]
    mouth = geometry['neutralMouthBounds']
    # Both expression panels change the mouth (an opened upper lip rises): exclude
    # rows from max(3 px, half the neutral mouth height) above its top downward.
    patch[3] = min(patch[3], mouth[1] - max(3, (mouth[3] - mouth[1]) // 2))
    if patch[3] - patch[1] < 8 or patch[2] - patch[0] < 8:
        raise ValueError('Registration feature patch collapsed; inspect own nose/eye/mouth geometry')
    mask = np.zeros((h, w), bool)
    mask[patch[1]:patch[3], patch[0]:patch[2]] = True
    mask &= source[:, :, 3] > 24
    if context.front_hair is not None:
        mask &= ~_dilate(context.front_hair, 5)
    if closed_eyes:
        # Panel 0 is the closed-eye panel: its eye interiors legitimately differ.
        for side in ('l', 'r'):
            eye = _union([measured['eyes'][side]['apertureBounds'], measured['eyes'][side]['lashBounds']])
            mask[max(0, eye[1] - 3):eye[3] + 3, max(0, eye[0] - 3):eye[2] + 3] = False
    return patch, mask


def _patch_samples(context: FixContext, source: np.ndarray, nose_bounds, closed_eyes: bool) -> dict[str, Any]:
    patch, mask = _feature_patch(context, source, nose_bounds, closed_eyes)
    ys, xs = np.where(mask)
    if len(xs) < REGISTRATION_MIN_PIXELS:
        raise ValueError(f'Registration patch has only {len(xs)} unoccluded own feature pixels; inspect hair/eye geometry')
    target = source[ys, xs, :3].astype(np.float64)
    baseline = 1.15 * float(np.mean((target - target.mean(0)) ** 2))
    if baseline <= 1e-6:
        raise ValueError('Registration patch has no source structure; inspect own feature geometry')
    return {'patch': patch, 'ys': ys, 'xs': xs, 'points': np.stack([xs, ys], 1).astype(np.float64),
            'center': ((patch[0] + patch[2] - 1) / 2, (patch[1] + patch[3] - 1) / 2),
            'target': target, 'baseline': baseline}


def _panel_record(samples: dict[str, Any], panel_rgb: np.ndarray, index: int, nose_bounds, score: float,
                  s: float, tx: float, ty: float) -> dict[str, Any]:
    points, target, baseline = samples['points'], samples['target'], samples['baseline']
    identity = _score(target, _bilinear(panel_rgb, points[:, 0], points[:, 1]))
    center = samples['center']
    return {'translation': [round(tx, 3), round(ty, 3)], 'scale': round(s, 5), 'score': score,
            'nose_bounds': [int(v) for v in nose_bounds], 'panel': index, 'center': [center[0], center[1]],
            'featurePatch': samples['patch'], 'featurePixels': int(len(samples['xs'])), 'identityScore': identity,
            'normalizedScore': score / baseline, 'normalizedIdentityScore': identity / baseline}


def _reject(context: FixContext, record: dict[str, Any], problems: list[str]) -> None:
    record['rejected'] = problems
    context.record['registration']['panels'].append(record)
    raise ValueError(f'Misregistered expression panel {record["panel"]} ({"; ".join(problems)}); '
                     'preserve this sheet and regenerate the expression sheet')


def _register_reference(context: FixContext, source: np.ndarray, nose_bounds) -> dict[str, Any]:
    """Similarity estimate for panel 1 (open eyes), the transform both panels share."""
    panel = context.reference_panel
    if panel is None:
        raise ValueError('Expression panel 1 was not prepared before registration')
    samples = _patch_samples(context, source, nose_bounds, closed_eyes=False)
    points, target, center = samples['points'], samples['target'], samples['center']
    ys, xs = samples['ys'], samples['xs']
    panel_rgb = panel[:, :, :3].astype(np.float32)
    # Coarse: blurred images, every other pixel in both axes.
    blur = lambda array: np.array(Image.fromarray(np.uint8(array)).filter(ImageFilter.GaussianBlur(1.5)), np.float32)
    coarse = (ys % 2 == 0) & (xs % 2 == 0)
    source_blur, panel_blur = blur(source[:, :, :3]), blur(panel[:, :, :3])
    low, high = REGISTRATION_SCALE_SEARCH
    m = REGISTRATION_MAX_SHIFT
    shifts = np.arange(-m, m + .5, 3.0)
    best = _search(points[coarse], center, source_blur[ys[coarse], xs[coarse]].astype(np.float64), panel_blur,
                   np.round(np.arange(low, high + .001, .02), 6), shifts, shifts)
    # Fine: full-resolution pixels around the coarse optimum, then sub-pixel.
    _, s, tx, ty = best
    best = _search(points, center, target, panel_rgb, _around(s, .02, .005, low, high),
                   _around(tx, 3, 1, -m, m), _around(ty, 3, 1, -m, m))
    _, s, tx, ty = best
    score, s, tx, ty = _search(points, center, target, panel_rgb, _around(s, .005, .001, low, high),
                               _around(tx, 1, .5, -m, m), _around(ty, 1, .5, -m, m))
    record = _panel_record(samples, panel_rgb, 1, nose_bounds, score, s, tx, ty)
    record['scaleSource'] = 'panel 1 similarity estimate'
    problems = []
    if abs(s - 1) > REGISTRATION_MAX_SCALE_DELTA:
        problems.append(f'scale {s:.3f} outside 1+-{REGISTRATION_MAX_SCALE_DELTA}')
    if max(abs(tx), abs(ty)) >= m:
        problems.append(f'translation ({tx:.1f},{ty:.1f}) reached the +-{m:.0f}px search limit')
    if record['normalizedScore'] > REGISTRATION_MAX_NORMALIZED_SCORE:
        problems.append(f'normalized score {record["normalizedScore"]:.3f} > {REGISTRATION_MAX_NORMALIZED_SCORE}')
    if problems:
        _reject(context, record, problems)
    return {'record': record, 'scale': s, 'translation': (tx, ty), 'center': center,
            'nose_bounds': [int(v) for v in nose_bounds]}


def _panel_scale_ratio(context: FixContext, closed: np.ndarray, reference: np.ndarray) -> dict[str, Any]:
    """Free similarity from panel 0 to panel 1 over the shared, unchanged sheet content.

    Both panels are Qwen drawings of the same render, so hair, face contour, ears,
    brows and nose match closely; only eyes and mouth change. Matching that large
    area is well conditioned and verifies that panel 0 really has panel 1's scale.
    """
    crop = context.panel_crop
    if crop is None:
        raise ValueError('Expression panel crop was not prepared before registration')
    geometry = context.geometry
    measured = geometry['measurements']
    h, w = reference.shape[:2]
    inset = int(REGISTRATION_MAX_SHIFT)  # panel edges may carry seams or drift in/out
    mask = np.zeros((h, w), bool)
    mask[max(0, crop[1] + inset):crop[3] - inset, max(0, crop[0] + inset):crop[2] - inset] = True
    mask &= (reference[:, :, 3] > 0) & (closed[:, :, 3] > 0)
    pad = REGISTRATION_PANEL_EXCLUSION_PAD
    for side in ('l', 'r'):
        eye = _union([measured['eyes'][side]['apertureBounds'], measured['eyes'][side]['lashBounds']])
        mask[max(0, eye[1] - pad):eye[3] + pad, max(0, eye[0] - pad):eye[2] + pad] = False
    mouth = geometry['neutralMouthBounds']
    half = (mouth[2] - mouth[0]) // 2 + pad
    mask[max(0, mouth[1] - pad):mouth[3] + 2 * pad, max(0, mouth[0] - half):mouth[2] + half] = False
    ys, xs = np.where(mask)
    if len(xs) < REGISTRATION_MIN_PIXELS:
        raise ValueError(f'Expression panels share only {len(xs)} pixels outside eyes/mouth; inspect expression crop')
    points = np.stack([xs, ys], 1).astype(np.float64)
    center = ((crop[0] + crop[2] - 1) / 2, (crop[1] + crop[3] - 1) / 2)
    target = reference[ys, xs, :3].astype(np.float64)
    baseline = 1.15 * float(np.mean((target - target.mean(0)) ** 2))
    if baseline <= 1e-6:
        raise ValueError('Expression panel 1 has no structure outside eyes/mouth; inspect the sheet')
    closed_rgb = closed[:, :, :3].astype(np.float32)
    blur = lambda array: np.array(Image.fromarray(np.uint8(array)).filter(ImageFilter.GaussianBlur(1.5)), np.float32)
    sparse = (ys % 3 == 0) & (xs % 3 == 0)
    low, high = REGISTRATION_SCALE_SEARCH
    m = REGISTRATION_MAX_SHIFT
    shifts = np.arange(-m, m + .5, 3.0)
    best = _search(points[sparse], center, blur(reference[:, :, :3])[ys[sparse], xs[sparse]].astype(np.float64),
                   blur(closed[:, :, :3]), np.round(np.arange(low, high + .001, .02), 6), shifts, shifts)
    _, s, tx, ty = best
    best = _search(points[sparse], center, target[sparse], closed_rgb, _around(s, .02, .005, low, high),
                   _around(tx, 3, 1, -m, m), _around(ty, 3, 1, -m, m))
    _, s, tx, ty = best
    _, s, tx, ty = _search(points[sparse], center, target[sparse], closed_rgb, _around(s, .005, .001, low, high),
                           _around(tx, 1, .5, -m, m), _around(ty, 1, .5, -m, m))
    score = _score(target, _bilinear(closed_rgb, center[0] + (points[:, 0] - center[0] - tx) / s,
                                     center[1] + (points[:, 1] - center[1] - ty) / s))
    return {'scale': round(s, 5), 'translation': [round(tx, 3), round(ty, 3)], 'normalizedScore': score / baseline,
            'pixels': int(len(xs))}


def _register_closed(context: FixContext, source: np.ndarray, panel: np.ndarray, nose_bounds,
                     reference: dict[str, Any]) -> tuple[dict[str, Any], float, float, float]:
    """Panel 0 (closed eyes): panel 1 scale, translation searched near panel 1's."""
    samples = _patch_samples(context, source, nose_bounds, closed_eyes=True)
    points, target, center = samples['points'], samples['target'], samples['center']
    if center != reference['center']:
        raise ValueError('Registration patch differs between expression panels; inspect own feature geometry')
    panel_rgb = panel[:, :, :3].astype(np.float32)
    s = reference['scale']
    rx, ry = reference['translation']
    r = REGISTRATION_PANEL0_MAX_OFFSET
    best = _search(points, center, target, panel_rgb, [s], _around(rx, r, 1, rx - r, rx + r), _around(ry, r, 1, ry - r, ry + r))
    _, _, tx, ty = best
    score, _, tx, ty = _search(points, center, target, panel_rgb, [s], _around(tx, 1, .25, rx - r, rx + r),
                               _around(ty, 1, .25, ry - r, ry + r))
    record = _panel_record(samples, panel_rgb, 0, nose_bounds, score, s, tx, ty)
    offset = [round(tx - rx, 3), round(ty - ry, 3)]
    ratio = _panel_scale_ratio(context, panel, context.reference_panel)
    record.update({'scaleSource': 'panel 1 similarity estimate (same Qwen render)', 'offsetFromPanel1': offset,
                   'panel0ToPanel1': ratio})
    problems = []
    if abs(ratio['scale'] - 1) > REGISTRATION_PANEL_SCALE_TOLERANCE:
        problems.append(f'panel 0 -> panel 1 scale {ratio["scale"]:.3f} outside 1+-{REGISTRATION_PANEL_SCALE_TOLERANCE} '
                        '(panels of one render must share scale)')
    if max(abs(offset[0]), abs(offset[1])) >= r:
        problems.append(f'offset from panel 1 ({offset[0]:.1f},{offset[1]:.1f}) reached the +-{r:.0f}px search limit')
    if record['normalizedScore'] > REGISTRATION_PANEL0_MAX_NORMALIZED_SCORE:
        problems.append(f'normalized score {record["normalizedScore"]:.3f} > {REGISTRATION_PANEL0_MAX_NORMALIZED_SCORE} '
                        'at panel 1 scale')
    if problems:
        _reject(context, record, problems)
    return record, s, tx, ty


def similarity_register(context: FixContext, source: np.ndarray, panel: np.ndarray, nose_bounds):
    """Replacement for ``register_panel(source, panel, nose_bounds)``.

    Consumer (upstream ``add_expressions``): ``registered, record = register_panel(...)``,
    called for panel 0 then panel 1; ``registered`` goes into ``panels`` (used as
    ``panel[..., :3]`` by ``rebuild_lid`` and copied by ``mouth_feature``/``sprite``), so it
    stays a full-canvas uint8 RGBA array transformed with the same PIL affine/bicubic call;
    ``record`` is JSON-dumped into the round provenance under ``expression_registration``.
    Panel 1 is registered first (from ``context.reference_panel``, prepared by the
    ``add_expressions`` wrapper exactly as upstream builds it) because panel 0 needs its scale.
    """
    index = context.panel_index
    context.panel_index += 1
    if index == 0:
        context.reference = _register_reference(context, source, nose_bounds)
        record, s, tx, ty = _register_closed(context, source, panel, nose_bounds, context.reference)
        center = context.reference['center']
    elif index == 1:
        reference = context.reference
        if (reference is None or reference['nose_bounds'] != [int(v) for v in nose_bounds]
                or context.reference_panel is None or not np.array_equal(panel, context.reference_panel)):
            raise ValueError('Expression panel 1 differs from the panel registered before panel 0')
        record, s, center = reference['record'], reference['scale'], reference['center']
        tx, ty = reference['translation']
    else:
        raise ValueError(f'Unexpected expression panel {index}; the sheet has two panels')
    context.record['registration']['panels'].append(record)
    # output(q) = panel(c + (q - c - t) / s); PIL applies coefficients to pixel-centre
    # coordinates, hence the 0.5 - 0.5/s correction (zero when s == 1, matching upstream).
    a = 1 / s
    cx = center[0] - (center[0] + tx) / s + .5 - .5 / s
    cy = center[1] - (center[1] + ty) / s + .5 - .5 / s
    h, w = panel.shape[:2]
    shifted = np.array(Image.fromarray(panel).transform((w, h), Image.Transform.AFFINE, (a, 0, cx, 0, a, cy),
                                                       Image.Resampling.BICUBIC))
    return shifted, record


# --- Fix #4 ----------------------------------------------------------------------------

def contrast_lid(context: FixContext, panel: np.ndarray, a: dict[str, Any], side: str):
    """Replacement for ``source_eye_blinks.rebuild_lid(panel, a, side)``.

    Consumer (upstream ``add_expressions``): ``lid, _ = rebuild_lid(panels[0], anchors[key], side)``;
    ``lid`` becomes ``parts['eye_close-'+side]`` (full-canvas uint8 RGBA, transparent RGB
    zeroed), its alpha must be non-empty for ``weighted_center`` and ``make_profile``
    (alpha > 32 with dark G channel). The second value keeps the upstream
    ``{'window', 'ridge'}`` shape.
    """
    if context.lid_occluders is None:
        raise ValueError('Lid occluder masks were not prepared before closed-lid extraction')
    H, W = panel.shape[:2]
    eye_w = a['x1'] - a['x0'] + 1
    eye_h = a['y1'] - a['y0'] + 1
    # Anchor box plus a margin proportional to the eye for the lash tail
    # (outer corner) and a lid drawn slightly below the open aperture.
    outer, inner = max(3, round(.2 * eye_w)), max(2, round(.08 * eye_w))
    x0 = max(0, int(a['x0'] - (outer if side == 'l' else inner)))
    x1 = min(W, int(a['x1'] + (inner if side == 'l' else outer) + 1))
    y0 = max(0, int(a['y0'] - max(2, round(.15 * eye_h))))
    y1 = min(H, int(a['y1'] + max(4, round(.5 * eye_h)) + 1))
    rgb = panel[y0:y1, x0:x1, :3].astype(float)
    coverage = panel[y0:y1, x0:x1, 3].astype(float) / 255
    occluded = context.lid_occluders[y0:y1, x0:x1]
    h, w = rgb.shape[:2]
    luma = _luma(rgb)
    open_skin = ~occluded & (coverage > .5)
    if open_skin.sum() < max(8, w):
        raise ValueError(f'Eye {side} is mostly covered by own front hair/eyebrow; no closed-lid paint to extract')
    skin = float(np.quantile(luma[open_skin], LID_SKIN_QUANTILE))
    dark = np.clip((skin - luma - LID_CONTRAST_FLOOR) / LID_CONTRAST_RANGE, 0, 1) * coverage
    dark[occluded] = 0
    # Upstream ridge tracing (one connected lash ridge, +-2 rows per column).
    yy, xx = np.indices((h, w))
    expected = (a['y0'] + a['y1']) / 2 + 3 - y0
    scores = dark * 3 - .002 * (yy - expected) ** 2
    cost = np.empty((h, w))
    back = np.zeros((h, w), int)
    cost[:, 0] = scores[:, 0]
    rows = np.arange(h)
    offsets = np.arange(-2, 3)
    for x in range(1, w):
        candidates = np.full((len(offsets), h), -np.inf)
        for i, offset in enumerate(offsets):
            origin = rows + offset
            valid = (origin >= 0) & (origin < h)
            candidates[i, valid] = cost[origin[valid], x - 1] - .18 * offset * offset
        at = np.argmax(candidates, 0)
        cost[:, x] = scores[:, x] + candidates[at, rows]
        back[:, x] = rows + offsets[at]
    ridge = np.zeros(w, int)
    ridge[-1] = int(np.argmax(cost[:, -1]))
    for x in range(w - 1, 0, -1):
        ridge[x - 1] = back[ridge[x], x]
    band = np.clip(6.5 - np.abs(yy - ridge[None, :]), 0, 1)
    alpha = np.clip((dark - .04) / .70, 0, 1) * band
    if w > 4:
        alpha[:, :2] *= np.array([.2, .7])[None, :]
        alpha[:, -2:] *= np.array([.7, .2])[None, :]
    # Upstream edge unmixing against the per-column skin estimate, measured on
    # unoccluded pixels only.
    background = np.empty((w, 3))
    for x in range(w):
        column = open_skin[:, x]
        background[x] = np.quantile(rgb[column, x] if column.sum() >= 3 else rgb[open_skin], LID_SKIN_QUANTILE, axis=0)
    weight = np.maximum(alpha[:, :, None], .05)
    foreground = np.clip((rgb - (1 - alpha[:, :, None]) * background[None, :, :]) / weight, 0, 255)
    foreground[alpha > .97] = rgb[alpha > .97]
    out = np.zeros((H, W, 4), np.uint8)
    out[y0:y1, x0:x1, :3] = np.uint8(np.rint(foreground))
    out[y0:y1, x0:x1, 3] = np.uint8(np.rint(alpha * 255))
    out[out[:, :, 3] == 0, :3] = 0
    solid = int((out[:, :, 3] > 32).sum())
    if solid < max(4, w // 3):
        raise ValueError(f'No closed-lid lash contrast found for eye {side} in expression panel 0; '
                         'preserve this sheet and regenerate the expression sheet')
    context.record['closedLids']['eyes'][side] = {
        'window': [x0, y0, x1, y1], 'skinLuma': skin, 'occludedPixels': int(occluded.sum()), 'lidPixels': solid}
    return out, {'window': [x0, y0, x1, y1], 'ridge': [[x + x0, int(y + y0)] for x, y in enumerate(ridge)]}


# --- Fix #5 ----------------------------------------------------------------------------

def _mouth_scale(neutral_width: float) -> float:
    low, high = MOUTH_NEUTRAL_WIDTH_RANGE
    if neutral_width < low:
        return neutral_width / low
    if neutral_width > high:
        return neutral_width / high
    return 1.0


def _inside_face(face: np.ndarray, x0: int, y0: int, y1: int, x1: int, margin: float) -> tuple[int, int]:
    """Clamp a window's x range [x0, x1) to the own face extent on every window row."""
    rows = face[y0:y1]
    if not len(rows) or not rows.any(1).all():
        raise ValueError(f'Mouth search rows {y0}-{y1} leave the own face component; inspect neutral mouth/face geometry')
    left = int(np.argmax(rows, 1).max())
    right = int(face.shape[1] - np.argmax(rows[:, ::-1], 1).max())
    return max(x0, int(np.ceil(left + margin))), min(x1, int(np.floor(right - margin)))


def scaled_mouths(context: FixContext, module, panels, parts, anchors, bounds_by_name):
    """Replacement for ``add_mouth_expressions(panels, parts, anchors, bounds_by_name)``.

    Consumer (upstream ``add_expressions``): called for its side effect of setting
    ``parts['mouth_open']`` and ``parts['mouth_smile']`` (full-canvas RGBA arrays), return
    value unused. Body is the upstream one with each pixel constant multiplied by ``k``
    (from the neutral mouth width), the default window clamped inside the own face, and
    jaw/contour features rejected.
    """
    face = context.face_mask
    if face is None:
        raise ValueError('Own face component was not measured before mouth expressions')
    face_bounds = _bounds(face)
    nose = context.geometry['measurements']['parts'].get('nose')
    a = anchors['mouth']
    mc = np.array([a['cx'], a['cy']])
    neutral_width = a['x1'] - a['x0']
    k = _mouth_scale(neutral_width)
    chin_limit = face_bounds[3] - MOUTH_CHIN_MARGIN * k
    edge = MOUTH_FACE_EDGE_MARGIN * k
    record = context.record['mouthExpressions']
    record.update({'scale': k, 'neutralWidth': neutral_width, 'faceBounds': face_bounds, 'chinLimitY': chin_limit})
    if a['y1'] > chin_limit:
        raise ValueError(f'Neutral mouth bottom {a["y1"]} is within {MOUTH_CHIN_MARGIN * k:.1f}px of the own face '
                         f'bottom {face_bounds[3]}; inspect neutralMouthMask and faceComponentPoint')
    for name, index in [('mouth_open', 0), ('mouth_smile', 1)]:
        if name in bounds_by_name:
            bounds = [int(v) for v in bounds_by_name[name]]
            source = 'request mouth_bounds'
        else:
            half_width = max(MOUTH_HALF_WIDTH[0] * k, min(MOUTH_HALF_WIDTH[1] * k, neutral_width * 1.4))
            rows = MOUTH_ROWS * k
            top = mc[1] - rows
            if nose is not None:
                # The mouth search never includes the nose.
                top = min(max(top, nose[3] + 1), mc[1] - 4)
            # Never reach the chin contour: stay MOUTH_CHIN_MARGIN*k above the face bottom.
            y0, y1 = int(top), int(min(mc[1] + rows + 1, chin_limit))
            x0, x1 = _inside_face(face, int(mc[0] - half_width), y0, y1, int(mc[0] + half_width + 1), edge)
            bounds = [x0, y0, x1, y1]
            source = 'neutral-width-scaled upstream window inside own face'
            if x0 > a['x0'] or x1 < a['x1']:
                raise ValueError(f'{name} search window {bounds} inside the own face cannot contain the neutral mouth '
                                 f'[{a["x0"]},{a["x1"]}]; inspect face component and neutral mouth geometry')
        ar, b = module.mouth_feature(panels[index], bounds)
        feature_width = b[2] - b[0]
        entry = {'window': [int(v) for v in bounds], 'windowSource': source, 'featureBounds': [int(v) for v in b],
                 'featureWidth': int(feature_width)}
        record['mouths'][name] = entry
        problems = []
        if b[0] <= bounds[0] + 1 and b[2] >= bounds[2] - 1:
            problems.append(f'feature {list(b)} spans the whole search window {bounds}')
        if feature_width > MOUTH_MAX_WIDTH_RATIO * neutral_width:
            problems.append(f'feature width {feature_width}px > {MOUTH_MAX_WIDTH_RATIO} x neutral width {neutral_width}px')
        if problems:
            entry['rejected'] = problems
            raise ValueError(f'{name} extraction picked a jaw/contour component, not a mouth ({"; ".join(problems)}); '
                             'preserve this sheet and regenerate the expression sheet or review the face geometry')
        if name == 'mouth_open':
            width = float(np.clip(neutral_width * .85, MOUTH_OPEN_WIDTH[0] * k, MOUTH_OPEN_WIDTH[1] * k))
        else:
            width = float(np.clip(neutral_width * 1.12, MOUTH_SMILE_WIDTH[0] * k, MOUTH_SMILE_WIDTH[1] * k))
        scale = width / feature_width
        center = np.array([(b[0] + b[2] - 1) / 2, (b[1] + b[3] - 1) / 2])
        offset = mc - scale * center
        h, w = ar.shape[:2]
        parts[name] = np.array(Image.fromarray(ar).transform(
            (w, h), Image.Transform.AFFINE, (1 / scale, 0, -offset[0] / scale, 0, 1 / scale, -offset[1] / scale),
            Image.Resampling.BICUBIC))
        entry.update({'targetWidth': width, 'resize': scale})


# --- Fix #7 ----------------------------------------------------------------------------

def gaussian_regularize(values) -> list[float]:
    x = np.arange(-BLINK_RADIUS, BLINK_RADIUS + 1)
    kernel = np.exp(-x * x / (2 * BLINK_SIGMA * BLINK_SIGMA))
    kernel /= kernel.sum()
    padded = np.pad(np.asarray(values, float), BLINK_RADIUS, mode='edge')
    return [float(v) for v in np.convolve(padded, kernel, mode='valid')]


def regularize_blinks(context: FixContext, original):
    """Wrap ``blink_profiles(parts, anchors)``.

    Upstream sets ``anchors[eyeL|eyeR]['blink']`` to the ``make_profile`` dict
    (``upper``/``lower``/``closed`` are equal-length float lists sampled on
    ``u0..u1``; ``lower >= upper + 1``) and returns None; the dict is written
    verbatim to ``rig-overrides.json`` ``anchorOverrides``. Only the three
    trajectories change; length, keys and the lower/upper invariant are kept.
    """
    def wrapped(parts, anchors):
        result = original(parts, anchors)
        for key in ('eyeL', 'eyeR'):
            profile = anchors[key]['blink']
            before = {name: list(profile[name]) for name in BLINK_CURVES}
            smoothed = {name: gaussian_regularize(before[name]) for name in BLINK_CURVES}
            clamped = 0
            for i, (top, bottom) in enumerate(zip(smoothed['upper'], smoothed['lower'])):
                if bottom < top + 1:
                    smoothed['lower'][i] = top + 1
                    clamped += 1
            profile.update(smoothed)
            shift = max(abs(a - b) for name in BLINK_CURVES for a, b in zip(before[name], smoothed[name]))
            context.record['blinkRegularization']['eyes'][key] = {
                'original': before, 'smoothed': smoothed, 'maxRigCoordinateShiftPx': shift,
                'lowerUpperInvariantClamps': clamped}
        return result
    return wrapped


# --- Installation ----------------------------------------------------------------------

def install(module, context: FixContext) -> None:
    """Patch the loaded upstream builder module in place.

    Every patched name is resolved as a module global at call time by upstream
    ``build``/``add_expressions``/``blink_profiles``, so assigning the module
    attribute replaces it for the whole build.
    """
    original_neutral = module.neutral_features
    original_expressions = module.add_expressions

    def neutral_features(raw, source, geometry):
        # ``raw['face']`` is already narrowed to the faceComponentPoint component here.
        face = _bounds(raw['face'][:, :, 3] > 8)
        measured = context.geometry['measurements']['face']
        # Geometry measures face alpha > 20 while upstream selects alpha > 8, so a soft
        # skin edge legitimately extends a few pixels beyond the measured bounds.
        pad = max(4, round(0.03 * (measured[2] - measured[0])))
        if face is None or not (face[0] >= measured[0] - pad and face[1] >= measured[1] - pad
                                and face[2] <= measured[2] + pad and face[3] <= measured[3] + pad):
            raise ValueError('Selected own face component is empty or outside measured face bounds')
        context.face_mask = raw['face'][:, :, 3] > 8
        context.record['mouthExpressions'].update({
            'faceSource': 'selected own face component (faceComponentPoint)', 'measuredFaceBounds': face,
            'geometryFaceBounds': list(measured)})
        return original_neutral(raw, source, geometry)

    def add_expressions(d, raw, source, parts, anchors):
        # Occluders come from this pose's own decomposition (already framed and
        # multiplied by source alpha by upstream build), registered to source space.
        shape = source.shape[:2]
        front = raw['front hair'][:, :, 3] > LID_OCCLUDER_ALPHA if 'front hair' in raw else np.zeros(shape, bool)
        occluders = front.copy()
        for name, layer in raw.items():
            if name.startswith('eyebrow'):
                occluders |= layer[:, :, 3] > LID_OCCLUDER_ALPHA
        context.front_hair = front
        context.lid_occluders = _dilate(occluders, 3)
        context.panel_index = 0
        context.reference = None
        # Panel 1 exactly as upstream builds it (same atlas, split, LANCZOS resize and
        # canvas paste), so registration can estimate the shared scale on it first.
        request = module.read(d / 'plan/own-expression-request.json')
        atlas = Image.open(d / 'raw/own-expression-G0.png').convert('RGBA')
        w, h = atlas.size
        crop = request['source_canvas_crop']
        panel = atlas.crop((round(w / 2), 0, round(2 * w / 2), h)).resize(
            (crop[2] - crop[0], crop[3] - crop[1]), Image.Resampling.LANCZOS)
        full = Image.new('RGBA', (shape[1], shape[0]))
        full.paste(panel, crop[:2])
        context.reference_panel = np.array(full)
        context.panel_crop = [int(v) for v in crop]
        return original_expressions(d, raw, source, parts, anchors)

    module.neutral_features = neutral_features
    module.add_expressions = add_expressions
    module.register_panel = lambda source, panel, nose_bounds: similarity_register(context, source, panel, nose_bounds)
    module.rebuild_lid = lambda panel, a, side: contrast_lid(context, panel, a, side)
    module.add_mouth_expressions = (lambda panels, parts, anchors, bounds_by_name:
                                    scaled_mouths(context, module, panels, parts, anchors, bounds_by_name))
    module.blink_profiles = regularize_blinks(context, module.blink_profiles)
