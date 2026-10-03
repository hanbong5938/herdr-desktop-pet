#!/usr/bin/env python3
"""Correct the authority face identity with one local Qwen-Image-2.1 two-reference edit.

image1 = authority.headCrop of the approved authority, image2 =
identityReference.faceCrop (face identity only). The generated crop is resized
back to the head crop and composited ONLY inside a feathered ellipse inscribed
in authority.faceEditRect; every pixel outside that support and the whole alpha
channel are proven identical to the authority. The operator reviews
identity/corrected-<seed>-review.png and then manually points run.json
authority at the accepted corrected file.
"""
from __future__ import annotations

import argparse
import io
import json
import math
import sys
import uuid
from pathlib import Path
from typing import Any

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_config import (RunConfigError, load_run, sha256_bytes, sha256_file,  # noqa: E402
                        write_new, write_new_json)
import qwen_local as qwen  # noqa: E402

MASK_THRESHOLD = 16
REVIEW_PANEL_HEIGHT = 512
REVIEW_GAP = 16
REVIEW_LABEL_HEIGHT = 24
METHOD = ('Local Qwen-Image-2.1 two-reference face edit: image1 = authority headCrop, image2 = identity reference '
          'faceCrop (face identity only). Generated crop resized (LANCZOS) back to the head crop and composited only '
          'inside the feathered faceEditRect ellipse; outside-support RGBA and full alpha proven unchanged. '
          'Not manually painted and not a donor-face transplant.')


def png_bytes(image: Image.Image) -> bytes:
    with io.BytesIO() as buffer:
        image.save(buffer, format='PNG')
        return buffer.getvalue()


def decode(data: bytes) -> Image.Image:
    with Image.open(io.BytesIO(data)) as image:
        image.load()
        return image.copy()


def box(value: Any, label: str) -> tuple[int, int, int, int]:
    if (not isinstance(value, list) or len(value) != 4 or any(type(item) is not int for item in value)
            or value[0] < 0 or value[1] < 0 or value[2] <= value[0] or value[3] <= value[1]):
        raise RunConfigError(f'{label} must be four integers [x0, y0, x1, y1] describing a non-empty rectangle')
    return value[0], value[1], value[2], value[3]


def inside(inner: tuple[int, int, int, int], outer: tuple[int, int, int, int]) -> bool:
    return outer[0] <= inner[0] and outer[1] <= inner[1] and inner[2] <= outer[2] and inner[3] <= outer[3]


def face_mask(size: tuple[int, int], rect: tuple[int, int, int, int], feather: float) -> Image.Image:
    """Feathered ellipse inscribed in ``rect`` (inset by the feather), clipped to ``rect``."""
    inset = math.ceil(feather)
    x0, y0, x1, y1 = rect
    ellipse = (x0 + inset, y0 + inset, x1 - 1 - inset, y1 - 1 - inset)
    if ellipse[2] <= ellipse[0] or ellipse[3] <= ellipse[1]:
        raise RunConfigError(f'authority.faceEditRect {list(rect)} is too small for feather {feather}px')
    mask = Image.new('L', size, 0)
    ImageDraw.Draw(mask).ellipse(ellipse, fill=255)
    mask = mask.point(lambda value: 255 if value > MASK_THRESHOLD else 0)
    if feather > 0:
        mask = mask.filter(ImageFilter.GaussianBlur(feather))
    envelope = Image.new('L', size, 0)
    envelope.paste(255, rect)
    return ImageChops.multiply(mask, envelope)


def flatten(image: Image.Image) -> Image.Image:
    rgba = image.convert('RGBA')
    background = Image.new('RGB', rgba.size, (255, 255, 255))
    background.paste(rgba, mask=rgba.getchannel('A'))
    return background


def review_sheet(panels: list[tuple[str, Image.Image]]) -> Image.Image:
    scaled = []
    for label, image in panels:
        flat = flatten(image)
        width = max(1, round(flat.width * REVIEW_PANEL_HEIGHT / flat.height))
        scaled.append((label, flat.resize((width, REVIEW_PANEL_HEIGHT), Image.Resampling.LANCZOS)))
    total = sum(image.width for _, image in scaled) + REVIEW_GAP * (len(scaled) + 1)
    sheet = Image.new('RGB', (total, REVIEW_PANEL_HEIGHT + REVIEW_LABEL_HEIGHT + REVIEW_GAP * 2), (240, 240, 240))
    draw = ImageDraw.Draw(sheet)
    left = REVIEW_GAP
    for label, image in scaled:
        draw.text((left, REVIEW_GAP // 2), label, fill=(0, 0, 0))
        sheet.paste(image, (left, REVIEW_GAP + REVIEW_LABEL_HEIGHT))
        left += image.width + REVIEW_GAP
    return sheet


def correct(args: argparse.Namespace) -> dict[str, Any]:
    run = load_run(args.run)
    qwen.configure(run.endpoints['qwen'])
    seed = run.seeds['identity'] if args.seed is None else args.seed
    if not 0 <= seed < 2 ** 64:
        raise ValueError('--seed must be an unsigned 64-bit integer')
    if not 0 <= args.feather <= 64:
        raise ValueError('--feather must be between 0 and 64 pixels')
    authority = run.authority
    head_crop = box(authority['headCrop'], 'authority.headCrop')
    qwen.expected_canvas(head_crop[2] - head_crop[0], head_crop[3] - head_crop[1], args.resolution)
    if authority.get('faceEditRect') is None:
        raise RunConfigError('run.json authority.faceEditRect [x0, y0, x1, y1] (authority canvas pixels, inside '
                             'authority.headCrop) is required: it bounds the only pixels identity correction may change')
    face_rect = box(authority['faceEditRect'], 'authority.faceEditRect')
    if not inside(face_rect, head_crop):
        raise RunConfigError(f'authority.faceEditRect {list(face_rect)} must lie inside authority.headCrop {list(head_crop)}')

    authority_path, authority_data = run.checked_input('authority')
    source = decode(authority_data)
    if qwen.image_details(authority_data, 'authority')['format'] != 'PNG' or source.size != (run.canvas, run.canvas):
        raise RunConfigError(f'Authority must be a {run.canvas}x{run.canvas} PNG: {authority_path}')
    if source.mode not in ('RGB', 'RGBA'):
        source = source.convert('RGBA')
    identity_path, identity_data = run.checked_input('identityReference')
    identity = decode(identity_data)
    face_crop = box(run.identity['faceCrop'], 'identityReference.faceCrop')
    if not inside(face_crop, (0, 0, identity.width, identity.height)):
        raise RunConfigError(f'identityReference.faceCrop {list(face_crop)} lies outside {identity.width}x{identity.height}')
    if identity.mode not in ('RGB', 'RGBA'):
        identity = identity.convert('RGBA')

    out_dir = run.root / 'identity'
    corrected_path = out_dir / f'corrected-{seed}.png'
    provenance_path = corrected_path.with_name(corrected_path.name + '.provenance.json')
    review_path = out_dir / f'corrected-{seed}-review.png'
    for destination in (corrected_path, provenance_path, review_path):
        if destination.exists():
            raise FileExistsError(f'Refusing to overwrite {destination}; choose another --seed')
    qwen.check_previous_attempts(out_dir / 'attempts', f'{seed}-*', corrected_path, f'identity seed {seed}')

    before_head = source.crop(head_crop)
    identity_face = identity.crop(face_crop)
    image1 = png_bytes(before_head)
    image2 = png_bytes(identity_face)
    mask = face_mask(source.size, face_rect, args.feather)
    mask_crop = mask.crop(head_crop)
    local_rect = [face_rect[0] - head_crop[0], face_rect[1] - head_crop[1],
                  face_rect[2] - head_crop[0], face_rect[3] - head_crop[1]]
    source_prompt = run.prompts['identityCorrect']
    formatted = source_prompt.replace('{faceEditRect}', json.dumps(local_rect))
    missing_tags = [tag for tag in ('<image1>', '<image2>') if tag not in formatted]
    final_prompt = ''.join(tag + '\n' for tag in missing_tags) + formatted

    attempt = out_dir / 'attempts' / f'{seed}-{uuid.uuid4().hex}'
    raw, job = qwen.generate_job('identity', authority['sourcePose'], image1, image2, final_prompt, seed,
                                 args.resolution, attempt, run.root.name, Path(__file__))
    write_new(attempt / 'face-edit-mask.png', png_bytes(mask))
    generated_raw = decode(raw)
    expected = job['expectedCanvas']
    if generated_raw.size != (expected['width'], expected['height']):
        raise RuntimeError(f'Qwen output {generated_raw.size} differs from expected canvas {expected}; attempt preserved at {attempt}')
    crop_size = (head_crop[2] - head_crop[0], head_crop[3] - head_crop[1])
    generated = generated_raw.convert('RGB').resize(crop_size, Image.Resampling.LANCZOS)

    blended = Image.composite(generated, before_head.convert('RGB'), mask_crop)
    if source.mode == 'RGBA':
        blended = blended.convert('RGBA')
        blended.putalpha(before_head.getchannel('A'))
    output = source.copy()
    output.paste(blended, (head_crop[0], head_crop[1]))
    corrected = png_bytes(output)

    # Prove preservation on the exact encoded bytes that will be published.
    original_rgba = np.array(source.convert('RGBA'))
    result_rgba = np.array(decode(corrected).convert('RGBA'))
    support = np.array(mask) > 0
    if original_rgba.shape != result_rgba.shape:
        raise RuntimeError(f'Corrected image shape changed; attempt preserved at {attempt}')
    changed = np.any(original_rgba != result_rgba, axis=2)
    outside_equal = bool(np.array_equal(original_rgba[~support], result_rgba[~support]))
    alpha_equal = bool(np.array_equal(original_rgba[:, :, 3], result_rgba[:, :, 3]))
    changed_pixels = int(changed.sum())
    changed_outside = int((changed & ~support).sum())
    if not outside_equal or not alpha_equal or changed_outside or changed_pixels == 0:
        raise RuntimeError(f'Preservation invariant failed (outsideEqual={outside_equal}, alphaEqual={alpha_equal}, '
                           f'changedOutside={changed_outside}, changed={changed_pixels}); attempt preserved at {attempt}')

    review = png_bytes(review_sheet([
        ('identity reference faceCrop', identity_face),
        ('authority headCrop before', before_head),
        ('after (Qwen, masked composite)', output.crop(head_crop)),
    ]))
    corrected_hash = sha256_bytes(corrected)
    write_new(corrected_path, corrected)
    write_new(review_path, review)
    mask_png = png_bytes(mask)
    snippet = dict(authority)
    snippet['path'] = corrected_path.relative_to(run.root).as_posix()
    snippet['sha256'] = corrected_hash
    record = {
        'version': 1, 'kind': 'identity-correction', 'status': 'success', 'createdAt': qwen.utc_now(),
        'method': METHOD, 'manualFacePainting': False, 'donorFaceTransplant': False,
        'authority': {'path': str(authority_path), 'sha256': sha256_bytes(authority_data),
                      'mode': source.mode, 'dimensions': [source.width, source.height]},
        'identityReference': {'path': str(identity_path), 'sha256': sha256_bytes(identity_data),
                              'dimensions': [identity.width, identity.height]},
        'crop': list(head_crop), 'canonicalCrop': list(face_crop),
        'faceEditRect': list(face_rect), 'faceEditRectInCrop': local_rect,
        'inputs': {'image1': {'role': 'authority headCrop', 'sha256': sha256_bytes(image1), 'crop': list(head_crop)},
                   'image2': {'role': 'identity reference faceCrop (face identity only)', 'sha256': sha256_bytes(image2),
                              'crop': list(face_crop)}},
        'mask': {'path': str(attempt / 'face-edit-mask.png'), 'sha256': sha256_bytes(mask_png),
                 'shape': 'ellipse inscribed in faceEditRect, inset by ceil(feather)', 'threshold': MASK_THRESHOLD,
                 'gaussianFeather': args.feather, 'envelope': 'faceEditRect', 'supportPixels': int(support.sum())},
        'sourcePrompt': source_prompt, 'finalPrompt': final_prompt, 'seed': seed, 'resolution': args.resolution,
        'steps': qwen.STEPS, 'endpoint': job['endpoint'], 'provider': job['provider'], 'models': job['models'],
        'promptId': job['promptId'], 'attempt': str(attempt),
        'attemptProvenance': str(attempt / 'provenance.json'),
        'attemptProvenanceSha256': sha256_file(attempt / 'provenance.json'),
        'raw': {'path': str(attempt / 'raw.png'), 'sha256': sha256_bytes(raw),
                'dimensions': [generated_raw.width, generated_raw.height]},
        'resize': {'to': list(crop_size), 'method': 'LANCZOS', 'colour': 'RGB'},
        'composite': 'Image.composite(generated RGB, authority RGB, feathered mask) inside headCrop; authority alpha restored',
        'proof': {'verifiedOn': 'decoded corrected PNG bytes vs decoded authority (RGBA)',
                  'outsideMaskRGBAExactlyUnchanged': outside_equal, 'alphaExactlyUnchanged': alpha_equal,
                  'changedPixels': changed_pixels, 'changedPixelsOutsideSupport': changed_outside},
        'output': {'path': str(corrected_path), 'sha256': corrected_hash, 'mode': output.mode,
                   'dimensions': [output.width, output.height]},
        'review': {'path': str(review_path), 'sha256': sha256_bytes(review),
                   'panels': ['identity reference faceCrop', 'authority headCrop before', 'after']},
        'artisticIdentityAccepted': False,
        'adoption': 'Operator reviews the review sheet, then manually replaces run.json authority with runJsonAuthority.',
        'runJsonAuthority': snippet,
        'rights': qwen.RIGHTS,
    }
    write_new_json(provenance_path, record)
    return {'status': 'success', 'corrected': str(corrected_path), 'sha256': corrected_hash,
            'provenance': str(provenance_path), 'review': str(review_path), 'attempt': str(attempt),
            'promptId': job['promptId'], 'changedPixels': changed_pixels,
            'outsideMaskRGBAExactlyUnchanged': outside_equal, 'alphaExactlyUnchanged': alpha_equal,
            'runJsonAuthority': snippet}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--run', required=True, help='run directory containing run.json')
    parser.add_argument('--seed', type=int, help='Qwen seed (default: run.json seeds.identity)')
    parser.add_argument('--resolution', type=int, default=768, help='TextEncodeQwenImage21 resolution (default: 768)')
    parser.add_argument('--feather', type=float, default=6.0, help='Gaussian feather radius in pixels (default: 6)')
    args = parser.parse_args()
    try:
        summary = correct(args)
    except BaseException as error:
        print(json.dumps({'status': 'failed', 'type': type(error).__name__, 'error': str(error)}, ensure_ascii=False),
              file=sys.stderr, flush=True)
        return 130 if isinstance(error, KeyboardInterrupt) else 1
    print(json.dumps(summary, indent=2, ensure_ascii=False), flush=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
