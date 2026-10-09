#!/usr/bin/env node
import { createReadStream } from 'node:fs';
import { copyFile, lstat, mkdir, readdir, readFile, realpath } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, extname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SITE = join(ROOT, 'site');
const ORIGIN = 'https://hanbong5938.github.io';
const PREFIX = '/herdr-desktop-pet/';
const LOCALES = new Map([['ko', `${ORIGIN}${PREFIX}`], ['en', `${ORIGIN}${PREFIX}en/`]]);
const PUBLIC_FILES = new Set([
  'index.html', 'en/index.html', '404.html', 'sitemap.xml',
  'assets/css/site.css', 'assets/js/site.js',
  'assets/images/favicon.svg', 'assets/images/rubelia-thumbnail.png',
  'assets/images/product-desktop.png', 'assets/images/product-detail.png',
  'assets/images/og-ko.png', 'assets/images/og-en.png',
]);
const MIMES = new Map([
  ['.html', 'text/html; charset=utf-8'], ['.css', 'text/css; charset=utf-8'],
  ['.js', 'text/javascript; charset=utf-8'], ['.svg', 'image/svg+xml'],
  ['.png', 'image/png'], ['.xml', 'application/xml; charset=utf-8'],
]);

function fail(message) { throw new Error(message); }
function inside(parent, child) {
  const rel = relative(parent, child);
  return rel === '' || (rel !== '..' && !rel.startsWith(`..${sep}`) && !isAbsolute(rel));
}

async function siteFiles() {
  const files = [];
  async function visit(directory, prefix = '') {
    for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
      const name = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) await visit(join(directory, entry.name), name);
      else if (entry.isFile() && PUBLIC_FILES.has(name)) files.push(name);
      else fail(`site/${name}: not a publishable regular file (symlinks and unexpected artifacts are forbidden)`);
    }
  }
  await visit(SITE);
  for (const name of PUBLIC_FILES) if (!files.includes(name)) fail(`site/${name}: missing public file`);
  return files;
}

function entities(value) {
  return value.replace(/&(#(?:x[0-9a-f]+|[0-9]+)|amp|quot|apos|lt|gt);/gi, (_, entity) => {
    if (entity[0] === '#') {
      const hex = entity[1]?.toLowerCase() === 'x';
      const number = Number.parseInt(entity.slice(hex ? 2 : 1), hex ? 16 : 10);
      return number > 0 && number <= 0x10ffff && !(number >= 0xd800 && number <= 0xdfff) ? String.fromCodePoint(number) : '\ufffd';
    }
    return { amp: '&', quot: '"', apos: "'", lt: '<', gt: '>' }[entity.toLowerCase()];
  });
}

// Extract start tags without mistaking a quoted '>' for the end of a tag.
function* tags(html) {
  let i = 0;
  while ((i = html.indexOf('<', i)) !== -1) {
    if (html.startsWith('<!--', i)) {
      const end = html.indexOf('-->', i + 4);
      if (end === -1) fail('unterminated HTML comment');
      i = end + 3;
      continue;
    }
    const start = i++;
    let quote = '';
    for (; i < html.length; i++) {
      const char = html[i];
      if (quote) { if (char === quote) quote = ''; }
      else if (char === '"' || char === "'") quote = char;
      else if (char === '>') break;
    }
    if (i === html.length) fail('unterminated HTML tag');
    const token = html.slice(start + 1, i++);
    const match = /^\s*([a-z][\w:-]*)(?=[\s/>]|$)/i.exec(token);
    if (!match) continue;
    const name = match[1].toLowerCase();
    const attrs = new Map();
    const attrPattern = /([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g;
    let attr;
    attrPattern.lastIndex = match[0].length;
    while ((attr = attrPattern.exec(token))) {
      const key = attr[1].toLowerCase();
      if (attrs.has(key)) fail(`duplicate ${key} attribute on <${name}>`);
      attrs.set(key, entities(attr[2] ?? attr[3] ?? attr[4] ?? ''));
    }
    yield { name, attrs };
    if (name === 'script' || name === 'style') {
      const close = new RegExp(`</${name}\\s*>`, 'ig');
      close.lastIndex = i;
      const end = close.exec(html);
      if (!end) fail(`unterminated <${name}>`);
      i = close.lastIndex;
    }
  }
}

function localReference(raw, source, files, ids, { absolute = false } = {}) {
  if (!raw || raw.trim() !== raw) fail(`${source}: empty or whitespace-padded URL`);
  if (/^(?:mailto:|tel:|data:)/i.test(raw) && !absolute) return;
  if (absolute && !raw.startsWith(`${ORIGIN}${PREFIX}`)) fail(`${source}: expected an absolute project URL, got ${raw}`);
  if (raw.startsWith('//')) fail(`${source}: protocol-relative URL not allowed: ${raw}`);
  let url;
  try { url = new URL(raw, `${ORIGIN}${PREFIX}${source.split(' ')[0]}`); }
  catch { fail(`${source}: invalid URL ${raw}`); }
  if (url.origin !== ORIGIN) {
    if (absolute) fail(`${source}: unexpected origin ${raw}`);
    return;
  }
  const path = url.pathname;
  if (!path.startsWith(PREFIX)) fail(`${source}: URL must stay inside ${PREFIX}: ${raw}`);
  const encoded = path.slice(PREFIX.length);
  let decoded;
  try { decoded = decodeURIComponent(encoded); } catch { fail(`${source}: invalid URL encoding: ${raw}`); }
  if (decoded.includes('\\') || decoded.includes('\0') || decoded.split('/').includes('..') || decoded.split('/').includes('.')) {
    fail(`${source}: unsafe URL path ${raw}`);
  }
  const target = decoded.endsWith('/') || !decoded ? `${decoded}index.html` : decoded;
  if (!files.has(target)) fail(`${source}: missing site/${target} referenced by ${raw}`);
  if (url.hash) {
    let id;
    try { id = decodeURIComponent(url.hash.slice(1)); } catch { fail(`${source}: malformed fragment ${raw}`); }
    if (id && (!ids.has(target) || !ids.get(target).has(id))) fail(`${source}: missing #${id} in site/${target}`);
  }
}

function parseDocument(html, name) {
  const ids = new Set();
  const links = [];
  const canonical = [];
  const alternates = new Map();
  const metas = new Map();
  for (const { name: tag, attrs } of tags(html)) {
    const id = attrs.get('id');
    if (id !== undefined) {
      if (!id || ids.has(id)) fail(`site/${name}: empty or duplicate id ${JSON.stringify(id)}`);
      ids.add(id);
    }
    if (tag === 'link') {
      const rel = (attrs.get('rel') || '').toLowerCase().split(/\s+/);
      const href = attrs.get('href');
      if (rel.includes('canonical')) canonical.push(href);
      else if (rel.includes('alternate') && attrs.has('hreflang')) {
        const language = attrs.get('hreflang').toLowerCase();
        if (alternates.has(language)) fail(`site/${name}: duplicate hreflang ${language}`);
        alternates.set(language, href);
      } else if (href) links.push(href);
    }
    if (tag === 'meta' && (attrs.has('property') || attrs.has('name'))) {
      const key = (attrs.get('property') || attrs.get('name')).toLowerCase();
      if (key.startsWith('og:') || key.startsWith('twitter:')) {
        if (metas.has(key)) fail(`site/${name}: duplicate ${key}`);
        metas.set(key, attrs.get('content'));
      }
    }
    if (tag === 'a' || tag === 'area') { if (attrs.has('href')) links.push(attrs.get('href')); }
    if (['img', 'script', 'source', 'video', 'audio', 'iframe'].includes(tag) && attrs.has('src')) links.push(attrs.get('src'));
    if (tag === 'video' && attrs.has('poster')) links.push(attrs.get('poster'));
    if ((tag === 'img' || tag === 'source') && attrs.has('srcset')) {
      for (const candidate of attrs.get('srcset').split(',')) links.push(candidate.trim().split(/\s+/)[0]);
    }
  }
  return { ids, links, canonical, alternates, metas };
}

async function check() {
  const names = await siteFiles();
  const files = new Set(names);
  const documents = new Map();
  for (const name of names.filter(name => name.endsWith('.html'))) {
    documents.set(name, parseDocument(await readFile(join(SITE, name), 'utf8'), name));
  }
  const ids = new Map([...documents].map(([name, doc]) => [name, doc.ids]));
  for (const [name, doc] of documents) {
    for (const link of doc.links) localReference(link, `${name} resource`, files, ids);
    if (name === '404.html') continue;
    const expected = LOCALES.get(name === 'index.html' ? 'ko' : 'en');
    if (doc.canonical.length !== 1 || doc.canonical[0] !== expected) fail(`site/${name}: canonical must be ${expected}`);
    if (doc.alternates.size !== 3) fail(`site/${name}: expected ko, en and x-default hreflang links`);
    for (const [locale, url] of [...LOCALES, ['x-default', LOCALES.get('ko')]]) {
      if (doc.alternates.get(locale) !== url) fail(`site/${name}: incorrect ${locale} hreflang URL`);
    }
    if (doc.metas.get('og:url') !== expected) fail(`site/${name}: og:url must be ${expected}`);
    const expectedImage = `${ORIGIN}${PREFIX}assets/images/og-${name === 'index.html' ? 'ko' : 'en'}.png`;
    if (doc.metas.get('og:image') !== expectedImage) fail(`site/${name}: og:image must be ${expectedImage}`);
    for (const key of ['og:image', 'twitter:image']) {
      if (key === 'twitter:image' && doc.metas.has(key) && doc.metas.get(key) !== expectedImage) fail(`site/${name}: twitter:image must be ${expectedImage}`);
      if (doc.metas.has(key)) localReference(doc.metas.get(key), `${name} ${key}`, files, ids, { absolute: true });
    }
  }
  const css = await readFile(join(SITE, 'assets/css/site.css'), 'utf8');
  const cssSource = 'assets/css/site.css';
  for (const match of css.matchAll(/url\(\s*["']?([^"')]+)["']?\s*\)/gi)) {
    localReference(match[1].trim(), `${cssSource} url()`, files, ids);
  }
  for (const match of css.matchAll(/@import\s+["']([^"']+)["']/gi)) {
    localReference(match[1], `${cssSource} @import`, files, ids);
  }
  const sitemap = await readFile(join(SITE, 'sitemap.xml'), 'utf8');
  const locations = [...sitemap.matchAll(/<loc>\s*([^<]+?)\s*<\/loc>/gi)].map(match => entities(match[1]));
  if (locations.length !== LOCALES.size || locations.some(url => ![...LOCALES.values()].includes(url)) || new Set(locations).size !== LOCALES.size) {
    fail('site/sitemap.xml: expected exactly the Korean and English canonical URLs');
  }
  for (const url of locations) localReference(url, 'sitemap.xml loc', files, ids, { absolute: true });
  console.log(`Checked ${names.length} public site files and local references.`);
  return names;
}

async function stage(destination) {
  if (!destination) fail('usage: node tools/pages.mjs stage OUTPUT_DIRECTORY');
  const output = resolve(destination);
  let ancestor = output;
  while (true) {
    try { await lstat(ancestor); break; }
    catch (error) { if (error.code !== 'ENOENT') throw error; ancestor = dirname(ancestor); }
  }
  const canonicalOutput = resolve(await realpath(ancestor), relative(ancestor, output));
  if (inside(ROOT, canonicalOutput) || inside(canonicalOutput, ROOT)) fail('stage output must be outside the repository and its parent paths');
  const names = await check();
  try {
    const stat = await lstat(output);
    if (!stat.isDirectory() || (await readdir(output)).length) fail('stage output must be a new or empty directory');
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
  for (const name of names) {
    const target = join(output, name);
    await mkdir(dirname(target), { recursive: true });
    await copyFile(join(SITE, name), target);
  }
  console.log(`Staged ${names.length} public files to ${output}`);
}

async function serve(port) {
  if (!/^[0-9]+$/.test(port || '') || Number(port) < 1 || Number(port) > 65535) fail('usage: node tools/pages.mjs serve --port NUMBER (1-65535)');
  const server = createServer(async (request, response) => {
    try {
      if (request.method !== 'GET' && request.method !== 'HEAD') {
        response.writeHead(405, { Allow: 'GET, HEAD' }).end();
        return;
      }
      const raw = request.url || '/';
      const rawPath = raw.split(/[?#]/, 1)[0];
      let decoded;
      try { decoded = decodeURIComponent(rawPath); } catch { response.writeHead(400).end(); return; }
      if (decoded.includes('\\') || decoded.includes('\0') || decoded.split('/').some(part => part === '..' || part === '.')) {
        response.writeHead(400).end(); return;
      }
      const url = new URL(raw, 'http://127.0.0.1');
      if (url.pathname === PREFIX.slice(0, -1)) {
        response.writeHead(308, { Location: `${PREFIX}${url.search}` }).end(); return;
      }
      let name = url.pathname.startsWith(PREFIX) ? url.pathname.slice(PREFIX.length) : null;
      if (name && !name.endsWith('/') && !extname(name)) {
        const dir = join(SITE, name);
        if (inside(SITE, dir) && (await lstat(dir).catch(() => null))?.isDirectory()) {
          response.writeHead(308, { Location: `${url.pathname}/${url.search}` }).end(); return;
        }
      }
      if (name !== null) {
        try { name = decodeURIComponent(name); } catch { response.writeHead(400).end(); return; }
        if (name.endsWith('/') || !name) name += 'index.html';
      }
      let status = 200;
      if (name === null || !PUBLIC_FILES.has(name)) { name = '404.html'; status = 404; }
      let file = join(SITE, name);
      for (let part = SITE, index = 0, pieces = name.split('/'); index < pieces.length; index++) {
        part = join(part, pieces[index]);
        const stat = await lstat(part).catch(() => null);
        if (!stat || stat.isSymbolicLink() || (index === pieces.length - 1 ? !stat.isFile() : !stat.isDirectory())) {
          name = '404.html'; file = join(SITE, name); status = 404; break;
        }
      }
      const stat = await lstat(file).catch(() => null);
      if (!stat || stat.isSymbolicLink() || !stat.isFile()) {
        response.writeHead(500, { 'Content-Type': 'text/plain; charset=utf-8', 'X-Content-Type-Options': 'nosniff' })
          .end(request.method === 'HEAD' ? undefined : 'Internal server error');
        return;
      }
      response.writeHead(status, { 'Content-Type': MIMES.get(extname(name)) || 'application/octet-stream', 'Content-Length': stat.size, 'X-Content-Type-Options': 'nosniff' });
      if (request.method === 'HEAD') response.end();
      else createReadStream(file).on('error', () => response.destroy()).pipe(response);
    } catch (error) {
      console.error(error);
      if (!response.headersSent) response.writeHead(500).end('Internal server error');
      else response.destroy();
    }
  });
  server.listen(Number(port), '127.0.0.1', () => console.log(`Serving http://127.0.0.1:${port}${PREFIX}`));
}

try {
  const [command, ...args] = process.argv.slice(2);
  if (command === 'check' && args.length === 0) await check();
  else if (command === 'stage' && args.length === 1) await stage(args[0]);
  else if (command === 'serve' && args.length === 2 && args[0] === '--port') await serve(args[1]);
  else fail('usage: node tools/pages.mjs check | stage OUTPUT_DIRECTORY | serve --port NUMBER');
} catch (error) {
  console.error(`Pages: ${error.message}`);
  process.exitCode = 1;
}
