#!/usr/bin/env python3
"""Author independent pose illustrations and own-source expression sheets with local Qwen.

poses:       copy the approved authority exactly for authority.sourcePose and
             generate every other pose as a new complete two-image Qwen
             illustration (image1 = authority, image2 = authority headCrop).
expressions: for each pose, edit a two-panel sheet of its OWN matted head
             (image1) with its own head crop as reference (image2).
             --supersede-rejected archives a published sheet that
             raw/expression-review.json rejects (moved into
             raw/rejected/<sha256[:16]>/, never deleted) and generates a new
             attempt with seed seeds.expressions + pose index + 1000 * N,
             N = number of archived rejections.
Everything comes from RUN/run.json; nothing is overwritten.
"""
from __future__ import annotations

import argparse
import io
import json
import os
import sys
import uuid
from pathlib import Path
from typing import Any

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_config import (RunConfigError, load_json, load_run, replace_json,  # noqa: E402
                        sha256_bytes, sha256_file, write_new, write_new_json)
import qwen_local as qwen  # noqa: E402

ROUND = 'v1'
POSE_RESOLUTION = 1280
EXPRESSION_RESOLUTION = 768
REVIEW_NAME = 'expression-review.json'
REJECTED_DIR = 'rejected'
ARCHIVE_MANIFEST = 'superseded.json'
REGENERATION_SEED_STRIDE = 1000
REVIEW_SOURCES = ('visual-review', 'build-registration')


def image_info(data: bytes) -> dict[str, Any]:
    with Image.open(io.BytesIO(data)) as image:
        image.verify()
    with Image.open(io.BytesIO(data)) as image:
        return {'format': image.format, 'width': image.width, 'height': image.height, 'mode': image.mode}


def checked_png(path: Path, expected_hash: str | None = None) -> tuple[bytes, dict[str, Any]]:
    data = path.read_bytes()
    info = image_info(data)
    if info['format'] != 'PNG' or (expected_hash is not None and sha256_bytes(data) != expected_hash):
        raise ValueError(f'PNG or SHA-256 mismatch: {path}')
    return data, info


def png_bytes(image: Image.Image) -> bytes:
    with io.BytesIO() as buffer:
        image.save(buffer, format='PNG')
        return buffer.getvalue()


def seed_for(run, key: str, pose: str) -> int:
    seed = run.seeds[key] + run.poses.index(pose)
    if not 0 <= seed < 2 ** 64:
        raise ValueError(f'Unsigned 64-bit seed overflow for {pose}')
    return seed


def authority(run) -> tuple[Path, bytes, str]:
    path, data = run.checked_input('authority')
    _, info = checked_png(path)
    if (info['width'], info['height']) != (run.canvas, run.canvas):
        raise RunConfigError(f'Authority must be {run.canvas}x{run.canvas}: {path} is {info["width"]}x{info["height"]}')
    return path, data, sha256_bytes(data)


def authority_record(path: Path, digest: str) -> dict[str, str]:
    return {'path': str(path), 'sha256': digest}


def pose_inputs(run, pose: str, path: Path, digest: str, head_hash: str) -> dict[str, Any]:
    if pose == run.authority['sourcePose']:
        return {'approvedRaw': str(path), 'sha256': digest}
    return {'image1': {'path': str(path), 'sha256': digest},
            'image2': {'source': str(path), 'sourceSha256': digest,
                       'crop': list(run.authority['headCrop']), 'sha256': head_hash}}


def finished_pose(run, pose: str, raw_path: Path, provenance_path: Path, inputs: dict[str, Any],
                  authority_hash: str) -> dict[str, str] | None:
    if not raw_path.exists() and not provenance_path.exists():
        return None
    if not raw_path.exists() or not provenance_path.exists():
        raise RuntimeError(f'Incomplete published source for {pose}: {raw_path}, {provenance_path}; inspect preserved attempts')
    data, info = checked_png(raw_path)
    record = load_json(provenance_path)
    if (record.get('pose') != pose or record.get('status') != 'success' or record.get('rawSha256') != sha256_bytes(data)
            or record.get('sourceInputs') != inputs or (info['width'], info['height']) != (run.canvas, run.canvas)):
        raise RuntimeError(f'Published source or provenance failed verification for {pose}')
    if pose != run.authority['sourcePose']:
        proof = load_json(Path(record['attemptProvenance']))
        if (proof.get('status') != 'success' or proof.get('raw', {}).get('sha256') != sha256_bytes(data)
                or not proof.get('terminalSuccess') or proof.get('inputRoles', [{}])[0].get('sha256') != authority_hash):
            raise RuntimeError(f'Original generation attempt failed verification for {pose}')
    return {'id': pose, 'path': str(raw_path), 'sha256': sha256_bytes(data), 'provenance': str(provenance_path)}


def authority_provenance(path: Path) -> dict[str, str] | None:
    sibling = path.with_name(path.name + '.provenance.json')
    if not sibling.is_file():
        return None
    return {'path': str(sibling), 'sha256': sha256_file(sibling)}


def poses(args: argparse.Namespace) -> None:
    run = load_run(args.run)
    qwen.configure(run.endpoints['qwen'])
    authority_path, original, authority_hash = authority(run)
    head_crop = tuple(run.authority['headCrop'])
    source_pose = run.authority['sourcePose']
    with Image.open(io.BytesIO(original)) as image:
        head = png_bytes(image.crop(head_crop))
    head_hash = sha256_bytes(head)
    selected = run.select(args.poses)
    runner = Path(__file__)
    generation = run.root / 'generation'
    results: list[dict[str, str]] = []
    for pose in selected:
        raw_path = generation / f'{pose}-{ROUND}.png'
        provenance_path = raw_path.with_name(raw_path.name + '.provenance.json')
        inputs = pose_inputs(run, pose, authority_path, authority_hash, head_hash)
        existing = finished_pose(run, pose, raw_path, provenance_path, inputs, authority_hash)
        if existing:
            results.append(existing)
            print(json.dumps({'pose': pose, 'status': 'retained-verified', 'sha256': existing['sha256']}), flush=True)
            continue
        if pose == source_pose:
            record = {'version': 1, 'pose': pose, 'status': 'success',
                      'method': 'exact approved authority copy',
                      'note': 'NOT newly generated; byte-identical copy of run.json authority',
                      'rawSha256': authority_hash, 'sourceInputs': inputs,
                      'runConfig': str(run.root / 'run.json'), 'runConfigAuthority': dict(run.authority),
                      'authorityProvenance': authority_provenance(authority_path),
                      'dimensions': [run.canvas, run.canvas]}
            write_new(raw_path, original)
        else:
            qwen.check_previous_attempts(generation / 'attempts', f'{pose}-{ROUND}-*', raw_path, pose)
            intent = run.prompts['poseIntents'][pose]
            prompt = run.prompts['pose'].replace('{intent}', intent)
            seed = seed_for(run, 'poses', pose)
            attempt = generation / 'attempts' / f'{pose}-{ROUND}-{uuid.uuid4().hex}'
            raw, job = qwen.generate_job('poses', pose, original, head, prompt, seed, POSE_RESOLUTION,
                                         attempt, run.root.name, runner)
            raw_hash = sha256_bytes(raw)
            if raw_hash == authority_hash or raw_hash in {item['sha256'] for item in results}:
                raise RuntimeError(f'Pose {pose} duplicates an existing source; raw attempt preserved at {attempt}')
            record = {'version': 1, 'pose': pose, 'status': 'success',
                      'method': 'new complete local two-image Qwen illustration',
                      'rawSha256': raw_hash, 'dimensions': [run.canvas, run.canvas], 'sourceInputs': inputs,
                      'headCrop': list(head_crop), 'attemptProvenance': str(attempt / 'provenance.json'),
                      'promptId': job['promptId'], 'intent': intent, 'prompt': prompt, 'seed': seed,
                      'resolution': POSE_RESOLUTION, 'endpoint': job['endpoint'], 'provider': job['provider'],
                      'models': job['models']}
            write_new(raw_path, raw)
        write_new_json(provenance_path, record)
        result = finished_pose(run, pose, raw_path, provenance_path, inputs, authority_hash)
        results.append(result)
        print(json.dumps({'pose': pose, 'status': 'done', 'sha256': result['sha256'], 'path': result['path']}), flush=True)

    index_path = run.root / 'source-index.json'
    by_id = {item['id']: item for item in results}
    expected_authority = authority_record(authority_path, authority_hash)
    if index_path.exists():
        prior = load_json(index_path)
        if prior.get('version') != 1 or prior.get('authority') != expected_authority:
            raise RuntimeError('Existing source index has different authority')
        for item in prior.get('poses', []):
            pose = item['id']
            expected = pose_inputs(run, pose, authority_path, authority_hash, head_hash)
            verified = finished_pose(run, pose, Path(item['path']), Path(item['provenance']), expected, authority_hash)
            if verified != item or (pose in by_id and by_id[pose] != item):
                raise RuntimeError(f'Existing source index entry changed for {pose}')
            by_id[pose] = item
    if len({item['sha256'] for item in by_id.values()}) != len(by_id):
        raise RuntimeError('Distinct poses have identical raw image hashes')
    index = {'version': 1, 'poses': [by_id[pose] for pose in run.poses if pose in by_id],
             'authority': expected_authority}
    replace_json(index_path, index)
    print(json.dumps({'status': 'source-index-written', 'path': str(index_path), 'poses': len(by_id)}), flush=True)


def sheet_check(sheet_size: tuple[int, int], info: dict[str, Any]) -> dict[str, Any]:
    """Require exactly the TextEncodeQwenImage21 canvas for the input sheet and an even width.

    The input sheet is two copies of the own face crop, so its aspect is
    2 * crop width / crop height (not 2:1 for a rectangular crop). Qwen renders at
    that aspect with both sides rounded to multiples of 32; an even width splits
    the sheet into two equal panels.
    """
    sheet_width, sheet_height = sheet_size
    expected = qwen.expected_canvas(sheet_width, sheet_height, EXPRESSION_RESOLUTION)
    width, height = info['width'], info['height']
    input_ratio = sheet_width / sheet_height
    ratio = width / height
    return {'inputSheet': [sheet_width, sheet_height], 'inputRatio': input_ratio,
            'expectedCanvas': list(expected), 'width': width, 'height': height, 'ratio': ratio,
            'relativeDeviationFromInput': abs(ratio - input_ratio) / input_ratio,
            'evenWidth': width % 2 == 0, 'pass': (width, height) == expected and width % 2 == 0}


def expression_seed(run, pose: str, regeneration: int) -> int:
    if len(run.poses) >= REGENERATION_SEED_STRIDE:
        raise ValueError('Too many poses for distinct regeneration seeds')
    seed = seed_for(run, 'expressions', pose) + REGENERATION_SEED_STRIDE * regeneration
    if seed >= 2 ** 64:
        raise ValueError(f'Unsigned 64-bit seed overflow for {pose} regeneration {regeneration}')
    return seed


def archived_rejections(raw_dir: Path) -> list[dict[str, Any]]:
    """Archive manifests under raw/rejected/, ordered by rejection number 1..N."""
    root = raw_dir / REJECTED_DIR
    if not root.exists():
        return []
    manifests = []
    for entry in sorted(root.iterdir()):
        manifest_path = entry / ARCHIVE_MANIFEST
        if not entry.is_dir() or not manifest_path.is_file():
            raise RuntimeError(f'{entry} is not a rejected-sheet archive ({ARCHIVE_MANIFEST} missing); inspect it by hand')
        manifest = load_json(manifest_path)
        if entry.name != str(manifest.get('expressionSheetSha256', ''))[:16]:
            raise RuntimeError(f'Archive manifest does not describe its directory: {manifest_path}')
        manifests.append(manifest)
    manifests.sort(key=lambda item: item.get('rejectionNumber', 0))
    if [item.get('rejectionNumber') for item in manifests] != list(range(1, len(manifests) + 1)):
        raise RuntimeError(f'Rejected-sheet archives in {root} are not numbered 1..{len(manifests)}')
    return manifests


def attempt_marker_exists(path: Path, record: dict[str, Any]) -> bool:
    """True when the attempt already holds a matching rejection; refuse a conflicting one."""
    if not path.exists():
        return False
    existing = load_json(path)
    if (existing.get('decision') != 'rejected' or existing.get('promptId') != record['promptId']
            or existing.get('sha256') != record['sha256']):
        raise RuntimeError(f'{path} conflicts with the expression rejection; inspect by hand')
    return True


def complete_archive(manifest: dict[str, Any]) -> None:
    """Finish (or confirm) an archival: rename every still-in-place file into the archive.

    Renames only into destinations that do not exist; nothing is deleted. Also
    records the rejection beside the superseded Qwen attempt (``visual-review.json``)
    so ``qwen_local.check_previous_attempts`` accepts that preserved attempt.
    """
    for move in manifest['moves']:
        origin, destination = Path(move['from']), Path(move['to'])
        if destination.exists():
            if sha256_file(destination) != move['sha256']:
                raise RuntimeError(f'Archived file changed: {destination}')
            continue
        if not origin.is_file() or sha256_file(origin) != move['sha256']:
            raise RuntimeError(f'Cannot complete archival of {origin}: missing or changed; inspect {destination.parent}')
        os.rename(origin, destination)
    marker = manifest['attemptReview']
    if not attempt_marker_exists(Path(marker['path']), marker['record']):
        write_new_json(Path(marker['path']), marker['record'])


def rejection_for(review_path: Path, sheet_sha: str) -> dict[str, Any]:
    review = load_json(review_path)
    problems = []
    if review.get('decision') != 'rejected':
        problems.append('decision must be "rejected"')
    if review.get('sha256') != sheet_sha:
        problems.append('sha256 must equal the published own-expression-G0.png')
    if not isinstance(review.get('reason'), str) or not review['reason'].strip():
        problems.append('reason is required')
    source = review.get('source', 'visual-review')
    if source not in REVIEW_SOURCES:
        problems.append(f'source must be one of {", ".join(REVIEW_SOURCES)}')
    if source == 'build-registration' and not (isinstance(review.get('error'), str) and review['error'].strip()):
        problems.append('a build-registration rejection must quote the build-rigs.py error')
    if problems:
        raise RuntimeError(f'{review_path} is not a valid rejection: ' + '; '.join(problems))
    return review


def supersede(pose: str, raw_dir: Path, review_path: Path, review: dict[str, Any], result_path: Path,
              request_path: Path, request: dict[str, Any], sheet_sha: str, number: int) -> dict[str, Any]:
    """Move the rejected sheet, its request and the review into a new raw/rejected/<sha[:16]>/."""
    proof_path = Path(request['attemptProvenance'])
    proof = load_json(proof_path)
    archive = raw_dir / REJECTED_DIR / sheet_sha[:16]
    moves = [{'from': str(path), 'to': str(archive / path.name), 'sha256': sha256_file(path)}
             for path in (review_path, request_path, result_path)]
    source = review.get('source', 'visual-review')
    marker = proof_path.parent / 'visual-review.json'
    marker_record = {'version': 1, 'decision': 'rejected', 'promptId': proof['promptId'], 'sha256': sheet_sha,
                     'reason': review['reason'], 'source': source, 'review': str(archive / review_path.name),
                     'reviewSha256': moves[0]['sha256'],
                     'recordedBy': 'generate-sources.py expressions --supersede-rejected'}
    attempt_marker_exists(marker, marker_record)
    manifest = {'version': 1, 'pose': pose, 'rejectionNumber': number, 'archivedAt': qwen.utc_now(),
                'expressionSheetSha256': sheet_sha, 'promptId': proof['promptId'], 'seed': request.get('seed'),
                'attemptProvenance': str(proof_path), 'reason': review['reason'], 'source': source,
                'error': review.get('error'), 'review': review, 'moves': moves,
                'attemptReview': {'path': str(marker), 'record': marker_record},
                'method': 'rename into a new archive directory; nothing deleted or overwritten'}
    (raw_dir / REJECTED_DIR).mkdir(exist_ok=True)
    archive.mkdir()
    write_new_json(archive / ARCHIVE_MANIFEST, manifest)
    complete_archive(manifest)
    return manifest


def verified_expression(pose: str, request_path: Path, result_path: Path, crop: list[int], source: bytes,
                        master_path: Path, sheet_bytes: bytes, face_bytes: bytes, sheet_size: tuple[int, int],
                        target_path: Path, head_path: Path, seed: int) -> tuple[bytes, dict[str, Any]]:
    request = load_json(request_path)
    result, result_info = checked_png(result_path, request['expressionSheetSha256'])
    if (request['source_canvas_crop'] != crop or request['source_master_sha256'] != sha256_bytes(source)
            or request['inputSheetSha256'] != sha256_bytes(sheet_bytes)
            or request['ownHeadReferenceSha256'] != sha256_bytes(face_bytes)
            or request['source_master_path'] != str(master_path) or request['status'] != 'success'):
        raise RuntimeError(f'Expression inputs changed for {pose}')
    if sha256_file(target_path) != sha256_bytes(sheet_bytes) or sha256_file(head_path) != sha256_bytes(face_bytes):
        raise RuntimeError(f'Expression input image changed for {pose}')
    check = sheet_check(sheet_size, result_info)
    if not check['pass']:
        raise RuntimeError(f'Published expression sheet for {pose} is {check["width"]}x{check["height"]}, '
                           f'not the {check["expectedCanvas"][0]}x{check["expectedCanvas"][1]} canvas of its '
                           f'{sheet_size[0]}x{sheet_size[1]} input sheet with an even width')
    if request.get('seed') != seed:
        raise RuntimeError(f'Published expression seed for {pose} does not match its archived rejection count')
    proof = load_json(Path(request['attemptProvenance']))
    if proof.get('status') != 'success' or proof.get('raw', {}).get('sha256') != sha256_bytes(result) or not proof.get('terminalSuccess'):
        raise RuntimeError(f'Expression inference history failed verification for {pose}')
    return result, request


def expressions(args: argparse.Namespace) -> None:
    run = load_run(args.run)
    qwen.configure(run.endpoints['qwen'])
    authority(run)
    prompt = run.prompts['expression']
    runner = Path(__file__)
    for pose in run.select(args.poses):
        base = run.pose_dir(pose)
        plan_path = base / 'plan' / 'expression-plan.json'
        plan = load_json(plan_path)
        crop = plan['source_canvas_crop']
        if not isinstance(crop, list) or len(crop) != 4 or any(type(value) is not int for value in crop):
            raise ValueError(f'Invalid source_canvas_crop: {plan_path}')
        master_path = base / 'inputs' / 'master.png'
        source, info = checked_png(master_path, plan['source_master_sha256'])
        if not (0 <= crop[0] < crop[2] <= info['width'] and 0 <= crop[1] < crop[3] <= info['height']):
            raise ValueError(f'Crop outside source master: {plan_path}')
        with Image.open(io.BytesIO(source)) as image:
            face = image.convert('RGBA').crop(crop)
            sheet = Image.new('RGBA', (face.width * 2, face.height))
            sheet.paste(face, (0, 0))
            sheet.paste(face, (face.width, 0))
            sheet_size = sheet.size
            sheet_bytes = png_bytes(sheet)
            face_bytes = png_bytes(face)
        raw_dir = base / 'raw'
        target_path = raw_dir / 'own-expression-edit-target.png'
        head_path = raw_dir / 'own-expression-head-reference.png'
        result_path = raw_dir / 'own-expression-G0.png'
        request_path = base / 'plan' / 'own-expression-request.json'
        review_path = raw_dir / REVIEW_NAME
        archives = archived_rejections(raw_dir)
        if archives:
            complete_archive(archives[-1])
        if result_path.exists() or request_path.exists():
            if not result_path.exists() or not request_path.exists():
                raise RuntimeError(f'Incomplete expression for {pose}; inspect attempts and {raw_dir / REJECTED_DIR}')
            result, request = verified_expression(pose, request_path, result_path, crop, source, master_path,
                                                  sheet_bytes, face_bytes, sheet_size, target_path, head_path,
                                                  expression_seed(run, pose, len(archives)))
            sheet_sha = sha256_bytes(result)
            if not review_path.exists():
                print(json.dumps({'pose': pose, 'status': 'retained-verified-own-expression', 'sha256': sheet_sha}), flush=True)
                continue
            review = rejection_for(review_path, sheet_sha)
            if not args.supersede_rejected:
                raise RuntimeError(f'Published expression sheet for {pose} is rejected by {review_path}; '
                                   'rerun with --supersede-rejected to archive it and generate a new attempt')
            archived = supersede(pose, raw_dir, review_path, review, result_path, request_path, request,
                                 sheet_sha, len(archives) + 1)
            archives.append(archived)
            print(json.dumps({'pose': pose, 'status': 'archived-rejected-own-expression', 'sha256': sheet_sha,
                              'rejectionNumber': archived['rejectionNumber'],
                              'archive': str(Path(archived['moves'][0]['to']).parent)}), flush=True)
        elif review_path.exists():
            raise RuntimeError(f'{review_path} exists without a published expression sheet to reject; inspect it')
        qwen.check_previous_attempts(raw_dir / 'expression-attempts', f'{pose}-*', result_path, pose)
        if target_path.exists() and sha256_file(target_path) != sha256_bytes(sheet_bytes):
            raise RuntimeError(f'Previous expression input sheet differs for {pose}')
        if head_path.exists() and sha256_file(head_path) != sha256_bytes(face_bytes):
            raise RuntimeError(f'Previous expression head crop differs for {pose}')
        expected_width, _ = qwen.expected_canvas(sheet_size[0], sheet_size[1], EXPRESSION_RESOLUTION)
        if expected_width % 2:
            raise RuntimeError(f'Expression canvas for {pose} would have an odd width; widen source_canvas_crop')
        if not target_path.exists():
            write_new(target_path, sheet_bytes)
        if not head_path.exists():
            write_new(head_path, face_bytes)
        regeneration = len(archives)
        seed = expression_seed(run, pose, regeneration)
        supersedes = None
        if archives:
            latest = archives[-1]
            supersedes = {'rejectionNumber': latest['rejectionNumber'],
                          'archive': str(Path(latest['moves'][0]['to']).parent),
                          'expressionSheetSha256': latest['expressionSheetSha256'], 'promptId': latest['promptId'],
                          'seed': latest['seed'], 'reason': latest['reason'], 'source': latest['source']}
        attempt = raw_dir / 'expression-attempts' / f'{pose}-{uuid.uuid4().hex}'
        raw, job = qwen.generate_job('expressions', pose, sheet_bytes, face_bytes, prompt, seed,
                                     EXPRESSION_RESOLUTION, attempt, run.root.name, runner)
        output_info = image_info(raw)
        check = sheet_check(sheet_size, output_info)
        if not check['pass']:
            raise RuntimeError(f'Expression sheet for {pose} is {output_info["width"]}x{output_info["height"]}, not the '
                               f'{check["expectedCanvas"][0]}x{check["expectedCanvas"][1]} canvas of its input sheet with '
                               f'an even width; raw attempt preserved at {attempt}')
        if sha256_bytes(raw) in {item['expressionSheetSha256'] for item in archives}:
            raise RuntimeError(f'Expression sheet for {pose} repeats a rejected sheet; raw attempt preserved at {attempt}')
        request = {'version': 1, 'pose': pose, 'status': 'success', 'source_canvas_crop': crop,
                   'source_master_sha256': sha256_bytes(source), 'source_master_path': str(master_path),
                   'inputSheet': str(target_path), 'inputSheetSha256': sha256_bytes(sheet_bytes),
                   'ownHeadReference': str(head_path), 'ownHeadReferenceSha256': sha256_bytes(face_bytes),
                   'panel0': 'Own-pose natural closed eyelids and small open mouth',
                   'panel1': 'Own-pose visibly open eyes and gentle smile',
                   'provider': job['provider'], 'endpoint': job['endpoint'], 'models': job['models'], 'prompt': prompt,
                   'seed': seed, 'regeneration': regeneration, 'supersedes': supersedes,
                   'steps': qwen.STEPS, 'resolution': EXPRESSION_RESOLUTION, 'promptId': job['promptId'],
                   'attemptProvenance': str(attempt / 'provenance.json'),
                   'expressionSheetSha256': sha256_bytes(raw),
                   'expressionSheetDimensions': [output_info['width'], output_info['height']],
                   'expressionSheetCheck': check}
        write_new(result_path, raw)
        write_new_json(request_path, request)
        print(json.dumps({'pose': pose, 'status': 'done-own-expression', 'sha256': sha256_bytes(raw),
                          'path': str(result_path), 'request': str(request_path)}), flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    for command in ('poses', 'expressions'):
        mode = sub.add_parser(command)
        mode.add_argument('--run', required=True, help='run directory containing run.json')
        mode.add_argument('--poses', default='all', help='all or comma-separated semantic pose IDs')
        if command == 'expressions':
            mode.add_argument('--supersede-rejected', action='store_true',
                              help=f'archive a published sheet rejected by raw/{REVIEW_NAME} into '
                                   f'raw/{REJECTED_DIR}/<sha256[:16]>/ and generate a new attempt')
    args = parser.parse_args()
    try:
        (poses if args.command == 'poses' else expressions)(args)
    except BaseException as error:
        print(json.dumps({'status': 'failed', 'type': type(error).__name__, 'error': str(error)}, ensure_ascii=False),
              file=sys.stderr, flush=True)
        return 130 if isinstance(error, KeyboardInterrupt) else 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
