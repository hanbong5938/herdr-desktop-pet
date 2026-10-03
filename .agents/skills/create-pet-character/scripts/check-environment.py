"""Read-only readiness probe for the See-through (8188) and Qwen-Image-2.1 (8189) ComfyUI servers.

Run on the host that serves ComfyUI. Pass --comfy-root only when running with the
See-through ComfyUI installation's own Python; that enables its file/package checks.
Nothing is installed, downloaded, uploaded, submitted or interrupted: every HTTP
request is a GET of /system_stats, /object_info/<node> or /queue.
"""
from __future__ import annotations

import argparse
import csv
import importlib.metadata
import json
import math
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

DEPENDENCIES = Path(__file__).resolve().parent.parent / 'external-dependencies.json'


def positive_gib(value):
    capacity = float(value)
    if not math.isfinite(capacity) or capacity <= 0:
        raise ValueError('VRAM must be a positive finite GiB number')
    return capacity


def positive_seconds(value):
    seconds = float(value)
    if not math.isfinite(seconds) or seconds <= 0:
        raise ValueError('timeout must be a positive finite number of seconds')
    return seconds


def single_gpu_vram(gpu_csv):
    """nvidia-smi reports MiB; never guess which GPU ComfyUI uses on a multi-GPU host."""
    rows = list(csv.reader(gpu_csv.splitlines())) if gpu_csv else []
    if len(rows) != 1 or len(rows[0]) != 2:
        return None
    try:
        return positive_gib(rows[0][1]) / 1024
    except (TypeError, ValueError):
        return None


def get_json(base, path, timeout):
    request = urllib.request.Request(base.rstrip('/') + path, method='GET')
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode('utf-8'))


def probe(base, path, timeout, errors, label):
    try:
        return get_json(base, path, timeout)
    except (OSError, ValueError) as exc:
        errors.append(f'{label}: GET {base.rstrip("/")}{path} failed: {exc}')
        return None


def node_info(base, name, timeout, errors, label):
    response = probe(base, '/object_info/' + urllib.parse.quote(name, safe=''), timeout, errors, label)
    info = response.get(name) if isinstance(response, dict) else None
    if response is not None and not isinstance(info, dict):
        errors.append(f'{label}: node {name} is not registered')
    return info if isinstance(info, dict) else None


def model_choices(info, input_name):
    try:
        choices = info['input']['required'][input_name][0]
    except (KeyError, IndexError, TypeError):
        return None
    return choices if isinstance(choices, list) else None


def stats_device(stats):
    """Return (GiB, device) for exactly one CUDA or MPS device reported by ComfyUI."""
    devices = stats.get('devices') if isinstance(stats, dict) else None
    usable = [d for d in devices or [] if isinstance(d, dict) and d.get('type') in ('cuda', 'mps')]
    if len(usable) != 1:
        return None, None
    total = usable[0].get('vram_total')
    if not isinstance(total, (int, float)) or not math.isfinite(total) or total <= 0:
        return None, None
    return total / 1024 ** 3, {'name': usable[0].get('name'), 'type': usable[0].get('type')}


def queue_counts(queue):
    if not isinstance(queue, dict):
        return None
    return {'running': len(queue.get('queue_running') or []), 'pending': len(queue.get('queue_pending') or [])}


def check_seethrough(url, dependencies, timeout, errors):
    label = 'See-through ' + url
    stats = probe(url, '/system_stats', timeout, errors, label)
    if stats is None:
        return {'url': url, 'reachable': False, 'device': None, 'deviceCapacityGiB': None}
    missing = [name for name in dependencies['seeThrough']['requiredNodes']
               if node_info(url, name, timeout, errors, label) is None]
    capacity, device = stats_device(stats)
    return {'url': url, 'reachable': stats is not None,
            'comfyuiVersion': (stats or {}).get('system', {}).get('comfyui_version'),
            'device': device, 'deviceCapacityGiB': capacity, 'missingNodes': missing,
            'queue': queue_counts(probe(url, '/queue', timeout, errors, label))}


def check_qwen(url, dependencies, timeout, errors):
    label = 'Qwen-Image-2.1 ' + url
    qwen = dependencies['qwenImage21']
    stats = probe(url, '/system_stats', timeout, errors, label)
    if stats is None:
        return {'url': url, 'reachable': False, 'expectedComfyuiCommit': qwen['comfyui']['commit']}
    infos = {name: node_info(url, name, timeout, errors, label) for name in qwen['requiredNodes']}
    models = {}
    for entry in qwen['modelFiles']:
        info = infos.get(entry['loader'])
        choices = model_choices(info, entry['input']) if info else None
        present = bool(choices) and entry['file'] in choices
        models[entry['file']] = present
        if info and not present:
            errors.append(f'{label}: {entry["loader"]}.{entry["input"]} does not list {entry["file"]}')
    clip = infos.get('CLIPLoader')
    clip_types = model_choices(clip, 'type') if clip else None
    if clip and 'qwen_image' not in (clip_types or []):
        errors.append(f'{label}: CLIPLoader does not support type qwen_image')
    encoder = infos.get('TextEncodeQwenImage21')
    autogrow = None
    if encoder:
        try:
            kind, options = encoder['input']['required']['images']
            names = options['template']['names']
            image_type = options['template']['input']['required']['image'][0]
            autogrow = kind == 'COMFY_AUTOGROW_V3' and 'image_1' in names and image_type == 'IMAGE'
        except (KeyError, TypeError, ValueError):
            autogrow = False
        if not autogrow:
            errors.append(f'{label}: TextEncodeQwenImage21 does not expose IMAGE autogrow input images.image_1')
    _, device = stats_device(stats)
    return {'url': url, 'reachable': stats is not None,
            'comfyuiVersion': (stats or {}).get('system', {}).get('comfyui_version'),
            'expectedComfyuiCommit': qwen['comfyui']['commit'], 'device': device,
            'missingNodes': [name for name, info in infos.items() if info is None],
            'modelFiles': models, 'clipTypeQwenImage': None if clip is None else 'qwen_image' in (clip_types or []),
            'imageAutogrow': autogrow,
            'queue': queue_counts(probe(url, '/queue', timeout, errors, label))}


def check_comfy_root(comfy_root, dependencies, errors):
    root = Path(comfy_root).expanduser().resolve()
    for path in ['main.py', 'custom_nodes/ComfyUI-See-through/nodes.py']:
        if not (root / path).is_file():
            errors.append('Missing ' + str(root / path))
    versions = {}
    for package in ['torch', 'diffusers', 'accelerate']:
        try:
            versions[package] = importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError:
            errors.append('Missing Python package in ' + sys.executable + ': ' + package)
    version = tuple(int(n) for n in re.findall('[0-9]+', versions.get('diffusers', '0.0.0'))[:3])
    minimum = dependencies['seeThrough']['diffusersMinimum']
    if version < tuple(int(n) for n in minimum.split('.')):
        errors.append('Supported See-through profile requires diffusers >= ' + minimum)
    return {'comfyRoot': str(root), 'python': sys.executable, 'versions': versions}


def build_report(args):
    dependencies = json.loads(DEPENDENCIES.read_text(encoding='utf-8'))
    errors = []
    seethrough = check_seethrough(args.seethrough_url, dependencies, args.timeout, errors)
    qwen = check_qwen(args.qwen_url, dependencies, args.timeout, errors)
    local = check_comfy_root(args.comfy_root, dependencies, errors) if args.comfy_root else None

    gpu = None
    if args.vram_gib is not None:
        capacity, source = args.vram_gib, 'explicit capacity'
    elif seethrough['deviceCapacityGiB'] is not None:
        capacity = seethrough['deviceCapacityGiB']
        source = 'See-through /system_stats single ' + seethrough['device']['type'] + ' device'
    else:
        try:
            gpu = subprocess.run(['nvidia-smi', '--query-gpu=name,memory.total', '--format=csv,noheader,nounits'],
                                 capture_output=True, text=True, timeout=15, check=True).stdout.strip()
        except (OSError, subprocess.SubprocessError):
            gpu = None
        capacity = single_gpu_vram(gpu)
        source = 'single NVIDIA GPU' if capacity is not None else 'unavailable'
    group_offload = None
    if capacity is None:
        errors.append('Cannot determine the See-through GPU total memory; pass --vram-gib with the capacity of the device used by ComfyUI')
    else:
        group_offload = capacity <= dependencies['profile']['groupOffload']['maximumVramGiB']
    if local is not None:
        try:
            from diffusers import DiffusionPipeline
            if group_offload and not hasattr(DiffusionPipeline, 'enable_group_offload'):
                errors.append('This diffusers runtime has no DiffusionPipeline.enable_group_offload')
        except Exception as exc:
            errors.append('Cannot load diffusers: ' + str(exc))

    license_items = ([('ComfyUI', dependencies['comfyui']), ('ComfyUI-See-through', dependencies['seeThrough'])]
                     + [(item['id'], item) for item in dependencies['models']]
                     + [('Qwen-Image-2.1', dependencies['qwenImage21']),
                        (dependencies['matting']['model']['id'], dependencies['matting']['model'])])
    license_entries = [{'id': name, **item.get('licenseEvidence', {'status': 'unverified'}), 'installedRevisionChecked': False}
                       for name, item in license_items]
    steps = []
    if seethrough['device'] and seethrough['device']['type'] == 'mps' and args.vram_gib is None:
        steps.append('MPS host: pass this capacity explicitly to the See-through runner; it only auto-detects one CUDA device.')
    steps.append('Verify both See-through loaders actually enable group offload in the first inference log.' if group_offload
                 else 'Verify both See-through loaders use group_offload=false in the first inference log.' if group_offload is False
                 else 'Confirm the See-through device capacity before running inference.')
    steps.append('Run one Qwen two-reference edit on an idle queue before batch generation; a node/model listing is not inference proof.')
    return {'python': sys.executable, 'local': local, 'seeThrough': seethrough, 'qwenImage21': qwen,
            'technicalReadiness': 'not-ready' if errors else 'environment-ready',
            'licenseReview': {'status': 'pending', 'installedEnvironment': 'unverified', 'entries': license_entries,
                              'acknowledgementGrantsRights': False},
            'gpu': gpu, 'vramGiB': capacity, 'groupOffload': group_offload, 'vramSource': source,
            'errors': errors, 'inferenceRun': False, 'next': steps}


def main():
    dependencies = json.loads(DEPENDENCIES.read_text(encoding='utf-8'))
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--comfy-root', help='See-through ComfyUI root; run this script with that installation\'s Python')
    parser.add_argument('--seethrough-url', default=dependencies['seeThrough']['defaultUrl'])
    parser.add_argument('--qwen-url', default=dependencies['qwenImage21']['comfyui']['defaultUrl'])
    parser.add_argument('--vram-gib', type=positive_gib,
                        help='Total memory of the device used by the See-through ComfyUI; required when detection is ambiguous')
    parser.add_argument('--timeout', type=positive_seconds, default=10.0, help='Per-request timeout in seconds')
    report = build_report(parser.parse_args())
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 1 if report['errors'] else 0


if __name__ == '__main__':
    sys.exit(main())
