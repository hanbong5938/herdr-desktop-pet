"""Private local ComfyUI client and Qwen-Image-2.1 two-reference edit graph.

Importable only. Callers MUST call ``configure(run.endpoints['qwen'])`` before
any request; nothing here touches the network at import time. Every helper
refuses to upload or submit while another ComfyUI job is running or pending and
never cancels, interrupts or deletes other work.
"""
from __future__ import annotations

import io
import json
import posixpath
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_config import encode_json, sha256_bytes, sha256_file, write_new  # noqa: E402

BASE_URL: str | None = None
DEADLINE_SECONDS = 1200
POLL_SECONDS = 2
MAX_RESOLUTION = 2048
STEPS = 25
SAVE_IMAGE_NODE = '19'
COMFYUI_COMMIT = '8d534945ebd53cff61e8def81757c6a6c1b9cf2d'
MODELS = {
    'unet': 'qwen_image_2.1_bf16.safetensors',
    'textEncoder': 'qwen3vl_8b_bf16.safetensors',
    'vae': 'qwen_image_2.1_vae_bf16.safetensors',
}
MODEL_LICENSE = {
    'name': 'Qwen RESEARCH LICENSE AGREEMENT',
    'url': 'https://huggingface.co/Qwen/Qwen-Image-2.1/blob/main/LICENSE',
    'use': 'private non-commercial research/evaluation only; commercial use is not authorized by this runner',
}
PROVIDER = 'private local ComfyUI Qwen-Image-2.1 BF16'
RIGHTS = 'Private local user-approved authoring; publication and redistribution not inferred.'
ALPHA_HANDLING = {
    'loadNode': 'LoadImage',
    'loadOutputs': {'image': 0, 'invertedAlphaMask': 1},
    'joinNode': 'JoinImageWithAlpha',
    'maskPolarity': 'JoinImageWithAlpha applies 1-mask internally, restoring source alpha',
    'conditioning': 'TextEncodeQwenImage21 receives the reconstructed RGBA IMAGE for image_1 and image_2',
    'sourceAndUploadBytesModified': False,
}


def configure(endpoint: str) -> None:
    """Point the client at ``run.endpoints['qwen']``."""
    global BASE_URL
    if not isinstance(endpoint, str) or not endpoint.startswith(('http://', 'https://')):
        raise ValueError(f'Qwen endpoint must be an http(s) URL: {endpoint!r}')
    BASE_URL = endpoint.rstrip('/')


def base_url() -> str:
    if BASE_URL is None:
        raise RuntimeError('qwen_local.configure(run.endpoints["qwen"]) was not called')
    return BASE_URL


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z')


def json_bytes(value: Any) -> bytes:
    return encode_json(value)


def request(path: str, data: bytes | None = None, headers: dict[str, str] | None = None,
            timeout: int = 120) -> tuple[bytes, dict[str, str]]:
    req = urllib.request.Request(base_url() + path, data=data, headers=headers or {})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return response.read(), dict(response.headers.items())
    except urllib.error.HTTPError as error:
        detail = error.read().decode('utf-8', 'replace')
        raise RuntimeError(f'ComfyUI HTTP {error.code} for {path}: {detail}') from error
    except urllib.error.URLError as error:
        raise RuntimeError(f'ComfyUI request failed for {path}: {error.reason}') from error


def request_json(path: str, value: Any = None, timeout: int = 120) -> Any:
    if value is None:
        body = None
        headers: dict[str, str] = {}
    else:
        body = json.dumps(value, ensure_ascii=False).encode('utf-8')
        headers = {'Content-Type': 'application/json'}
    raw, _ = request(path, body, headers, timeout)
    try:
        return json.loads(raw)
    except json.JSONDecodeError as error:
        raise RuntimeError(f'ComfyUI returned non-JSON data for {path}') from error


def multipart(fields: dict[str, Any], file_field: str, filename: str, content: bytes,
              content_type: str) -> tuple[bytes, str]:
    boundary = '----pet-character-' + uuid.uuid4().hex
    chunks = []
    for name, value in fields.items():
        chunks.extend([
            f'--{boundary}\r\n'.encode(),
            f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode(),
            str(value).encode('utf-8'),
            b'\r\n',
        ])
    chunks.extend([
        f'--{boundary}\r\n'.encode(),
        f'Content-Disposition: form-data; name="{file_field}"; filename="{filename}"\r\n'.encode(),
        f'Content-Type: {content_type}\r\n\r\n'.encode(),
        content,
        b'\r\n',
        f'--{boundary}--\r\n'.encode(),
    ])
    return b''.join(chunks), f'multipart/form-data; boundary={boundary}'


def expected_canvas(width: int, height: int, resolution: int) -> tuple[int, int]:
    """Generation canvas chosen by TextEncodeQwenImage21 for an image_1 of ``width``x``height``."""
    if resolution <= 0 or resolution > MAX_RESOLUTION or resolution % 32:
        raise ValueError(f'resolution must be a positive multiple of 32 no greater than {MAX_RESOLUTION}')
    ratio = width / height
    canvas_width = max(32, round(((resolution * resolution * ratio) ** 0.5) / 32) * 32)
    canvas_height = max(32, round(((resolution * resolution / ratio) ** 0.5) / 32) * 32)
    return canvas_width, canvas_height


def image_details(data: bytes, label: str = 'image') -> dict[str, Any]:
    try:
        with Image.open(io.BytesIO(data)) as image:
            image.verify()
        with Image.open(io.BytesIO(data)) as image:
            return {'format': image.format, 'mode': image.mode, 'width': image.width, 'height': image.height}
    except Exception as error:
        raise ValueError(f'{label} is not a valid image: {error}') from error


def queue_counts(queue: Any) -> tuple[int, int]:
    running = queue.get('queue_running', []) if isinstance(queue, dict) else None
    pending = queue.get('queue_pending', []) if isinstance(queue, dict) else None
    if not isinstance(running, list) or not isinstance(pending, list):
        raise RuntimeError('ComfyUI /queue response has an unexpected shape')
    return len(running), len(pending)


def require_idle_queue() -> Any:
    queue = request_json('/queue')
    running, pending = queue_counts(queue)
    if running or pending:
        raise RuntimeError(f'ComfyUI queue is not empty ({running} running, {pending} pending); nothing was submitted')
    return queue


def model_choices(node_info: Any, input_name: str) -> list[Any]:
    try:
        choices = node_info['input']['required'][input_name][0]
    except (KeyError, IndexError, TypeError) as error:
        raise RuntimeError(f'ComfyUI object_info has no {input_name!r} model choices') from error
    if not isinstance(choices, list):
        raise RuntimeError(f'ComfyUI object_info has malformed {input_name!r} model choices')
    return choices


def require_runtime_models() -> dict[str, Any]:
    node_names = (
        'LoadImage',
        'JoinImageWithAlpha',
        'UNETLoader',
        'CLIPLoader',
        'VAELoader',
        'TextEncodeQwenImage21',
        'QwenImage21Cache',
        'KSampler',
        'VAEDecode',
        'SaveImage',
    )
    info = {}
    for node_name in node_names:
        response = request_json('/object_info/' + urllib.parse.quote(node_name, safe=''))
        node_info = response.get(node_name) if isinstance(response, dict) else None
        if not isinstance(node_info, dict):
            raise RuntimeError(f'ComfyUI does not register required core node {node_name}')
        info[node_name] = node_info

    checks = (
        ('UNETLoader', 'unet_name', MODELS['unet']),
        ('CLIPLoader', 'clip_name', MODELS['textEncoder']),
        ('VAELoader', 'vae_name', MODELS['vae']),
    )
    for node_name, input_name, filename in checks:
        if filename not in model_choices(info[node_name], input_name):
            raise RuntimeError(f'ComfyUI {node_name} does not list required model {filename}')

    if 'qwen_image' not in model_choices(info['CLIPLoader'], 'type'):
        raise RuntimeError('ComfyUI CLIPLoader does not support type qwen_image')
    try:
        autogrow = info['TextEncodeQwenImage21']['input']['required']['images']
        autogrow_type, autogrow_options = autogrow
        autogrow_names = autogrow_options['template']['names']
        autogrow_image_type = autogrow_options['template']['input']['required']['image'][0]
    except (KeyError, TypeError, ValueError) as error:
        raise RuntimeError('ComfyUI TextEncodeQwenImage21 has an unexpected images autogrow schema') from error
    if autogrow_type != 'COMFY_AUTOGROW_V3' or 'image_1' not in autogrow_names or autogrow_image_type != 'IMAGE':
        raise RuntimeError('ComfyUI TextEncodeQwenImage21 does not expose IMAGE autogrow input images.image_1')
    return {
        'requiredNodes': list(node_names),
        'modelsPresent': dict(MODELS),
        'clipType': 'qwen_image',
        'textEncodeAutogrowInputs': ['images.image_1', 'images.image_2'],
        'schemaBasis': {
            'comfyUiCommit': COMFYUI_COMMIT,
            'coreNode': 'TextEncodeQwenImage21',
            'officialTemplate': 'image_qwen_image_2_1_image_edit.json',
        },
    }


def terminal_execution_error(status: Any) -> str | None:
    if not isinstance(status, dict):
        return None
    status_text = str(status.get('status_str', '')).lower()
    if status_text in {'error', 'failed', 'failure', 'interrupted'}:
        return f'status_str={status_text}'
    for message in status.get('messages', []):
        if isinstance(message, (list, tuple)) and message:
            event = message[0]
        elif isinstance(message, dict):
            event = message.get('type')
        else:
            continue
        if event in {'execution_error', 'execution_interrupted'}:
            return str(event)
    return None


def workflow(upload_name: str, reference_upload_name: str | None, prompt: str, seed: int,
             resolution: int, steps: int, prefix: str) -> dict[str, Any]:
    """Proven Qwen-Image-2.1 edit graph; both images keep alpha via JoinImageWithAlpha."""
    graph: dict[str, Any] = {
        '1': {'class_type': 'LoadImage', 'inputs': {'image': upload_name}},
        '10': {'class_type': 'JoinImageWithAlpha', 'inputs': {'image': ['1', 0], 'alpha': ['1', 1]}},
        '2': {'class_type': 'UNETLoader', 'inputs': {'unet_name': MODELS['unet'], 'weight_dtype': 'default'}},
        '3': {'class_type': 'CLIPLoader', 'inputs': {'clip_name': MODELS['textEncoder'], 'type': 'qwen_image', 'device': 'default'}},
        '4': {'class_type': 'VAELoader', 'inputs': {'vae_name': MODELS['vae']}},
        '5': {
            'class_type': 'TextEncodeQwenImage21',
            'inputs': {
                'clip': ['3', 0],
                'prompt': prompt,
                'negative_prompt': '',
                'vae': ['4', 0],
                'resolution': resolution,
                'images.image_1': ['10', 0],
            },
        },
        '6': {'class_type': 'QwenImage21Cache', 'inputs': {'model': ['2', 0], 'device': 'auto', 'dtype': 'default'}},
        '7': {
            'class_type': 'KSampler',
            'inputs': {
                'model': ['6', 0],
                'positive': ['5', 0],
                'negative': ['5', 1],
                'latent_image': ['5', 2],
                'seed': seed,
                'steps': steps,
                'cfg': 1.0,
                'sampler_name': 'euler',
                'scheduler': 'simple',
                'denoise': 1.0,
            },
        },
        '8': {'class_type': 'VAEDecode', 'inputs': {'samples': ['7', 0], 'vae': ['4', 0]}},
        SAVE_IMAGE_NODE: {'class_type': 'SaveImage', 'inputs': {'images': ['8', 0], 'filename_prefix': prefix}},
    }
    if reference_upload_name is not None:
        graph['9'] = {'class_type': 'LoadImage', 'inputs': {'image': reference_upload_name}}
        graph['11'] = {'class_type': 'JoinImageWithAlpha', 'inputs': {'image': ['9', 0], 'alpha': ['9', 1]}}
        graph['5']['inputs']['images.image_2'] = ['11', 0]
    return graph


def upload(data: bytes, role: str, subfolder: str) -> dict[str, Any]:
    """Upload one PNG under a unique name; refuses while other work is queued."""
    require_idle_queue()
    filename = role + '-' + uuid.uuid4().hex + '.png'
    body, content_type = multipart({'type': 'input', 'subfolder': subfolder, 'overwrite': 'false'},
                                   'image', filename, data, 'image/png')
    response_bytes, _ = request('/upload/image', body, {'Content-Type': content_type})
    try:
        response = json.loads(response_bytes)
    except json.JSONDecodeError as error:
        raise RuntimeError('ComfyUI upload returned non-JSON data') from error
    if response.get('name') != filename or not isinstance(response.get('subfolder', ''), str):
        raise RuntimeError(f'Unexpected uploaded image: {response!r}')
    return {'role': role, 'sha256': sha256_bytes(data), 'dimensions': image_details(data, role), 'response': response,
            'loadImageName': posixpath.join(response['subfolder'], filename) if response.get('subfolder') else filename}


def await_success(prompt_id: str) -> tuple[Any, BaseException | None]:
    """Poll history until our prompt terminates; never cancels it on timeout."""
    deadline = time.monotonic() + DEADLINE_SECONDS
    while True:
        history = request_json('/history/' + urllib.parse.quote(prompt_id, safe=''))
        item = history.get(prompt_id) if isinstance(history, dict) else None
        if item is not None:
            status = item.get('status', {})
            problem = terminal_execution_error(status)
            if problem:
                return history, RuntimeError(f'Prompt {prompt_id} failed: {problem}: {status!r}')
            if status.get('completed'):
                if status.get('status_str') != 'success':
                    return history, RuntimeError(f'Prompt {prompt_id} completed without success: {status!r}')
                return history, None
        if time.monotonic() >= deadline:
            return history, TimeoutError(f'Prompt {prompt_id} not terminal after {DEADLINE_SECONDS}s; not cancelled')
        time.sleep(POLL_SECONDS)


def free_after_job(record: dict[str, Any]) -> None:
    # Only unload after our own prompt has TERMINATED and no other work is queued.
    record['queueBeforeFree'] = require_idle_queue()
    payload = json_bytes({'unload_models': True, 'free_memory': True})
    body, headers = request('/free', payload, {'Content-Type': 'application/json'})
    record['freeResponse'] = {'body': body.decode('utf-8', 'replace'), 'headers': headers}


def generate_job(kind: str, pose: str, image1: bytes, image2: bytes, prompt: str, seed: int,
                 resolution: int, attempt: Path, namespace: str, runner: Path) -> tuple[bytes, dict[str, Any]]:
    """Run one two-image Qwen edit into a fresh ``attempt`` directory.

    Writes ``workflow.json``, ``submitted.json``, ``history.json``, ``raw.png``
    and always ``provenance.json`` (also on failure). Returns the exact
    SaveImage PNG bytes and the provenance record.
    """
    if not 0 <= seed < 2 ** 64:
        raise ValueError(f'Unsigned 64-bit seed out of range: {seed}')
    attempt.mkdir(parents=True, exist_ok=False)
    helper = Path(__file__).resolve()
    runner = runner.resolve()
    record: dict[str, Any] = {
        'version': 1, 'kind': kind, 'pose': pose, 'status': 'started', 'startedAt': utc_now(),
        'endpoint': base_url(), 'provider': PROVIDER, 'comfyUiCommit': COMFYUI_COMMIT,
        'helper': str(helper), 'helperSha256': sha256_file(helper),
        'runner': str(runner), 'runnerSha256': sha256_file(runner),
        'prompt': prompt, 'seed': seed, 'resolution': resolution, 'steps': STEPS,
        'models': dict(MODELS), 'modelLicense': dict(MODEL_LICENSE),
        'sampler': 'euler', 'scheduler': 'simple', 'cfg': 1.0, 'denoise': 1.0,
        'alphaHandling': dict(ALPHA_HANDLING),
        'inputRoles': [{'role': 'image1', 'sha256': sha256_bytes(image1), 'image': image_details(image1, 'image1')},
                       {'role': 'image2', 'sha256': sha256_bytes(image2), 'image': image_details(image2, 'image2')}],
        'rights': RIGHTS,
    }
    try:
        record['queueBeforeUpload'] = require_idle_queue()
        record['runtimeModels'] = require_runtime_models()
        names = []
        for data, role in ((image1, 'image1'), (image2, 'image2')):
            entry = upload(data, role, f'{namespace}/{attempt.name}')
            record.setdefault('uploads', []).append(entry)
            names.append(entry['loadImageName'])
        record['queueBeforeSubmission'] = require_idle_queue()
        graph = workflow(names[0], names[1], prompt, seed, resolution, STEPS,
                         f'{namespace}/{kind}/{pose}/{attempt.name}')
        record['workflow'] = graph
        write_new(attempt / 'workflow.json', json_bytes(graph))
        submission = request_json('/prompt', {'client_id': f'{namespace}-{kind}-' + uuid.uuid4().hex, 'prompt': graph})
        record['submission'] = submission
        prompt_id = submission.get('prompt_id') if isinstance(submission, dict) else None
        if not isinstance(prompt_id, str) or not prompt_id or submission.get('node_errors'):
            raise RuntimeError(f'ComfyUI rejected workflow: {submission!r}')
        record['promptId'] = prompt_id
        write_new(attempt / 'submitted.json', json_bytes(record))
        print(json.dumps({'pose': pose, 'kind': kind, 'status': 'submitted', 'promptId': prompt_id}), flush=True)
        history, problem = await_success(prompt_id)
        write_new(attempt / 'history.json', json_bytes(history))
        record['history'] = history
        if problem:
            raise problem
        record['terminalSuccess'] = True
        descriptors = history[prompt_id].get('outputs', {}).get(SAVE_IMAGE_NODE, {}).get('images', [])
        if (not isinstance(descriptors, list) or len(descriptors) != 1 or not isinstance(descriptors[0], dict)
                or descriptors[0].get('type') != 'output' or not descriptors[0].get('filename')):
            raise RuntimeError(f'Expected exactly one SaveImage output at node {SAVE_IMAGE_NODE}: {descriptors!r}')
        descriptor = descriptors[0]
        query = urllib.parse.urlencode({key: descriptor[key] for key in ('filename', 'subfolder', 'type') if key in descriptor})
        raw, headers = request('/view?' + query)
        info = image_details(raw, 'SaveImage result')
        first = record['inputRoles'][0]['image']
        width, height = expected_canvas(first['width'], first['height'], resolution)
        record['expectedCanvas'] = {'width': width, 'height': height}
        if info['format'] != 'PNG' or (info['width'], info['height']) != (width, height):
            raise RuntimeError(f'SaveImage dimensions differ from expected {width}x{height}: {info!r}')
        write_new(attempt / 'raw.png', raw)
        record['raw'] = {'path': str(attempt / 'raw.png'), 'sha256': sha256_bytes(raw), 'bytes': len(raw), 'image': info,
                         'saveImage': descriptor, 'responseHeaders': headers,
                         'pixelModification': 'none; exact SaveImage /view PNG bytes'}
        free_after_job(record)
        record['status'] = 'success'
        return raw, record
    except BaseException as error:
        record['status'] = 'failed' if not record.get('terminalSuccess') else 'output-or-cleanup-failed'
        record['error'] = {'type': type(error).__name__, 'message': str(error)}
        if isinstance(error, KeyboardInterrupt):
            record['error']['message'] = 'local interruption; submitted prompt was not cancelled'
        raise
    finally:
        record['finishedAt'] = utc_now()
        write_new(attempt / 'provenance.json', json_bytes(record))


def check_previous_attempts(directory: Path, pattern: str, output_path: Path, label: str) -> None:
    """Refuse a new submission while an earlier attempt is unresolved.

    An earlier attempt blocks when it was submitted without a terminal record,
    when its prompt is still not terminal on the server, or when it succeeded
    but its output was never published (unless ``visual-review.json`` in the
    attempt records an explicit rejection of exactly that preserved raw).
    """
    if output_path.exists():
        return
    for attempt in sorted(directory.glob(pattern)):
        submitted = attempt / 'submitted.json'
        proof = attempt / 'provenance.json'
        if not submitted.exists():
            continue
        details = json.loads(submitted.read_text(encoding='utf-8'))
        if not proof.exists():
            raise RuntimeError(f'Unresolved previous prompt {details.get("promptId")} for {label} at {attempt}; inspect history before retry')
        outcome = json.loads(proof.read_text(encoding='utf-8'))
        if outcome.get('promptId') and not outcome.get('terminalSuccess'):
            history = request_json('/history/' + urllib.parse.quote(outcome['promptId'], safe=''))
            status = history.get(outcome['promptId'], {}).get('status', {}) if isinstance(history, dict) else {}
            if not terminal_execution_error(status) and not status.get('completed'):
                raise RuntimeError(f'Previous prompt {outcome["promptId"]} is not terminal; no duplicate submitted')
        if outcome.get('terminalSuccess'):
            review_path = attempt / 'visual-review.json'
            if review_path.exists():
                review = json.loads(review_path.read_text(encoding='utf-8'))
                preserved = outcome.get('raw', {})
                if (review.get('decision') == 'rejected' and review.get('promptId') == outcome['promptId']
                        and preserved.get('path') and review.get('sha256') == preserved.get('sha256')
                        and sha256_file(Path(preserved['path'])) == preserved['sha256']):
                    continue
            raise RuntimeError(f'Previous completed prompt {outcome["promptId"]} at {attempt}; recover its preserved raw '
                               'or record a rejection in visual-review.json before generating again')
