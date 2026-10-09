import { test, expect } from 'bun:test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { once } from 'node:events';
import { copyFile, mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { request, type IncomingHttpHeaders } from 'node:http';
import { createServer as createNetServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const prefix = '/herdr-desktop-pet/';
const korean = '<h1>한국어 fixture</h1>';
const english = '<h1>English fixture</h1>';
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
    await mkdir(join(site, 'en'), { recursive: true });
    await mkdir(join(site, 'assets', 'css'), { recursive: true });
    await mkdir(outside);
    await copyFile(fileURLToPath(new URL('./pages.mjs', import.meta.url)), join(root, 'tools', 'pages.mjs'));
    await writeFile(join(site, 'index.html'), korean);
    await writeFile(join(site, 'en', 'index.html'), english);
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
    verify(await get(port, `${prefix}assets/css/site.css`), 200, 'text/css; charset=utf-8', 'body { color: blue; }');
    for (const [path, location] of [
      ['/herdr-desktop-pet?source=test', `${prefix}?source=test`],
      [`${prefix}en?source=test`, `${prefix}en/?source=test`],
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
