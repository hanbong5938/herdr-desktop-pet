import { existsSync, lstatSync, realpathSync, statSync } from "node:fs";
import { mkdir, mkdtemp, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { spawnSync } from "node:child_process";
import {
  fetchReleases, isPublishedRelease, parseStableTag, rewriteRelativeLinks,
  type ReleaseRecord,
} from "./release-docs";

export const MANAGED_STATIC_PAGES = [
  "Home", "Home-ko", "Versioning-and-Compatibility", "Unreleased", "Publishing",
  "Migration-v0.1.4-to-v0.1.6", "Upgrade-Unreleased", "Upgrade-Unreleased-ko",
  "Migration-v0.1.11-to-v0.2.0",
  "Release-Status", "Older-Releases", "_Sidebar",
] as const;
const MANIFEST = ".wiki-managed-pages";
const historicalTags: Record<string, true> = {
  "v0.1.4": true, "v0.1.5": true, "v0.1.6": true, "v0.1.7": true,
  "v0.1.8": true, "v0.1.9": true, "v0.1.10": true, "v0.1.11": true,
};
const stablePattern = "(?:0|[1-9][0-9]*)";
const releaseFilename = new RegExp(`^Release-v${stablePattern}\\.${stablePattern}\\.${stablePattern}\\.md$`);
const lineFilename = new RegExp(`^Release-Line-${stablePattern}\\.${stablePattern}\\.md$`);

export function managedPageName(filename: string): boolean {
  return MANAGED_STATIC_PAGES.some((name) => `${name}.md` === filename)
    || releaseFilename.test(filename) || lineFilename.test(filename);
}

export type PublicationState = "published" | "draft" | "prerelease" | "partial" | "unpublished" | "tagged-only" | "no-release";
export function publicationState(tag: string, record?: ReleaseRecord): PublicationState {
  if (!record) return tag === "v0.1.5" ? "tagged-only" : "no-release";
  if (record.draft) return "draft";
  if (record.prerelease) return "prerelease";
  if (isPublishedRelease(record)) return "published";
  return validPublicationTime(record.published_at) ? "partial" : "unpublished";
}

function validPublicationTime(value: string | null): boolean {
  return typeof value === "string" && value.length > 0 && Number.isFinite(Date.parse(value));
}
function compareTags(left: string, right: string): number {
  const a = parseStableTag(left);
  const b = parseStableTag(right);
  return b.major - a.major || b.minor - a.minor || b.patch - a.patch;
}
function git(root: string, args: string[], optional = false): string | undefined {
  const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  if (result.error || result.status !== 0) {
    if (optional) return undefined;
    // Git stderr can contain remote configuration: report only the operation, never credentials.
    throw new Error(`Cannot read Git source (${args[0]}). Ensure the repository and required refs are available.`);
  }
  return result.stdout;
}
function requireRef(value: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9_./-]*$/.test(value)
    || value.includes("..") || value.includes("//") || value.endsWith("/")
    || value.split("/").some((part) => part.startsWith(".") || part.endsWith(".lock"))) {
    throw new Error("source-ref must be a plain Git ref or commit ID");
  }
}
function requireRepository(repository: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*\/[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(repository)
    || repository.split("/").some((part) => part === "." || part === "..")) {
    throw new Error("repository must be OWNER/REPO");
  }
}
function inside(path: string, directory: string): boolean {
  const child = relative(directory, path);
  return child === "" || (!child.startsWith(`..${sep}`) && child !== ".." && !isAbsolute(child));
}
function canonicalOutput(path: string): string {
  const entry = lstatSync(path, { throwIfNoEntry: false });
  if (entry) {
    if (entry.isSymbolicLink() || !entry.isDirectory()) throw new Error("output must be a real directory");
    return realpathSync(path);
  }
  const parent = dirname(path);
  if (parent === path) throw new Error("Cannot resolve output directory");
  return join(canonicalOutput(parent), relative(parent, path));
}
function sourceSlug(path: string): string | undefined {
  const fixed: Record<string, string> = {
    "docs/releases/README.md": "Home",
    "docs/releases/README.ko.md": "Home-ko",
    "docs/releases/policy.md": "Versioning-and-Compatibility",
    "docs/releases/unreleased.md": "Unreleased",
    "docs/releases/publishing.md": "Publishing",
    "docs/migrations/v0.1.4-to-v0.1.6.md": "Migration-v0.1.4-to-v0.1.6",
    "docs/migrations/v0.1.11-to-v0.2.0.md": "Migration-v0.1.11-to-v0.2.0",
    "docs/migrations/unreleased.md": "Upgrade-Unreleased",
    "docs/migrations/unreleased.ko.md": "Upgrade-Unreleased-ko",
  };
  if (fixed[path]) return fixed[path];
  const note = /^docs\/releases\/(v\d+\.\d+\.\d+)\.md$/.exec(path);
  if (note) {
    parseStableTag(note[1]!);
    return `Release-${note[1]}`;
  }
  const line = /^docs\/releases\/((?:0|[1-9]\d*)\.(?:0|[1-9]\d*))\.md$/.exec(path);
  if (line) {
    parseStableTag(`v${line[1]}.0`);
    return `Release-Line-${line[1]}`;
  }
  return undefined;
}
function sourceURL(repository: string, ref: string, path: string, directory = false): string {
  return `https://github.com/${repository}/${directory ? "tree" : "blob"}/${encodeURIComponent(ref)}/${path.split("/").map(encodeURIComponent).join("/")}`;
}
function wikiURL(repository: string, slug: string): string {
  return `https://github.com/${repository}/wiki/${slug}`;
}
function validateCatalog(value: unknown): ReleaseRecord[] {
  if (!Array.isArray(value)) throw new Error("Release catalog must be an array");
  const tags = new Set<string>();
  const records: ReleaseRecord[] = [];
  for (const candidate of value) {
    const record: unknown = candidate;
    if (!record || typeof record !== "object"
      || !("tag_name" in record) || typeof record.tag_name !== "string"
      || !("html_url" in record) || typeof record.html_url !== "string" || !/^https:\/\/github\.com\//.test(record.html_url)
      || !("draft" in record) || typeof record.draft !== "boolean"
      || !("prerelease" in record) || typeof record.prerelease !== "boolean"
      || !("published_at" in record) || !(record.published_at === null || typeof record.published_at === "string")
      || !("assets" in record) || !Array.isArray(record.assets)) {
      throw new Error("Malformed release catalog record");
    }
    const assets: ReleaseRecord["assets"] = [];
    for (const candidateAsset of record.assets) {
      const asset: unknown = candidateAsset;
      if (!asset || typeof asset !== "object"
        || !("name" in asset) || typeof asset.name !== "string"
        || !("size" in asset) || typeof asset.size !== "number" || !Number.isFinite(asset.size) || asset.size < 0) {
        throw new Error("Malformed release catalog asset");
      }
      const state = "state" in asset ? asset.state : undefined;
      if (state !== undefined && typeof state !== "string") throw new Error("Malformed release catalog asset state");
      assets.push({ name: asset.name, size: asset.size, ...(state === undefined ? {} : { state }) });
    }
    if (tags.has(record.tag_name)) throw new Error(`Duplicate release catalog tag: ${record.tag_name}`);
    tags.add(record.tag_name);
    records.push({
      tag_name: record.tag_name, html_url: record.html_url, draft: record.draft,
      prerelease: record.prerelease, published_at: record.published_at, assets,
    });
  }
  return records;
}
async function scanSources(root: string): Promise<Map<string, string>> {
  const sources = new Map<string, string>();
  for (const directory of ["docs/releases", "docs/migrations"]) {
    const entries = await readdir(join(root, directory), { withFileTypes: true });
    for (const entry of entries) {
      if (!entry.name.endsWith(".md")) continue;
      const path = `${directory}/${entry.name}`;
      const slug = sourceSlug(path);
      if (!entry.isFile() || !slug) throw new Error(`Unsupported canonical Markdown filename: ${path}`);
      if (sources.has(slug)) throw new Error(`Duplicate Wiki slug: ${slug}`);
      const fullPath = join(root, path);
      if (!inside(realpathSync(fullPath), root)) throw new Error(`Source escapes repository: ${path}`);
      const text = await readFile(fullPath, "utf8");
      if (!text.trim()) throw new Error(`Empty canonical page: ${path}`);
      sources.set(slug, path);
    }
  }
  for (const slug of MANAGED_STATIC_PAGES.slice(0, 8)) {
    if (!sources.has(slug)) throw new Error(`Missing canonical page: ${slug}`);
  }
  return sources;
}
function noteHeading(text: string, tag: string): void {
  const firstHeading = text.split(/\r?\n/).find((line) => /^#{1,6}\s/.test(line));
  if (firstHeading !== `# Herdr Desktop Pet ${tag} release notes` || !text.trim()) {
    throw new Error(`Invalid release note heading for ${tag}`);
  }
}
function stateLabel(state: PublicationState): string {
  return {
    published: "Published — complete binary release",
    draft: "Draft — not public or shipped",
    prerelease: "Prerelease — not a stable binary release",
    partial: "Partial release — required binary/checksum assets incomplete",
    unpublished: "Unpublished — no actual publication timestamp",
    "tagged-only": "Tagged only — packaging failed; no published binary release",
    "no-release": "Pending documentation — no GitHub Release record; not shipped",
  }[state];
}
function publicRecord(record?: ReleaseRecord): record is ReleaseRecord {
  return !!record && !record.draft && validPublicationTime(record.published_at);
}
function attribution(repository: string, ref: string, path: string, backfill = false): string {
  return `\n\n---\nGenerated from [reviewed repository source](${sourceURL(repository, ref, path)}) at \`${ref}\`. `
    + (backfill ? "Historical backfill: this release tag predates the canonical documentation tree; reviewed current notes reconstruct contemporaneous evidence. " : "")
    + "Edit the canonical repository Markdown, not this generated Wiki page.\n";
}

export interface WikiOptions {
  root: string;
  output: string;
  repository: string;
  sourceRef: string;
  releases?: ReleaseRecord[];
  releasesJson?: string;
  requireRelease?: string;
}

/** Render and validate the entire projection before mutating any managed output file. */
export async function generateWiki(options: WikiOptions): Promise<string[]> {
  requireRepository(options.repository);
  requireRef(options.sourceRef);
  const root = realpathSync(resolve(options.root));
  if (!statSync(root).isDirectory()) throw new Error("root must be a repository directory");
  const output = canonicalOutput(resolve(options.output));
  if (inside(output, root) || inside(root, output)) throw new Error("output must be outside and not contain the source repository");
  const head = git(root, ["rev-parse", "--verify", "HEAD^{commit}"])!.trim();
  git(root, ["rev-parse", "--verify", `${options.sourceRef}^{commit}`]);
  if (options.releases && options.releasesJson) throw new Error("Choose only one release catalog input");
  const catalog = validateCatalog(options.releases ?? (options.releasesJson
    ? JSON.parse(await readFile(options.releasesJson, "utf8"))
    : await fetchReleases(options.repository, process.env.GITHUB_TOKEN)));
  if (options.requireRelease) requirePublishedRelease(options.requireRelease, catalog);
  const sources = await scanSources(root);
  const records = new Map<string, ReleaseRecord>();
  for (const record of catalog) {
    try { parseStableTag(record.tag_name); } catch { continue; }
    records.set(record.tag_name, record);
  }
  const tags = new Set<string>(records.keys());
  for (const path of sources.values()) {
    const match = /^docs\/releases\/(v\d+\.\d+\.\d+)\.md$/.exec(path);
    if (match) tags.add(match[1]!);
  }
  const orderedTags = [...tags].sort(compareTags);
  for (const tag of orderedTags) {
    if (publicationState(tag, records.get(tag)) === "published" && !sources.has(`Release-${tag}`)) {
      sources.set(`Release-${tag}`, `docs/releases/${tag}.md`);
    }
  }
  const pages = new Map<string, string>();
  for (const [slug, path] of sources) {
    const tag = /^Release-(v\d+\.\d+\.\d+)$/.exec(slug)?.[1];
    let ref = head;
    let frozen = false;
    let backfill = false;
    let text: string;
    if (tag && publicationState(tag, records.get(tag)) === "published") {
      const contents = git(root, ["show", `${tag}:${path}`], true);
      if (contents === undefined) {
        if (!Object.hasOwn(historicalTags, tag)) throw new Error(`Published ${tag} is missing its frozen note at ${tag}:${path}`);
        if (!existsSync(join(root, path))) throw new Error(`Missing reviewed historical backfill for ${tag}`);
        text = await readFile(join(root, path), "utf8");
        backfill = true;
      } else {
        text = contents;
        ref = tag;
        frozen = true;
      }
    } else {
      text = await readFile(join(root, path), "utf8");
      backfill = !!tag && Object.hasOwn(historicalTags, tag);
    }
    if (tag) noteHeading(text, tag);
    const rewritten = rewriteRelativeLinks(text, path, (repoPath, fragment, isImage) => {
      let directory = false;
      if (frozen) {
        const kind = git(root, ["cat-file", "-t", `${tag}:${repoPath}`], true)?.trim();
        if (kind !== "blob" && kind !== "tree") throw new Error(`Broken frozen link in ${path}: ${repoPath}`);
        directory = kind === "tree";
      } else {
        const fullPath = join(root, repoPath);
        if (!existsSync(fullPath) || !inside(realpathSync(fullPath), root)) throw new Error(`Broken local link in ${path}: ${repoPath}`);
        directory = statSync(fullPath).isDirectory();
      }
      const mapped = sourceSlug(repoPath);
      if (!isImage && mapped && sources.has(mapped)) return `${wikiURL(options.repository, mapped)}${fragment}`;
      if (isImage) return `https://raw.githubusercontent.com/${options.repository}/${encodeURIComponent(ref)}/${repoPath.split("/").map(encodeURIComponent).join("/")}${fragment}`;
      return `${sourceURL(options.repository, ref, repoPath, directory)}${fragment}`;
    });
    let status = "";
    if (tag) {
      const record = records.get(tag);
      status = `> **Publication status:** ${stateLabel(publicationState(tag, record))}.`;
      if (publicRecord(record)) status += ` [GitHub Release](${record.html_url}); published at \`${record.published_at}\`.`;
      status += "\n\n";
    }
    pages.set(`${slug}.md`, status + rewritten.trimEnd() + attribution(options.repository, ref, path, backfill));
  }
  const row = (tag: string): string => {
    const record = records.get(tag);
    const state = publicationState(tag, record);
    const note = sources.has(`Release-${tag}`) ? `[${tag}](${wikiURL(options.repository, `Release-${tag}`)})` : `\`${tag}\``;
    const release = publicRecord(record) ? `[Release](${record.html_url})` : "—";
    const date = publicRecord(record) ? `\`${record.published_at}\`` : "—";
    return `| ${note} | ${stateLabel(state)} | ${date} | ${release} |`;
  };
  const tableHeader = "| Version | State | Actual publication timestamp | GitHub record |\n| --- | --- | --- | --- |\n";
  const indexFooter = `\n\n---\nGenerated from the current GitHub release catalog and current reviewed documentation at \`${head}\`. Requested source ref: \`${options.sourceRef}\`. Stable versions are ordered by numeric semantic version, not publication date. A published label requires a public non-draft, non-prerelease record and all three uploaded, nonzero binary/checksum assets.\n`;
  pages.set("Release-Status.md", "# Release status\n\nThis live index is not a snapshot of an older release tag. Pending documentation is not a shipped feature; partial, draft and prerelease records are not stable binary downloads.\n\n"
    + tableHeader + orderedTags.map(row).join("\n") + indexFooter);
  const published = orderedTags.filter((tag) => publicationState(tag, records.get(tag)) === "published");
  const latest = published[0];
  pages.set("Older-Releases.md", "# Older releases\n\n"
    + (latest ? `Latest complete stable binary release: [${latest}](${records.get(latest)!.html_url}).\n\n` : "No complete stable binary release is currently recorded.\n\n")
    + "Older releases and non-shipped versions remain visible as historical evidence, not support or EOL promises.\n\n"
    + tableHeader + orderedTags.filter((tag) => tag !== latest).map(row).join("\n") + indexFooter);
  const lineSlugs = [...sources.keys()].filter((slug) => slug.startsWith("Release-Line-")).sort((a, b) => compareTags(`v${a.slice(13)}.0`, `v${b.slice(13)}.0`));
  pages.set("_Sidebar.md", [
    "- [Home](Home)", "- [한국어](Home-ko)", "- [Release status](Release-Status)",
    "- [Versioning and compatibility](Versioning-and-Compatibility)", "- [Unreleased](Unreleased)",
    "- [Upgrade unreleased](Upgrade-Unreleased)", "- [미출시 업그레이드](Upgrade-Unreleased-ko)",
    "- [v0.1.4 → v0.1.6 migration](Migration-v0.1.4-to-v0.1.6)",
    ...(sources.has("Migration-v0.1.11-to-v0.2.0") ? ["- [v0.1.11 / beta3 → v0.2.0 migration](Migration-v0.1.11-to-v0.2.0)"] : []),
    ...lineSlugs.map((slug) => `- [Release line ${slug.slice(13)}](${slug})`),
    ...orderedTags.filter((tag) => sources.has(`Release-${tag}`)).map((tag) => `- [${tag} — ${publicationState(tag, records.get(tag))}](Release-${tag})`),
    "- [Older releases](Older-Releases)", "- [Publishing](Publishing)",
    "", "<!-- Generated navigation; edit canonical repository sources. -->", "",
  ].join("\n"));
  const names = [...pages.keys()].sort();
  if (names.some((name) => !managedPageName(name))) throw new Error("Invalid generated page filename");
  let previous: string[] = [];
  const manifestPath = join(output, MANIFEST);
  const manifestEntry = lstatSync(manifestPath, { throwIfNoEntry: false });
  if (manifestEntry) {
    if (manifestEntry.isSymbolicLink() || !manifestEntry.isFile()) throw new Error("Invalid managed-page manifest");
    previous = (await readFile(manifestPath, "utf8")).split(/\r?\n/).filter(Boolean);
    if (previous.some((name) => !managedPageName(name)) || new Set(previous).size !== previous.length) throw new Error("Invalid managed-page manifest entries");
  }
  for (const name of new Set([...names, ...previous])) {
    const destination = join(output, name);
    const entry = lstatSync(destination, { throwIfNoEntry: false });
    if (entry && (entry.isSymbolicLink() || !entry.isFile())) {
      throw new Error(`Managed output is not a regular file: ${name}`);
    }
  }
  // All Git, catalog, Markdown, link and destination validation is complete before writes.
  // Stage every changed byte first: rendering or staging failure leaves managed output intact.
  const changed: string[] = [];
  for (const [name, contents] of pages) {
    if (!existsSync(join(output, name)) || await readFile(join(output, name), "utf8") !== contents) changed.push(name);
  }
  const manifest = names.join("\n") + "\n";
  const manifestChanged = !existsSync(manifestPath) || await readFile(manifestPath, "utf8") !== manifest;
  await mkdir(dirname(output), { recursive: true });
  const staging = await mkdtemp(join(dirname(output), ".herdr-wiki-"));
  try {
    for (const name of changed) await writeFile(join(staging, name), pages.get(name)!, "utf8");
    if (manifestChanged) await writeFile(join(staging, MANIFEST), manifest, "utf8");
    await mkdir(output, { recursive: true });
    for (const name of changed) await rename(join(staging, name), join(output, name));
    for (const name of previous) {
      if (!pages.has(name) && existsSync(join(output, name))) await rm(join(output, name));
    }
    if (manifestChanged) await rename(join(staging, MANIFEST), manifestPath);
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
  return names;
}

export function requirePublishedRelease(tag: string, records: ReleaseRecord[]): void {
  parseStableTag(tag);
  const record = records.find((entry) => entry.tag_name === tag);
  if (!record || !isPublishedRelease(record)) throw new Error(`Required release ${tag} is not a complete public stable binary release`);
}

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const values = new Map<string, string>();
  const allowed: Record<string, true> = {
    "--root": true, "--output": true, "--repository": true, "--source-ref": true,
    "--releases-json": true, "--require-release": true,
  };
  for (let index = 0; index < args.length; index += 2) {
    const flag = args[index]!;
    const value = args[index + 1];
    if (!Object.hasOwn(allowed, flag) || values.has(flag) || !value || value.startsWith("--")) throw new Error(`Invalid argument: ${flag}`);
    values.set(flag, value);
  }
  const output = values.get("--output");
  const sourceRef = values.get("--source-ref");
  if (!output || !sourceRef) throw new Error("Usage: bun scripts/wiki-docs.ts --output DIR --source-ref REF [--root ROOT] [--repository OWNER/REPO] [--releases-json FILE] [--require-release TAG]");
  const repository = values.get("--repository") ?? "hanbong5938/herdr-desktop-pet";
  const pages = await generateWiki({
    root: values.get("--root") ?? resolve(import.meta.dir, ".."), output, repository, sourceRef,
    releasesJson: values.get("--releases-json"), requireRelease: values.get("--require-release"),
  });
  console.log(`wiki-docs: generated ${pages.length} managed pages`);
}
if (import.meta.main) {
  main().catch((error: unknown) => {
    console.error(`wiki-docs: ${error instanceof Error ? error.message : "generation failed"}`);
    process.exitCode = 1;
  });
}
