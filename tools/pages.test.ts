import { test, expect } from 'bun:test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { once } from 'node:events';
import { copyFile, cp, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { request, type IncomingHttpHeaders } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const prefix = '/herdr-desktop-pet/';
const korean = '<h1>한국어 fixture</h1>';
const english = '<h1>English fixture</h1>';
const chinese = '<h1>简体中文 fixture</h1>';
const japanese = '<h1>日本語 fixture</h1>';
const notFound = '<h1>Not found fixture</h1>';
const secret = 'PRIVATE_SENTINEL_1234';

async function availablePort(): Promise<number> {
  const server = createNetServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('No available TCP port');
  const closed = once(server, 'close');
  server.close();
  await closed;
  return address.port;
}

type HttpResult = { status: number; headers: IncomingHttpHeaders; body: string };

function get(port: number, path: string, method: 'GET' | 'HEAD' = 'GET'): Promise<HttpResult> {
  const { promise, resolve, reject } = Promise.withResolvers<HttpResult>();
  const req = request({ hostname: '127.0.0.1', port, path, method }, response => {
    const chunks: Buffer[] = [];
    response.on('data', chunk => chunks.push(chunk));
    response.on('error', reject);
    response.on('end', () => resolve({ status: response.statusCode!, headers: response.headers, body: Buffer.concat(chunks).toString('utf8') }));
  });
  // A real child process and TCP connection need a wall-clock failure deadline.
  req.setTimeout(3000, () => req.destroy(new Error(`HTTP request timed out: ${method} ${path}`)));
  req.on('error', reject);
  req.end();
  return promise;
}

function verify(result: HttpResult, status: number, mime: string, body: string): void {
  expect(result.status).toBe(status);
  expect(result.headers['content-type']).toBe(mime);
  expect(result.headers['x-content-type-options']).toBe('nosniff');
  expect(result.body).toBe(body);
  expect(result.body).not.toContain(secret);
}

test('serve CLI keeps public responses safe when requested paths and 404 fallback are unsafe', async () => {
  const root = await mkdtemp(join(tmpdir(), 'herdr-pages-serve-'));
  const site = join(root, 'site');
  const outside = join(root, 'outside');
  let child: ChildProcessWithoutNullStreams | undefined;
  try {
    await mkdir(join(root, 'tools'));
    for (const locale of ['en', 'zh-Hans', 'ja']) await mkdir(join(site, locale), { recursive: true });
    await mkdir(join(site, 'assets', 'css'), { recursive: true });
    await mkdir(outside);
    await copyFile(fileURLToPath(new URL('./pages.mjs', import.meta.url)), join(root, 'tools', 'pages.mjs'));
    await writeFile(join(site, 'index.html'), korean);
    await writeFile(join(site, 'en', 'index.html'), english);
    await writeFile(join(site, 'zh-Hans', 'index.html'), chinese);
    await writeFile(join(site, 'ja', 'index.html'), japanese);
    await writeFile(join(site, '404.html'), notFound);
    await writeFile(join(site, 'assets', 'css', 'site.css'), 'body { color: blue; }');
    await writeFile(join(outside, 'secret.txt'), secret);
    await writeFile(join(outside, 'index.html'), secret);
    await writeFile(join(outside, 'site.css'), secret);

    const port = await availablePort();
    child = spawn('node', [join(root, 'tools', 'pages.mjs'), 'serve', '--port', String(port)]);
    const server = child;
    let stderr = '';
    server.stderr.on('data', data => { stderr += data.toString(); });
    const { promise: readyPromise, resolve, reject } = Promise.withResolvers<void>();
    // The real Node server can hang or fail before its readiness log; fail with context.
    const timer = setTimeout(() => finish(new Error(`Server startup timed out: ${stderr}`)), 5000);
    let stdout = '';
    function finish(error?: Error) {
      clearTimeout(timer);
      server.stdout.off('data', ready);
      server.off('error', failed);
      server.off('exit', exited);
      if (error) reject(error); else resolve();
    }
    function ready(data: Buffer) {
      stdout += data.toString();
      if (stdout.includes(`Serving http://127.0.0.1:${port}${prefix}`)) finish();
    }
    function failed(error: Error) { finish(error); }
    function exited(code: number | null) { finish(new Error(`Server exited before readiness (${code}): ${stderr}`)); }
    server.stdout.on('data', ready);
    server.once('error', failed);
    server.once('exit', exited);
    await readyPromise;

    verify(await get(port, prefix), 200, 'text/html; charset=utf-8', korean);
    verify(await get(port, prefix, 'HEAD'), 200, 'text/html; charset=utf-8', '');
    verify(await get(port, `${prefix}en/?source=test`), 200, 'text/html; charset=utf-8', english);
    verify(await get(port, `${prefix}en/`, 'HEAD'), 200, 'text/html; charset=utf-8', '');
    verify(await get(port, `${prefix}zh-Hans/?source=test`), 200, 'text/html; charset=utf-8', chinese);
    verify(await get(port, `${prefix}zh-Hans/`, 'HEAD'), 200, 'text/html; charset=utf-8', '');
    verify(await get(port, `${prefix}ja/?source=test`), 200, 'text/html; charset=utf-8', japanese);
    verify(await get(port, `${prefix}ja/`, 'HEAD'), 200, 'text/html; charset=utf-8', '');
    verify(await get(port, `${prefix}assets/css/site.css`), 200, 'text/css; charset=utf-8', 'body { color: blue; }');
    for (const [path, location] of [
      ['/herdr-desktop-pet?source=test', `${prefix}?source=test`],
      [`${prefix}en?source=test`, `${prefix}en/?source=test`],
      [`${prefix}zh-Hans?source=test`, `${prefix}zh-Hans/?source=test`],
      [`${prefix}ja?source=test`, `${prefix}ja/?source=test`],
    ]) {
      const redirected = await get(port, path);
      expect(redirected.status).toBe(308);
      expect(redirected.headers.location).toBe(location);
    }
    verify(await get(port, `${prefix}unknown/deep/file`, 'GET'), 404, 'text/html; charset=utf-8', notFound);
    verify(await get(port, `${prefix}unknown/deep/file`, 'HEAD'), 404, 'text/html; charset=utf-8', '');

    await rm(join(site, 'assets', 'css', 'site.css'));
    await symlink(join(outside, 'site.css'), join(site, 'assets', 'css', 'site.css'));
    verify(await get(port, `${prefix}assets/css/site.css`), 404, 'text/html; charset=utf-8', notFound);
    verify(await get(port, `${prefix}assets/css/site.css`, 'HEAD'), 404, 'text/html; charset=utf-8', '');
    await rm(join(site, 'en'), { recursive: true });
    await symlink(outside, join(site, 'en'), 'dir');
    verify(await get(port, `${prefix}en/index.html`), 404, 'text/html; charset=utf-8', notFound);
    verify(await get(port, `${prefix}en/index.html`, 'HEAD'), 404, 'text/html; charset=utf-8', '');

    await rm(join(site, '404.html'));
    await symlink('../outside/secret.txt', join(site, '404.html'));
    for (const path of [`${prefix}unknown/deep/file`, `${prefix}404.html`]) {
      verify(await get(port, path), 500, 'text/plain; charset=utf-8', 'Internal server error');
      verify(await get(port, path, 'HEAD'), 500, 'text/plain; charset=utf-8', '');
    }
    verify(await get(port, prefix), 200, 'text/html; charset=utf-8', korean);

    await rm(join(site, '404.html'));
    verify(await get(port, `${prefix}unknown`), 500, 'text/plain; charset=utf-8', 'Internal server error');
    verify(await get(port, `${prefix}unknown`, 'HEAD'), 500, 'text/plain; charset=utf-8', '');
    await mkdir(join(site, '404.html'));
    verify(await get(port, `${prefix}unknown`), 500, 'text/plain; charset=utf-8', 'Internal server error');
    verify(await get(port, `${prefix}unknown`, 'HEAD'), 500, 'text/plain; charset=utf-8', '');
    verify(await get(port, prefix), 200, 'text/html; charset=utf-8', korean);
  } finally {
    if (child && child.exitCode === null && child.signalCode === null) {
      const { promise, resolve } = Promise.withResolvers<void>();
      // Bound real-process shutdown so a failed test cannot strand its fixture.
      const timer = setTimeout(() => child!.kill('SIGKILL'), 2000);
      child.once('close', () => { clearTimeout(timer); resolve(); });
      child.kill();
      await promise;
    }
    await rm(root, { recursive: true, force: true });
  }
});

type CliResult = { code: number | null; stdout: string; stderr: string };

function runPages(script: string, ...args: string[]): Promise<CliResult> {
  const { promise, resolve, reject } = Promise.withResolvers<CliResult>();
  const child = spawn('node', [script, ...args]);
  let stdout = '';
  let stderr = '';
  child.stdout.on('data', data => { stdout += data.toString(); });
  child.stderr.on('data', data => { stderr += data.toString(); });
  // A spawned Node process uses the real clock; this deadline only bounds a hung CLI.
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    child.kill('SIGKILL');
  }, 10000);
  child.once('error', error => { clearTimeout(timer); reject(error); });
  child.once('close', code => {
    clearTimeout(timer);
    if (timedOut) reject(new Error('Pages CLI timed out'));
    else resolve({ code, stdout, stderr });
  });
  return promise;
}

function pngChunk(type: string, payload: Buffer): Buffer {
  const chunk = Buffer.alloc(12 + payload.length);
  chunk.writeUInt32BE(payload.length, 0);
  chunk.write(type, 4, 4, 'ascii');
  payload.copy(chunk, 8);
  let crc = 0xffffffff;
  for (let i = 4; i < 8 + payload.length; i++) {
    crc ^= chunk[i];
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
  }
  chunk.writeUInt32BE((crc ^ 0xffffffff) >>> 0, 8 + payload.length);
  return chunk;
}

function beforeIend(png: Buffer): number {
  let offset = 8;
  while (offset + 12 <= png.length) {
    const length = png.readUInt32BE(offset);
    if (png.toString('ascii', offset + 4, offset + 8) === 'IEND') return offset;
    offset += 12 + length;
  }
  throw new Error('Fixture PNG has no IEND chunk');
}

test('stage publishes clean PNGs and check/stage reject private PNG chunks and malformed envelopes', async () => {
  const root = await mkdtemp(join(tmpdir(), 'herdr-pages-png-checkout-'));
  let outputs: string | undefined;
  try {
    outputs = await mkdtemp(join(tmpdir(), 'herdr-pages-png-outputs-'));
    await mkdir(join(root, 'tools'));
    const script = join(root, 'tools', 'pages.mjs');
    await copyFile(fileURLToPath(new URL('./pages.mjs', import.meta.url)), script);
    await cp(fileURLToPath(new URL('../site/', import.meta.url)), join(root, 'site'), { recursive: true });
    const image = join(root, 'site', 'assets', 'images', 'product-desktop.png');
    const clean = await readFile(image);
    const cleanStage = join(outputs, 'clean');
    const success = await runPages(script, 'stage', cleanStage);
    expect(success.code).toBe(0);
    expect(success.stdout).not.toContain(secret);
    expect(success.stderr).not.toContain(secret);
    expect(await readFile(join(cleanStage, 'assets', 'images', 'product-desktop.png'))).toEqual(clean);
    for (const path of ['', 'en/', 'zh-Hans/', 'ja/']) {
      expect(await readFile(join(cleanStage, path, 'index.html')))
        .toEqual(await readFile(join(root, 'site', path, 'index.html')));
    }
    expect(await readFile(join(cleanStage, 'sitemap.xml'))).toEqual(await readFile(join(root, 'site', 'sitemap.xml')));

    const iend = beforeIend(clean);
    const xmp = Buffer.from(`XML:com.adobe.xmp\0\0\0\0\0${secret}`);
    const cases: [string, Buffer][] = [
      ['exif', Buffer.concat([clean.subarray(0, iend), pngChunk('eXIf', Buffer.from(secret)), clean.subarray(iend)])],
      ['international-text', Buffer.concat([clean.subarray(0, iend), pngChunk('iTXt', Buffer.from(`Comment\0\0\0\0\0${secret}`)), clean.subarray(iend)])],
      ['text', Buffer.concat([clean.subarray(0, iend), pngChunk('tEXt', Buffer.from(`Comment\0${secret}`)), clean.subarray(iend)])],
      ['xmp', Buffer.concat([clean.subarray(0, iend), pngChunk('iTXt', xmp), clean.subarray(iend)])],
      ['profile', Buffer.concat([clean.subarray(0, iend), pngChunk('iCCP', Buffer.from(`Profile\0\0${secret}`)), clean.subarray(iend)])],
      ['unknown', Buffer.concat([clean.subarray(0, iend), pngChunk('iDOT', Buffer.from(secret)), clean.subarray(iend)])],
      ['invalid-signature', Buffer.concat([Buffer.from(secret).subarray(0, 8), clean.subarray(8)])],
      ['truncated-chunk', Buffer.concat([clean.subarray(0, iend), Buffer.from([0, 0, 0, 64, 0x74, 0x45, 0x58, 0x74]), Buffer.from(secret)])],
      ['trailing-data', Buffer.concat([clean, Buffer.from(secret)])],
    ];
    for (const [label, tampered] of cases) {
      await writeFile(image, tampered);
      for (const command of ['check', 'stage'] as const) {
        const destination = join(outputs, label);
        const result = await runPages(script, command, ...(command === 'stage' ? [destination] : []));
        expect(result.code).not.toBe(0);
        expect(result.stderr.length).toBeGreaterThan(0);
        expect(result.stdout).not.toContain(secret);
        expect(result.stderr).not.toContain(secret);
        if (command === 'stage') {
          const entries = await readdir(destination).catch((error: NodeJS.ErrnoException) => {
            if (error.code === 'ENOENT') return [];
            throw error;
          });
          expect(entries).toEqual([]);
        }
      }
    }
  } finally {
    await rm(root, { recursive: true, force: true });
    if (outputs) await rm(outputs, { recursive: true, force: true });
  }
});

test('check CLI rejects broken locale metadata and sitemap alternates', async () => {
  const root = await mkdtemp(join(tmpdir(), 'herdr-pages-locale-checkout-'));
  try {
    await mkdir(join(root, 'tools'));
    const script = join(root, 'tools', 'pages.mjs');
    await copyFile(fileURLToPath(new URL('./pages.mjs', import.meta.url)), script);
    await cp(fileURLToPath(new URL('../site/', import.meta.url)), join(root, 'site'), { recursive: true });
    const cases = [
      { file: 'ja/index.html', before: '<html lang="ja">', after: '<html lang="en">', error: 'html lang must be ja' },
      { file: 'zh-Hans/index.html', before: 'hreflang="ja"', after: 'hreflang="de"', error: 'incorrect ja hreflang URL' },
      { file: 'ja/index.html', before: 'content="ja_JP"', after: 'content="en_US"', error: 'incorrect og:locale' },
      { file: 'sitemap.xml', before: 'hreflang="zh-Hans"', after: 'hreflang="de"', error: 'incorrect zh-Hans alternate' },
    ];
    for (const { file, before, after, error } of cases) {
      const target = join(root, 'site', file);
      const original = await readFile(target, 'utf8');
      expect(original).toContain(before);
      await writeFile(target, original.replace(before, after));
      const result = await runPages(script, 'check');
      expect(result.code).not.toBe(0);
      expect(result.stderr).toContain(error);
      await writeFile(target, original);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
