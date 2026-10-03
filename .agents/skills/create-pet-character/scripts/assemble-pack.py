#!/usr/bin/env python3
"""Stage a herdr v5 ten-model pack from a run; cut over the builtin only after native QA.

Commands:
  stage     --run RUN [--output DIR]          (default DIR = RUN/pack; never overwritten)
  cutover   --run RUN --pack DIR --builtin PATH --native BIN
  thumbnail --run RUN --capture PNG --output PNG [--size WxH] [--replace]

``stage`` copies every pose's own ``native-models/<pose>/`` model, overrides and
motion, generates the v5 rig entry from the ``tools.motionReference`` pack's
entry template, carries that pack's license/owner records verbatim, prepends a
run notice to its attribution files, writes ``source-record.json`` and the
updated manifest, then regenerates the inventory with the repository's
``tools/character-pack.py inventory`` and requires native
``character-pack.py validate`` (the native executable) to pass before the
staged directory appears.

``cutover`` re-validates the staged pack natively FIRST; only then does it
tar-back up the current builtin under RUN, swap the directory and verify the
new builtin is byte-identical to the staged pack (and natively validates to
the same content digest), restoring the previous builtin on any failure.

``thumbnail`` derives a menu thumbnail from an actual native preview PNG that
``review-native.py`` recorded (review.json), alpha-bound cropped and aspect-fit;
an existing output is only replaced with ``--replace`` (previous copy kept).
"""
from __future__ import annotations

import argparse
import datetime
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_config import (POSES, Run, RunConfigError, encode_json, load_json,  # noqa: E402
                        load_run, sha256_bytes, sha256_file, write_new, write_new_json)

MOTION_PHASES = {'idle', 'running', 'waiting', 'unknown'}
MOTION_REACTIONS = {'head_tap', 'body_tap', 'pet', 'completion_observed'}
POSE_FILES = (('model.psd', '{pose}.psd', 'psd'),
              ('rig-overrides.json', '{pose}-overrides.json', 'overrides'),
              ('motion.json', '{pose}-motion.json', 'motion'))
CARRIED_KINDS = ('license', 'attribution', 'source')
SOURCE_RECORD = 'source-record.json'
MAX_TEXT_PAYLOAD = 4 * 1024 * 1024  # character-pack.py ordinary payload bound


class AssembleError(RuntimeError):
    pass


def require(condition: Any, message: str) -> None:
    if not condition:
        raise AssembleError(message)


def utc_stamp() -> str:
    return datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')


def regular(path: Path) -> Path:
    try:
        meta = path.lstat()
    except OSError as error:
        raise AssembleError(f'Missing required file: {path}') from error
    require(stat.S_ISREG(meta.st_mode), f'Expected a regular non-symlink file: {path}')
    return path


def real_dir(path: Path, label: str) -> Path:
    require(path.is_dir() and not path.is_symlink(), f'{label} must be a real directory: {path}')
    return path


def shown(run: Run, path: Path) -> str:
    try:
        return path.resolve().relative_to(run.root).as_posix()
    except ValueError:
        return str(path.resolve())


def file_record(run: Run, path: Path, expected: str | None = None) -> dict[str, Any]:
    digest = sha256_file(regular(path))
    if expected is not None:
        require(digest == expected, f'SHA-256 mismatch: {path}')
    return {'path': shown(run, path), 'sha256': digest, 'bytes': path.stat().st_size}


def json_record(run: Run, path: Path) -> dict[str, Any]:
    return {'file': file_record(run, path), 'record': load_json(path)}


def load(path: Path) -> Any:
    return load_json(regular(path))


# ---------------------------------------------------------------- repository tool

def pack_tool(run: Run) -> Path:
    """``tools/character-pack.py`` of the repository that owns ``tools.adapter``."""
    repo = run.tool('adapter').resolve().parent.parent
    tool = repo / 'tools' / 'character-pack.py'
    require(tool.is_file(), f'Repository pack tool missing: {tool}')
    return tool


def character_pack(run: Run, *arguments: str) -> dict[str, Any]:
    command = [sys.executable, str(pack_tool(run)), *arguments]
    result = subprocess.run(command, capture_output=True, text=True, check=False)
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    require(result.returncode == 0 and lines,
            f'{" ".join(command)} failed ({result.returncode}): {result.stderr.strip() or result.stdout.strip()}')
    try:
        parsed = json.loads(lines[-1])
    except json.JSONDecodeError as error:
        raise AssembleError(f'{" ".join(command)} returned non-JSON output') from error
    require(isinstance(parsed, dict), f'{" ".join(command)} returned a non-object')
    return parsed


def inventory(run: Run, pack: Path, check: bool) -> dict[str, Any]:
    arguments = ['inventory', '--path', str(pack)] + (['--check'] if check else [])
    result = character_pack(run, *arguments)
    require(result.get('ok') is True and result.get('command') == 'inventory', f'Inventory failed: {result}')
    return result


def native_validate(run: Run, pack: Path, native: Path) -> dict[str, Any]:
    """Full native validation via ``character-pack.py validate --native BIN``."""
    require(native.is_file() and os.access(native, os.X_OK), f'Native executable unavailable: {native}')
    manifest = load(pack / 'manifest.json')
    result = character_pack(run, 'validate', '--path', str(pack), '--native', str(native))
    require(result.get('command') == 'validate' and result.get('backend') == 'rig' and
            result.get('prepared') is True and result.get('width') == manifest['width'] and
            result.get('height') == manifest['height'] and isinstance(result.get('content_digest'), str),
            f'Native validation did not confirm a prepared {manifest["width"]}x{manifest["height"]} rig: {result}')
    return result


def configured_native(run: Run) -> Path:
    require(run.data['tools'].get('native'), 'run.json tools.native (herdr-desktop-pet executable) is required')
    return run.tool('native').resolve()


# ---------------------------------------------------------------- reference pack

def reference_pack(run: Run) -> tuple[Path, dict[str, Any], dict[str, Any]]:
    reference = real_dir(run.tool('motionReference').resolve(), 'tools.motionReference')
    manifest = load(reference / 'manifest.json')
    require(manifest.get('format') == 'herdr.character' and manifest.get('version') == 5 and
            manifest.get('render_mode') == 'rig', 'motionReference must be a herdr v5 rig pack')
    # Owner, license and attribution records belong to one character;
    # they are reused only for the same character.
    require(manifest.get('name') == run.character['label'],
            f'motionReference pack is for {manifest.get("name")!r}, not {run.character["label"]!r}; '
            'its license/attribution/owner records cannot be reused for another character')
    entry = load(reference / manifest['entry'])
    require(entry.get('version') == 2 and entry.get('initial') in POSES and
            {item.get('id') for item in entry.get('models', [])} == set(POSES) and
            entry.get('bindings') == {pose: pose for pose in POSES},
            'motionReference rig entry must route all ten semantic IDs to own models')
    return reference, manifest, entry


# ---------------------------------------------------------------- provenance chain

def collect_source(record: Any, models: set[str], providers: set[str]) -> None:
    if not isinstance(record, dict):
        return
    if isinstance(record.get('provider'), str) and record['provider']:
        providers.add(record['provider'])
    listed = record.get('models')
    values = listed.values() if isinstance(listed, dict) else listed if isinstance(listed, list) else ()
    models.update(value for value in values if isinstance(value, str) and value)


def checked_attempt(run: Run, value: Any, digest: str, label: str) -> dict[str, Any]:
    require(isinstance(value, str) and value, f'{label}: attempt provenance path missing')
    path = run.path(value)
    data = load(path)
    require(data.get('status') == 'success' and data.get('terminalSuccess') and
            data.get('raw', {}).get('sha256') == digest, f'{label}: Qwen attempt not a terminal success for this output')
    return {'file': file_record(run, path), 'record': data}


def pose_chain(run: Run, pose: str, reference_hashes: set[str],
               models: set[str], providers: set[str]) -> dict[str, Any]:
    base = run.pose_dir(pose)
    native = run.root / 'native-models' / pose
    authority = run.authority
    raw = run.root / 'generation' / f'{pose}-v1.png'
    raw_sha = sha256_file(regular(raw))
    generation_path = raw.with_name(raw.name + '.provenance.json')
    generation = load(generation_path)
    require(generation.get('pose') == pose and generation.get('status') == 'success' and
            generation.get('rawSha256') == raw_sha, f'{pose}: generation provenance does not match {raw}')
    detail: dict[str, Any] = {
        'rawFullSource': file_record(run, raw),
        'generation': json_record(run, generation_path),
    }
    if pose == authority['sourcePose']:
        require(raw_sha == authority['sha256'], f'{pose}: source pose is not the exact run authority')
        detail['origin'] = 'exact copy of run.json authority'
        linked = generation.get('authorityProvenance')
        if isinstance(linked, dict):
            detail['authorityProvenance'] = file_record(run, run.path(linked['path']), linked['sha256'])
    else:
        require(raw_sha != authority['sha256'], f'{pose}: generated pose duplicates the authority image')
        detail['origin'] = 'complete pose illustration generated in this run'
        detail['generationAttempt'] = checked_attempt(run, generation.get('attemptProvenance'), raw_sha,
                                                      f'{pose} generation')
        collect_source(detail['generationAttempt']['record'], models, providers)
    collect_source(generation, models, providers)

    master = base / 'inputs' / 'master.png'
    master_sha = sha256_file(regular(master))
    matte_path = base / 'inputs' / 'matting.json'
    matte = load(matte_path)
    require(matte.get('inputs', {}).get('sourceSha256') == raw_sha and matte.get('masterSha256') == master_sha,
            f'{pose}: matte record is not for this raw source/master')
    model = matte['inputs'].get('model')
    if isinstance(model, str) and model:
        models.add(Path(model).name)

    decomposition_path = base / 'native' / 'generation.json'
    decomposition = load(decomposition_path)
    require(decomposition.get('sourceSha256') == master_sha, f'{pose}: decomposition has a stale source')
    assets = decomposition.get('files')
    require(isinstance(assets, dict) and assets, f'{pose}: decomposition file hashes missing')
    for filename, digest in assets.items():
        file_record(run, base / 'native' / filename, digest)

    geometry = base / 'plan' / 'geometry.json'
    plan = base / 'plan' / 'expression-plan.json'
    request_path = base / 'plan' / 'own-expression-request.json'
    atlas = base / 'raw' / 'own-expression-G0.png'
    atlas_sha = sha256_file(regular(atlas))
    request = load(request_path)
    require(load(geometry).get('sourceSha256') == master_sha and
            load(plan).get('source_master_sha256') == master_sha and
            request.get('source_master_sha256') == master_sha and request.get('status') == 'success' and
            request.get('expressionSheetSha256') == atlas_sha,
            f'{pose}: geometry/expression plan/own expression has a stale source')
    expression_attempt = checked_attempt(run, request.get('attemptProvenance'), atlas_sha, f'{pose} expression')
    collect_source(request, models, providers)
    collect_source(expression_attempt['record'], models, providers)

    outputs = {name: native / name for name, _, _ in POSE_FILES}
    psd_sha = sha256_file(regular(outputs['model.psd']))
    require(outputs['model.psd'].read_bytes()[:4] == b'8BPS', f'{pose}: native model is not a layered PSD')
    require(psd_sha not in reference_hashes, f'{pose}: native PSD is reference-pack artwork')
    conversion_path = native / 'provenance.json'
    conversion = load(conversion_path)
    require(conversion.get('outputHashes', {}).get('model.psd') == psd_sha,
            f'{pose}: native PSD differs from the adapter output')
    own_path = native / 'own-source-record.json'
    own = load(own_path)
    require(own.get('pose') == pose and own.get('sourceSha256') == master_sha and
            own.get('geometrySha256') == sha256_file(geometry) and own.get('expressionSheetSha256') == atlas_sha and
            own.get('rasterProvenanceSha256') == sha256_file(conversion_path) and own.get('oldArtworkUsed') is False,
            f'{pose}: native own-source chain mismatch')
    motion = load(outputs['motion.json'])
    require(set(motion.get('phases', {})) == MOTION_PHASES and set(motion.get('reactions', {})) == MOTION_REACTIONS,
            f'{pose}: motion lacks the complete phase/reaction set')
    load(outputs['rig-overrides.json'])

    detail.update({
        'matte': json_record(run, matte_path),
        'master': file_record(run, master),
        'decomposition': json_record(run, decomposition_path),
        'decompositionAssets': assets,
        'geometry': file_record(run, geometry),
        'expressionPlan': file_record(run, plan),
        'expressionRequest': json_record(run, request_path),
        'expressionAtlas': file_record(run, atlas),
        'expressionAttempt': {'file': expression_attempt['file']},
        'nativeConversion': json_record(run, conversion_path),
        'nativeSourceChain': json_record(run, own_path),
        'nativeOutputs': {name: file_record(run, path) for name, path in outputs.items()},
    })
    if 'generationAttempt' in detail:
        detail['generationAttempt'] = {'file': detail['generationAttempt']['file']}
    return detail


# ---------------------------------------------------------------- stage

def run_notice(run: Run, models: list[str]) -> str:
    authority, identity = run.authority, run.identity
    lines = [
        f'{run.character["label"]} — herdr v5 ten-model pack staged from run "{run.root.name}".',
        f'Authority illustration SHA256 {authority["sha256"]} is the exact "{authority["sourcePose"]}" pose and the '
        'outfit/body/style authority. Identity reference SHA256 '
        f'{identity["sha256"]} supplied face identity only.',
        'Every other pose is a complete illustration generated for this run by local image editing; each pose has '
        'its own matte, See-through layer decomposition, own expression edit, source-layered PSD and native rig '
        'conversion. No reference-pack pose pixels and no post-export alpha crop are used.',
    ]
    if models:
        lines.append('Recorded model files: ' + ', '.join(models) + '.')
    lines.append('source-record.json records the exact file chain and hashes. This notice makes no claim about '
                 'native review, publication or licenses beyond the files named in manifest.json.')
    lines.append('')
    return '\n'.join(lines)


def stage(run: Run, output: Path) -> dict[str, Any]:
    output = output.expanduser().absolute()
    require(not output.exists() and not output.is_symlink(), f'Stage output exists; never overwritten: {output}')
    real_dir(output.parent, 'Stage output parent')
    report_path = output.parent / f'{output.name}.stage.json'
    require(not report_path.exists(), f'Stage report exists; never overwritten: {report_path}')
    native = configured_native(run)
    reference, reference_manifest, reference_entry = reference_pack(run)
    authority_path, _ = run.checked_input('authority')
    identity_path, _ = run.checked_input('identityReference')
    reference_payloads = reference_manifest['payloads']
    reference_hashes = {item['sha256'] for item in reference_payloads if item['kind'] in ('psd', 'png')}

    models: set[str] = set()
    providers: set[str] = set()
    poses = {pose: pose_chain(run, pose, reference_hashes, models, providers) for pose in run.poses}
    require(len({detail['rawFullSource']['sha256'] for detail in poses.values()}) == len(POSES),
            'Duplicate full-source illustration across poses')
    require(len({detail['nativeOutputs']['model.psd']['sha256'] for detail in poses.values()}) == len(POSES),
            'Duplicate native layered model across poses')
    model_list, provider_list = sorted(models), sorted(providers)

    authority_provenance = authority_path.with_name(authority_path.name + '.provenance.json')
    request = run.data.get('request')
    approval: dict[str, Any] = {
        'basis': ('run.json request, authority and identityReference as recorded by the operator' if request else
                  'run.json authority and identityReference as selected by the operator; this pipeline records no '
                  'separate user statement'),
        'authority': dict(run.authority, file=file_record(run, authority_path)),
        'identityReference': dict(run.identity, file=file_record(run, identity_path),
                                  role='face identity only; never outfit, background or props'),
        'requiredBindings': list(POSES),
    }
    if request is not None:
        require(isinstance(request, dict) and isinstance(request.get('userStatement'), str)
                and request['userStatement'] and isinstance(request.get('date'), str),
                'run.json request needs userStatement and date')
        approval['request'] = request
    if authority_provenance.is_file():
        approval['authority']['identityCorrection'] = json_record(run, authority_provenance)
        collect_source(approval['authority']['identityCorrection']['record'], models, providers)
        model_list, provider_list = sorted(models), sorted(providers)

    notice = run_notice(run, model_list)
    carried: dict[str, dict[str, Any]] = {}
    destination = Path(tempfile.mkdtemp(prefix=f'.{output.name}-stage-', dir=str(output.parent)))
    try:
        for pose in run.poses:
            for source_name, pattern, _ in POSE_FILES:
                write_new(destination / pattern.format(pose=pose),
                          regular(run.root / 'native-models' / pose / source_name).read_bytes())
        for item in reference_payloads:
            name, kind = item['path'], item['kind']
            if kind not in CARRIED_KINDS or name == SOURCE_RECORD:
                continue
            original = regular(reference / name).read_bytes()
            require(sha256_bytes(original) == item['sha256'], f'Reference payload changed since inventory: {name}')
            if name in reference_manifest['attributions']:
                data = (notice + f'\nReference pack "{reference_manifest["id"]}" {name} (preserved verbatim, SHA256 '
                        f'{item["sha256"]}; its pose, hash and review statements describe that reference pack, '
                        'not this staged pack):\n').encode('utf-8') + original
                treatment = 'run notice prepended; reference text preserved verbatim'
            else:
                data = original
                treatment = 'verbatim'
            write_new(destination / name, data)
            carried[name] = {'kind': kind, 'referenceSha256': item['sha256'], 'treatment': treatment}
        for path in [item['path'] for item in reference_manifest['licenses']] + list(reference_manifest['attributions']):
            require(path in carried, f'Reference license/attribution not in its inventory: {path}')

        entry_name = reference_manifest['entry']
        write_new_json(destination / entry_name, {
            'version': reference_entry['version'], 'initial': reference_entry['initial'],
            'models': [{'id': pose, 'file': f'{pose}.psd', 'overrides': f'{pose}-overrides.json',
                        'motion': f'{pose}-motion.json'} for pose in run.poses],
            'bindings': {pose: pose for pose in run.poses},
        })

        corrected = 'identityCorrection' in approval['authority']
        pipeline_steps = []
        if corrected:
            pipeline_steps.append(
                'identity-correct: local Qwen-Image-2.1 two-image face edit (image1 = authority headCrop, '
                'image2 = identityReference faceCrop, face identity only), composited only inside the feathered '
                'authority.faceEditRect ellipse mask with outside-support RGBA and full alpha proven unchanged')
        pipeline_steps += [
            f'authority: exact {run.authority["sourcePose"]} source from run.json authority',
            'poses: local Qwen-Image-2.1 two-image complete pose illustrations (image1 = authority, image2 = '
            f'authority headCrop; identity carried by the {"identity-corrected " if corrected else ""}authority, '
            'identityReference not passed)',
            'matte: ISNet anime matting to a 1280-square RGBA master',
            'decomposition: See-through semantic layers per pose',
            'geometry: See-through alpha-derived face/eye/mouth geometry, reviewed per pose',
            'expressions: own-pose two-panel Qwen-Image-2.1 sheet edit (image1 = two copies of the own geometry '
            'crop, image2 = own pose head crop), similarity-registered to the own master; only closed-lid and '
            'mouth features are extracted as rig layers, no panel or face is pasted onto the source',
            'rig: DAEMONLET semantic source layers, native layered PSD conversion and retargeted motion',
        ]
        reference_record = reference / SOURCE_RECORD
        source_record = {
            'schemaVersion': 1,
            'character': run.character,
            'run': {'root': str(run.root), 'runJson': file_record(run, run.root / 'run.json'), 'snapshot': run.data},
            'approval': approval,
            'pipeline': '; '.join(pipeline_steps),
            'models': model_list,
            'providers': provider_list,
            'endpoints': run.endpoints,
            'seeds': run.seeds,
            'motionReference': {
                'path': str(reference), 'manifestId': reference_manifest['id'],
                'manifestSha256': sha256_file(reference / 'manifest.json'),
                'sourceRecordSha256': sha256_file(reference_record) if reference_record.is_file() else None,
                'role': 'rig-entry, manifest, license and attribution templates and motion reference only; '
                        'no reference pose pixels are carried',
            },
            'carriedReferenceFiles': carried,
            'licenseBoundary': ('License and owner records are carried from reference pack '
                                f'"{reference_manifest["id"]}" for the same character; they grant only what their own '
                                'text states. This run records no new owner approval, model license or publication.'),
            'noPostExportAlphaCrop': True,
            'poses': poses,
        }
        record_bytes = encode_json(source_record)
        require(len(record_bytes) <= MAX_TEXT_PAYLOAD, f'{SOURCE_RECORD} exceeds the pack payload bound')
        write_new(destination / SOURCE_RECORD, record_bytes)

        sources = {
            'method': 'image-generated',
            'description': (f'{run.character["label"]}: '
                            f'{"identity-corrected " if corrected else ""}run authority illustration as the '
                            f'{run.authority["sourcePose"]} pose plus complete pose illustrations generated in this run '
                            'from the authority and its head crop; per-pose matte, See-through decomposition, own '
                            'expression sheet features, native rig and motion. '
                            f'{SOURCE_RECORD} records the exact chain.'),
            'urls': [SOURCE_RECORD],
        }
        if provider_list:
            sources['provider'] = '; '.join(provider_list)
        if model_list:
            sources['model'] = ', '.join(model_list)
        for key in ('provider', 'model'):
            require(len(sources.get(key, '')) <= 512, f'manifest source.{key} exceeds 512 characters')
        manifest = {key: value for key, value in reference_manifest.items() if key not in ('payloads', 'source')}
        manifest['name'] = run.character['label']
        manifest['source'] = sources
        kinds = {item['path']: item['kind'] for item in reference_payloads}
        kinds.update({entry_name: 'entry', SOURCE_RECORD: 'source'})
        for pose in run.poses:
            for _, pattern, kind in POSE_FILES:
                kinds[pattern.format(pose=pose)] = kind
        manifest['payloads'] = []
        for file in sorted(destination.iterdir(), key=lambda path: path.name):
            manifest['payloads'].append({'path': file.name, 'size': file.stat().st_size,
                                         'sha256': sha256_file(file), 'kind': kinds[file.name]})
        write_new_json(destination / 'manifest.json', manifest)
        regenerated = inventory(run, destination, check=False)
        require(inventory(run, destination, check=True).get('changed') is False, 'Inventory is not stable')
        require(len(list(destination.iterdir())) == 1 + 3 * len(POSES) + 2 + len(carried),
                'Unexpected staged pack file set')
        validation = native_validate(run, destination, native)
        os.replace(destination, output)
    except BaseException:
        shutil.rmtree(destination, ignore_errors=True)
        raise
    report = {'stage': str(output), 'manifestSha256': sha256_file(output / 'manifest.json'),
              'inventory': regenerated, 'nativeValidation': validation,
              'nativeValidatedPath': str(destination)}
    write_new_json(report_path, report)
    return report


# ---------------------------------------------------------------- cutover

def pack_files(root: Path) -> dict[str, str]:
    files = {}
    for path in root.iterdir():
        files[path.name] = sha256_file(regular(path))
    return files


def cutover(run: Run, pack: Path, builtin: Path, native: Path) -> dict[str, Any]:
    pack = real_dir(pack.expanduser().absolute(), 'Staged pack')
    builtin = real_dir(builtin.expanduser().absolute(), 'Builtin pack')
    native = native.expanduser().absolute()
    # Gate: nothing below this block may touch the builtin unless the staged
    # pack's inventory is current and native validation has passed.
    require(inventory(run, pack, check=True).get('changed') is False, 'Staged pack inventory is stale')
    staged_validation = native_validate(run, pack, native)

    record = load(pack / SOURCE_RECORD)
    require(record.get('approval', {}).get('authority', {}).get('sha256') == run.authority['sha256'],
            'Staged pack does not belong to this run authority')
    # The builtin need not BE tools.motionReference (that may be a private copy);
    # it must be byte-identical to the reference the stage was assembled from:
    # same manifest bytes, and every file matching that manifest's payload inventory.
    require(sha256_file(builtin / 'manifest.json') == record['motionReference']['manifestSha256'],
            'Builtin manifest differs from the motionReference manifest recorded at stage time; refuse cutover')
    old_manifest = load(builtin / 'manifest.json')
    reference_inventory = {item['path']: item for item in old_manifest['payloads']}
    expected = {'manifest.json', *reference_inventory}
    previous = pack_files(builtin)
    require(set(previous) == expected,
            f'Builtin has unrecognized or missing files; refuse replacing: {sorted(set(previous) ^ expected)}')
    mismatched = sorted(name for name, item in reference_inventory.items()
                        if previous[name] != item['sha256'] or (builtin / name).stat().st_size != item['size'])
    require(not mismatched, f'Builtin payloads differ from the stage-time reference inventory; refuse cutover: '
                            f'{mismatched}')
    staged = pack_files(pack)

    stamp = utc_stamp()
    backup = run.root / f'builtin-backup-{stamp}.tar'
    with open(backup, 'xb') as stream, tarfile.open(fileobj=stream, mode='w') as archive:
        for name in sorted(previous):
            archive.add(str(builtin / name), arcname=name, recursive=False)
    with tarfile.open(backup, mode='r') as archive:
        archived = {}
        for member in archive.getmembers():
            require(member.isfile(), f'Backup contains a non-file member: {member.name}')
            handle = archive.extractfile(member)
            archived[member.name] = sha256_bytes(handle.read() if handle else b'')
    require(archived == previous, 'Backup tar does not match the previous builtin')

    replacement = Path(tempfile.mkdtemp(prefix=f'.{builtin.name}-replacement-', dir=str(builtin.parent)))
    retired = Path(tempfile.mkdtemp(prefix=f'.{builtin.name}-previous-', dir=str(builtin.parent)))
    retired.rmdir()
    swapped = False
    try:
        for name in sorted(staged):
            shutil.copy2(str(pack / name), str(replacement / name))
        require(pack_files(replacement) == staged, 'Replacement copy is not byte-identical to the staged pack')
        replacement.chmod(builtin.stat().st_mode & 0o777)
        os.replace(builtin, retired)
        try:
            os.replace(replacement, builtin)
        except BaseException:
            os.replace(retired, builtin)
            raise
        swapped = True
        require(pack_files(builtin) == staged, 'Builtin is not byte-identical to the staged pack after cutover')
        builtin_validation = native_validate(run, builtin, native)
        require(builtin_validation['content_digest'] == staged_validation['content_digest'],
                'Builtin native content digest differs from the staged pack')
    except BaseException:
        if swapped:
            failed = Path(tempfile.mkdtemp(prefix=f'.{builtin.name}-failed-', dir=str(builtin.parent)))
            failed.rmdir()
            os.replace(builtin, failed)
            os.replace(retired, builtin)
            shutil.rmtree(failed, ignore_errors=True)
        raise
    finally:
        if replacement.exists():
            shutil.rmtree(replacement, ignore_errors=True)
    shutil.rmtree(retired)  # verified copy retained in the backup tar
    report = {'builtin': str(builtin), 'stagedPack': str(pack), 'backup': file_record(run, backup),
              'previousFiles': previous, 'newFiles': staged,
              'stagedNativeValidation': staged_validation, 'builtinNativeValidation': builtin_validation}
    write_new_json(run.root / f'cutover-{stamp}.json', report)
    return report


# ---------------------------------------------------------------- thumbnail

def native_capture(capture: Path) -> tuple[Path, dict[str, Any], dict[str, Any]]:
    """Find the review-native.py record proving ``capture`` is a native preview."""
    review_path = capture.parent / 'review.json'
    review = load(review_path)
    digest = sha256_file(capture)
    match = [row for row in review.get('captures', [])
             if isinstance(row.get('output'), dict) and Path(row['output'].get('path', '')).resolve() == capture.resolve()
             and row['output'].get('sha256') == digest]
    require(len(match) == 1, f'{capture} is not a recorded native capture in {review_path}')
    row = match[0]
    result = row.get('native_result', {})
    require(row.get('returncode') == 0 and result.get('command') == 'preview' and result.get('backend') == 'rig' and
            Path(result.get('output', '')).resolve() == capture.resolve() and result.get('hit_overlay') is False and
            result.get('reaction') is None and result.get('bytes') == capture.stat().st_size,
            'Capture must be a native rig preview without hit overlay or reaction')
    return review_path, review, row


def parse_size(text: str) -> tuple[int, int]:
    width, _, height = text.lower().partition('x')
    try:
        size = (int(width), int(height))
    except ValueError as error:
        raise AssembleError('--size must be WIDTHxHEIGHT') from error
    require(size[0] > 0 and size[1] > 0, '--size must be positive')
    return size


def thumbnail(run: Run, capture: Path, output: Path, size_text: str | None, replace: bool) -> dict[str, Any]:
    from PIL import Image

    capture = regular(capture.expanduser().absolute())
    output = output.expanduser().absolute()
    review_path, review, row = native_capture(capture)
    pack = Path(row['native_result']['path'])
    record = load(pack / SOURCE_RECORD)
    require(record.get('approval', {}).get('authority', {}).get('sha256') == run.authority['sha256'],
            'Captured pack does not belong to this run authority')
    exists = output.exists() or output.is_symlink()
    require(not exists or replace, f'Thumbnail exists; pass --replace to replace it: {output}')
    if size_text:
        size = parse_size(size_text)
    else:
        require(exists, '--size WIDTHxHEIGHT is required when the output does not exist yet')
        with Image.open(regular(output)) as previous:
            size = previous.size
    with Image.open(capture) as image:
        require(image.format == 'PNG', 'Native capture must be a PNG')
        art = image.convert('RGBA')
    bounds = art.getchannel('A').getbbox()
    require(bounds is not None and bounds != (0, 0) + art.size, 'Capture lacks transparent art bounds')
    art = art.crop(bounds)
    art.thumbnail(size, Image.Resampling.LANCZOS)
    frame = Image.new('RGBA', size, (0, 0, 0, 0))
    frame.alpha_composite(art, ((size[0] - art.width) // 2, (size[1] - art.height) // 2))
    buffer = io.BytesIO()
    frame.save(buffer, format='PNG')
    data = buffer.getvalue()

    folder = run.root / 'thumbnails'
    folder.mkdir(exist_ok=True)
    stamp = utc_stamp()
    candidate = folder / f'thumbnail-{stamp}.png'
    write_new(candidate, data)
    proof: dict[str, Any] = {
        'method': 'alpha-bound crop of a native preview capture, aspect-fit LANCZOS, centred on transparent canvas',
        'review': file_record(run, review_path), 'reviewStatus': review.get('status'),
        'capture': dict(file_record(run, capture), name=row.get('name'), controls=row.get('controls')),
        'pack': str(pack), 'packContentDigest': row['native_result'].get('content_digest'),
        'cropBounds': list(bounds), 'size': list(size), 'candidate': file_record(run, candidate),
        'output': str(output),
    }
    if exists:
        previous_copy = folder / f'previous-{stamp}{output.suffix or ".png"}'
        write_new(previous_copy, regular(output).read_bytes())
        proof['previous'] = file_record(run, previous_copy)
    write_new_json(folder / f'thumbnail-{stamp}.provenance.json', proof)
    temporary = output.with_name(f'.{output.name}.{stamp}.tmp')
    write_new(temporary, data)
    os.replace(temporary, output)
    require(sha256_file(output) == sha256_bytes(data), 'Thumbnail output verification failed')
    return {'thumbnail': str(output), 'provenance': str(folder / f'thumbnail-{stamp}.provenance.json')}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    staging = commands.add_parser('stage', help='assemble and natively validate the staged pack')
    staging.add_argument('--run', required=True)
    staging.add_argument('--output', type=Path, help='new pack directory (default RUN/pack)')
    cut = commands.add_parser('cutover', help='replace the builtin with a natively validated staged pack')
    cut.add_argument('--run', required=True)
    cut.add_argument('--pack', type=Path, required=True)
    cut.add_argument('--builtin', type=Path, required=True)
    cut.add_argument('--native', type=Path, required=True)
    thumb = commands.add_parser('thumbnail', help='derive the menu thumbnail from a native review capture')
    thumb.add_argument('--run', required=True)
    thumb.add_argument('--capture', type=Path, required=True, help='PNG inside a review-native.py output directory')
    thumb.add_argument('--output', type=Path, required=True)
    thumb.add_argument('--size', help='WIDTHxHEIGHT (default: size of the existing output)')
    thumb.add_argument('--replace', action='store_true', help='replace an existing output (previous copy kept in RUN)')
    args = parser.parse_args()
    try:
        run = load_run(args.run)
        if args.command == 'stage':
            result = stage(run, args.output or run.root / 'pack')
        elif args.command == 'cutover':
            result = cutover(run, args.pack, args.builtin, args.native)
        else:
            result = thumbnail(run, args.capture, args.output, args.size, args.replace)
    except (AssembleError, RunConfigError, KeyError, OSError, ValueError, tarfile.TarError) as error:
        print(f'assemble-pack: {error}', file=sys.stderr)
        return 1
    print(json.dumps(result, ensure_ascii=False))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
