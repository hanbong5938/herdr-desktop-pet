import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { dirname, join, posix, resolve } from "node:path";

export type ReleaseRecord = {
  tag_name: string;
  html_url: string;
  published_at: string | null;
  draft: boolean;
  prerelease: boolean;
  assets: { name: string; size: number; state?: string }[];
};

export function parseStableTag(tag: string): { version: string; major: number; minor: number; patch: number } {
  const match = /^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.exec(tag);
  if (!match || match[0] !== tag) throw new Error(`Expected a stable vMAJOR.MINOR.PATCH tag without leading zeros: ${tag}`);
  const [major, minor, patch] = match.slice(1).map(Number);
  if (![major, minor, patch].every(Number.isSafeInteger)) throw new Error(`Tag version exceeds safe integer range: ${tag}`);
  return { version: tag.slice(1), major: major!, minor: minor!, patch: patch! };
}

export function requiredAssetNames(tag: string): string[] {
  const { version } = parseStableTag(tag);
  const archive = `HerdrDesktopPet-v${version}-macos-arm64.tar.gz`;
  return [archive, `${archive}.sha256`, "SHA256SUMS"];
}

export function isPublishedRelease(record: ReleaseRecord): boolean {
  let required: string[];
  try { required = requiredAssetNames(record.tag_name); } catch { return false; }
  if (record.draft || record.prerelease || !record.published_at || !Number.isFinite(Date.parse(record.published_at))) return false;
  return required.every((name) => record.assets.some((asset) =>
    asset.name === name && Number.isFinite(asset.size) && asset.size > 0 &&
    asset.state === "uploaded"));
}

function requireRepository(repository: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*\/[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(repository) || /[\r\n]/.test(repository)) {
    throw new Error(`Invalid GitHub repository: ${repository}`);
  }
}

function isReleaseRecord(value: unknown): value is ReleaseRecord {
  if (!value || typeof value !== "object") return false;
  const record = value as ReleaseRecord;
  return typeof record.tag_name === "string" && typeof record.html_url === "string" &&
    (record.published_at === null || typeof record.published_at === "string") &&
    typeof record.draft === "boolean" && typeof record.prerelease === "boolean" &&
    Array.isArray(record.assets) && record.assets.every((asset) => asset &&
      typeof asset.name === "string" && typeof asset.size === "number" &&
      (asset.state === undefined || typeof asset.state === "string"));
}

export async function fetchReleases(repository: string, token?: string): Promise<ReleaseRecord[]> {
  requireRepository(repository);
  const headers: Record<string, string> = {
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  if (token) headers.Authorization = `Bearer ${token}`;
  const releases: ReleaseRecord[] = [];
  for (let page = 1; ; page++) {
    const response = await fetch(`https://api.github.com/repos/${repository}/releases?per_page=100&page=${page}`, { headers });
    if (!response.ok) throw new Error(`GitHub release catalog request failed (HTTP ${response.status}, page ${page})`);
    const data: unknown = await response.json();
    if (!Array.isArray(data) || !data.every(isReleaseRecord)) throw new Error(`Invalid GitHub release catalog response on page ${page}`);
    releases.push(...data);
    if (data.length < 100) return releases;
  }
}

function escaped(text: string, index: number): boolean {
  let count = 0;
  for (let i = index - 1; i >= 0 && text[i] === "\\"; i--) count++;
  return count % 2 !== 0;
}

// Mask code without changing offsets: links inside examples must remain literal.
function codeMask(markdown: string): Uint8Array {
  const mask = new Uint8Array(markdown.length);
  let offset = 0;
  let fence: { character: string; length: number } | undefined;
  for (const line of markdown.split(/(?<=\n)/)) {
    const marker = /^ {0,3}(`{3,}|~{3,})(.*?)(?:\r?\n)?$/.exec(line);
    if (fence) {
      mask.fill(1, offset, offset + line.length);
      if (marker && marker[1]![0] === fence.character && marker[1]!.length >= fence.length && !marker[2]!.trim()) fence = undefined;
    } else if (marker && (marker[1]![0] !== "`" || !marker[2]!.includes("`"))) {
      fence = { character: marker[1]![0]!, length: marker[1]!.length };
      mask.fill(1, offset, offset + line.length);
    }
    offset += line.length;
  }
  for (let i = 0; i < markdown.length; i++) {
    if (mask[i] || markdown[i] !== "`" || escaped(markdown, i)) continue;
    let end = i;
    while (markdown[end] === "`") end++;
    const delimiter = markdown.slice(i, end);
    let close = markdown.indexOf(delimiter, end);
    while (close !== -1 && (mask[close] || markdown[close - 1] === "`" || markdown[close + delimiter.length] === "`")) {
      close = markdown.indexOf(delimiter, close + delimiter.length);
    }
    if (close !== -1) {
      mask.fill(1, i, close + delimiter.length);
      i = close + delimiter.length - 1;
    } else i = end - 1;
  }
  return mask;
}

type Destination = { start: number; end: number; after: number };
function destinationAt(text: string, start: number): Destination | undefined {
  while (/\s/.test(text[start] ?? "") && start < text.length) start++;
  if (text[start] === "<") {
    const end = text.indexOf(">", start + 1);
    if (end < 0 || /[\r\n]/.test(text.slice(start, end))) return undefined;
    return { start: start + 1, end, after: end + 1 };
  }
  let depth = 0;
  let end = start;
  for (; end < text.length; end++) {
    const character = text[end]!;
    if (escaped(text, end)) continue;
    if (character === "(") depth++;
    else if (character === ")") {
      if (depth === 0) break;
      depth--;
    } else if (/\s/.test(character)) break;
  }
  if (end === start || depth !== 0) return undefined;
  return { start, end, after: end };
}

function labelEnd(text: string, start: number, mask: Uint8Array): number {
  let depth = 1;
  for (let i = start + 1; i < text.length; i++) {
    if (mask[i] || escaped(text, i)) continue;
    if (text[i] === "[") depth++;
    if (text[i] === "]" && --depth === 0) return i;
  }
  return -1;
}

function referenceId(label: string): string {
  return label.replace(/\\([\\[\]])/g, "$1").trim().replace(/\s+/g, " ").toLowerCase();
}

/** Resolve relative Markdown destinations; fragment includes its leading #. */
export function rewriteRelativeLinks(
  markdown: string,
  sourcePath: string,
  resolveLink: (repoPath: string, fragment: string, isImage: boolean) => string,
): string {
  const normalizedSource = posix.normalize(sourcePath);
  if (posix.isAbsolute(sourcePath) || normalizedSource === ".." || normalizedSource.startsWith("../") || sourcePath.includes("\\")) {
    throw new Error(`Source path must be repository-relative: ${sourcePath}`);
  }
  const mask = codeMask(markdown);
  const changes: { start: number; end: number; value: string }[] = [];
  const references = new Map<string, { destination: Destination; image: boolean; link: boolean }>();
  const definitionStarts = new Set<number>();
  let offset = 0;
  for (const line of markdown.split(/(?<=\n)/)) {
    const match = /^ {0,3}\[([^\]\r\n]+)\]:[ \t]*/.exec(line);
    if (match && !mask[offset] && !match[1]!.startsWith("^")) {
      const destination = destinationAt(markdown, offset + match[0].length);
      const id = referenceId(match[1]!);
      if (destination && !references.has(id)) references.set(id, { destination, image: false, link: false });
      definitionStarts.add(offset + line.indexOf("["));
    }
    offset += line.length;
  }
  const rewrite = (destination: Destination, isImage: boolean): void => {
    const raw = markdown.slice(destination.start, destination.end);
    const target = raw.replace(/\\([^\w\s])/g, "$1").replace(/&amp;/g, "&");
    if (!target || target.startsWith("#") || target.startsWith("//") || /^[A-Za-z][A-Za-z0-9+.-]*:/.test(target)) return;
    const hash = target.indexOf("#");
    const fragment = hash < 0 ? "" : target.slice(hash);
    const beforeHash = hash < 0 ? target : target.slice(0, hash);
    const question = beforeHash.indexOf("?");
    const query = question < 0 ? "" : beforeHash.slice(question);
    const encodedPath = question < 0 ? beforeHash : beforeHash.slice(0, question);
    let path: string;
    try { path = decodeURIComponent(encodedPath); } catch { throw new Error(`Invalid percent encoding in Markdown link: ${raw}`); }
    if (path.includes("\\") || /[\u0000-\u001f\u007f]/.test(path)) throw new Error(`Invalid repository link path: ${raw}`);
    const repoPath = path.startsWith("/") ? posix.normalize(path.slice(1)) : posix.normalize(posix.join(posix.dirname(normalizedSource), path));
    if (repoPath === ".." || repoPath.startsWith("../") || posix.isAbsolute(repoPath)) throw new Error(`Markdown link escapes repository: ${raw}`);
    const resolved = resolveLink(repoPath, fragment, isImage);
    const resolvedHash = resolved.indexOf("#");
    const value = query ? (resolvedHash < 0 ? resolved + query : resolved.slice(0, resolvedHash) + query + resolved.slice(resolvedHash)) : resolved;
    changes.push({ start: destination.start, end: destination.end, value });
  };
  for (let i = 0; i < markdown.length; i++) {
    if (mask[i] || markdown[i] !== "[" || escaped(markdown, i) || definitionStarts.has(i)) continue;
    const end = labelEnd(markdown, i, mask);
    if (end < 0) continue;
    const isImage = i > 0 && markdown[i - 1] === "!" && !escaped(markdown, i - 1);
    if (markdown[end + 1] === "(") {
      const destination = destinationAt(markdown, end + 2);
      if (destination) {
        let close = destination.after;
        while (close < markdown.length && /\s/.test(markdown[close]!)) close++;
        if (["\"", "'", "("].includes(markdown[close] ?? "")) {
          const closing = markdown[close] === "(" ? ")" : markdown[close]!;
          let titleEnd = close + 1;
          while (titleEnd < markdown.length && (markdown[titleEnd] !== closing || escaped(markdown, titleEnd))) titleEnd++;
          close = titleEnd + 1;
          while (close < markdown.length && /\s/.test(markdown[close]!)) close++;
        }
        if (markdown[close] === ")") rewrite(destination, isImage);
      }
    } else {
      let label = markdown.slice(i + 1, end);
      if (markdown[end + 1] === "[") {
        const referenceEnd = labelEnd(markdown, end + 1, mask);
        if (referenceEnd >= 0) {
          label = markdown.slice(end + 2, referenceEnd) || label;
          i = referenceEnd;
        }
      }
      const reference = references.get(referenceId(label));
      if (reference) {
        if (isImage) reference.image = true;
        else reference.link = true;
      }
    }
    // Keep scanning labels so embedded image links are also rewritten.
  }
  for (const reference of references.values()) rewrite(reference.destination, reference.image && !reference.link);
  changes.sort((left, right) => left.start - right.start);
  let result = "";
  let cursor = 0;
  for (const change of changes) {
    if (change.start < cursor) continue;
    result += markdown.slice(cursor, change.start) + change.value;
    cursor = change.end;
  }
  return result + markdown.slice(cursor);
}

export async function generateReleaseBody(root: string, tag: string, repository: string): Promise<string> {
  const { version } = parseStableTag(tag);
  requireRepository(repository);
  const packageJson = JSON.parse(await readFile(join(root, "package.json"), "utf8")) as { version?: unknown };
  const cargo = Bun.TOML.parse(await readFile(join(root, "native/Cargo.toml"), "utf8")) as { package?: { name?: string; version?: unknown } };
  const plugin = Bun.TOML.parse(await readFile(join(root, "herdr-plugin.toml"), "utf8")) as { version?: unknown };
  const lock = Bun.TOML.parse(await readFile(join(root, "native/Cargo.lock"), "utf8")) as { package?: { name?: string; version?: unknown; source?: unknown }[] };
  const rootCrates = lock.package?.filter((entry) => entry.name === cargo.package?.name && entry.source === undefined) ?? [];
  if (!cargo.package?.name || rootCrates.length !== 1 ||
    [packageJson.version, cargo.package.version, plugin.version, rootCrates[0]?.version].some((value) => value !== version)) {
    throw new Error(`Tag ${tag} must match package.json, native/Cargo.toml, herdr-plugin.toml, and the root crate in native/Cargo.lock`);
  }
  const sourcePath = `docs/releases/${tag}.md`;
  const markdown = await readFile(join(root, sourcePath), "utf8");
  const heading = `# Herdr Desktop Pet ${tag} release notes`;
  const lines = markdown.split(/\r?\n/);
  const first = lines.findIndex((line) => line.trim() !== "");
  if (first < 0 || lines[first] !== heading) throw new Error(`${sourcePath} must start with the exact heading: ${heading}`);
  const body = lines.slice(first + 1).join("\n").replace(/<!--[\s\S]*?-->/g, "").trim();
  const prose = body.split("\n").filter((line) => line.trim() && !/^\s*#/.test(line));
  if (!prose.length || prose.every((line) => /^(?:[-*\s]*)(?:TODO|TBD|PLACEHOLDER|COMING SOON|RELEASE NOTES HERE)[.!\s]*$/i.test(line))) {
    throw new Error(`${sourcePath} must contain nonempty, reviewed release notes, not a placeholder`);
  }
  const paths = new Set<string>();
  const result = rewriteRelativeLinks(markdown, sourcePath, (repoPath, fragment, isImage) => {
    paths.add(repoPath);
    const base = isImage ? `https://raw.githubusercontent.com/${repository}/${tag}` : `https://github.com/${repository}/blob/${tag}`;
    return `${base}/${repoPath.split("/").map(encodeURIComponent).join("/")}${fragment}`;
  });
  for (const path of paths) {
    try { await stat(join(root, path)); } catch { throw new Error(`${sourcePath} links to a missing repository path: ${path}`); }
  }
  return result;
}

async function main(args: string[]): Promise<void> {
  if (args[0] !== "check" || !args[1]) throw new Error("Usage: bun scripts/release-docs.ts check TAG [--root ROOT] [--repository OWNER/REPO] [--output FILE]");
  const tag = args[1];
  const options = new Map<string, string>();
  for (let i = 2; i < args.length; i += 2) {
    const name = args[i]!;
    const value = args[i + 1];
    if (!["--root", "--repository", "--output"].includes(name) || !value || value.startsWith("--") || options.has(name)) throw new Error(`Invalid or repeated option: ${name}`);
    options.set(name, value);
  }
  const root = resolve(options.get("--root") ?? join(import.meta.dir, ".."));
  const repository = options.get("--repository") ?? process.env.GITHUB_REPOSITORY ?? "hanbong5938/herdr-desktop-pet";
  const body = await generateReleaseBody(root, tag, repository);
  const output = resolve(options.get("--output") ?? join(root, "dist", `${tag}-release-notes.md`));
  await mkdir(dirname(output), { recursive: true });
  const temporary = `${output}.${process.pid}.tmp`;
  try {
    await writeFile(temporary, body, { flag: "wx" });
    await rename(temporary, output);
  } finally {
    await rm(temporary, { force: true });
  }
  console.log(`Validated ${tag}; wrote frozen release notes to ${output}`);
}

if (import.meta.main) {
  main(process.argv.slice(2)).catch((error: unknown) => {
    console.error(`release-docs: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  });
}
